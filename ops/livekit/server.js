const http = require("http");
const crypto = require("crypto");
const fs = require("fs");

const envPath = process.env.LIVEKIT_ENV || "/opt/livekit/livekit.env";
const envText = fs.readFileSync(envPath, "utf8");
const env = Object.fromEntries(
  envText
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#") && line.includes("="))
    .map((line) => {
      const idx = line.indexOf("=");
      return [
        line.slice(0, idx),
        line.slice(idx + 1).replace(/^['"]|['"]$/g, ""),
      ];
    }),
);

const apiKey = env.LIVEKIT_API_KEY;
const apiSecret = env.LIVEKIT_API_SECRET;
const defaultRoom = env.LIVEKIT_ROOM || "reaper-master";
const listenPasscode = process.env.LIVEKIT_LISTEN_PASSCODE || env.LIVEKIT_LISTEN_PASSCODE || "";
const tokenTtlSeconds = Math.max(
  60,
  Math.min(3600, Number(process.env.LIVEKIT_TOKEN_TTL_SECONDS || env.LIVEKIT_TOKEN_TTL_SECONDS || 1800)),
);
const allowedOrigins = String(
  process.env.LIVEKIT_ALLOWED_ORIGINS ||
    env.LIVEKIT_ALLOWED_ORIGINS ||
    "https://livekit.kenichi-kawabata.com",
)
  .split(",")
  .map((origin) => origin.trim())
  .filter(Boolean);
const rateWindowMs = 60 * 1000;
const maxTokenRequestsPerWindow = 20;
const rateBuckets = new Map();
const proofSessions = new Map();
const listenerProofs = new Map();

if (!listenPasscode) {
  console.error("LIVEKIT_LISTEN_PASSCODE is required");
  process.exit(1);
}
if (!apiKey || !apiSecret) {
  console.error("LIVEKIT_API_KEY and LIVEKIT_API_SECRET are required");
  process.exit(1);
}

function b64url(input) {
  return Buffer.from(input)
    .toString("base64")
    .replace(/=/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}

function signToken({ room, identity }) {
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "HS256", typ: "JWT" };
  const payload = {
    exp: now + tokenTtlSeconds,
    iss: apiKey,
    nbf: now - 5,
    sub: identity,
    name: identity,
    video: {
      roomJoin: true,
      room,
      canPublish: false,
      canSubscribe: true,
      canPublishData: false,
    },
  };
  const unsigned = `${b64url(JSON.stringify(header))}.${b64url(JSON.stringify(payload))}`;
  const sig = crypto.createHmac("sha256", apiSecret).update(unsigned).digest("base64url");
  return `${unsigned}.${sig}`;
}

function json(res, status, body) {
  res.writeHead(status, { "Content-Type": "application/json", "X-Content-Type-Options": "nosniff" });
  res.end(JSON.stringify(body));
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.on("data", (chunk) => {
      body += chunk;
      if (body.length > 4096) {
        reject(new Error("body_too_large"));
        req.destroy();
      }
    });
    req.on("end", () => {
      if (!body) {
        resolve({});
        return;
      }
      try {
        resolve(JSON.parse(body));
      } catch (err) {
        reject(err);
      }
    });
    req.on("error", reject);
  });
}

function safeEqual(a, b) {
  const left = Buffer.from(String(a || ""));
  const right = Buffer.from(String(b || ""));
  if (left.length !== right.length) {
    return false;
  }
  return crypto.timingSafeEqual(left, right);
}

function requestIp(req) {
  const forwarded = String(req.headers["x-forwarded-for"] || "");
  if (forwarded) {
    return forwarded.split(",")[0].trim();
  }
  return req.socket.remoteAddress || "unknown";
}

function allowOrigin(req, res) {
  const origin = req.headers.origin;
  if (!origin) {
    return true;
  }
  if (!allowedOrigins.includes(origin)) {
    return false;
  }
  res.setHeader("Access-Control-Allow-Origin", origin);
  res.setHeader("Vary", "Origin");
  return true;
}

function rateLimitToken(req) {
  const key = requestIp(req);
  const now = Date.now();
  const bucket = rateBuckets.get(key);
  if (!bucket || now - bucket.startedAt > rateWindowMs) {
    rateBuckets.set(key, { startedAt: now, count: 1 });
    return true;
  }
  bucket.count += 1;
  return bucket.count <= maxTokenRequestsPerWindow;
}

function cleanupProofs() {
  const now = Date.now();
  for (const [identity, session] of proofSessions) {
    if (session.expiresAt <= now) {
      proofSessions.delete(identity);
      listenerProofs.delete(identity);
    }
  }
}

function cleanText(value, maxLength = 80) {
  return String(value || "")
    .replace(/[^\p{L}\p{N}\s/:+._=-]/gu, "")
    .trim()
    .slice(0, maxLength);
}

function cleanNumber(value) {
  const num = Number(value);
  return Number.isFinite(num) ? num : 0;
}

function deviceLabel(userAgent) {
  const ua = String(userAgent || "");
  if (/iPhone/i.test(ua)) return "iPhone";
  if (/iPad/i.test(ua)) return "iPad";
  if (/Android/i.test(ua)) return "Android";
  if (/Macintosh/i.test(ua)) return "Mac browser";
  if (/Windows/i.test(ua)) return "Windows browser";
  return "browser";
}

function isLocalRequest(req) {
  const forwarded = String(req.headers["x-forwarded-for"] || "");
  const remote = req.socket.remoteAddress || "";
  return !forwarded && (remote === "127.0.0.1" || remote === "::1" || remote === "::ffff:127.0.0.1");
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  res.setHeader("Cache-Control", "no-store");
  res.setHeader("Referrer-Policy", "no-referrer");

  if (!allowOrigin(req, res)) {
    json(res, 403, { error: "origin_not_allowed" });
    return;
  }

  if (req.method === "OPTIONS") {
    res.writeHead(204, {
      "Access-Control-Allow-Methods": "POST, OPTIONS",
      "Access-Control-Allow-Headers": "Content-Type",
    });
    res.end();
    return;
  }

  if (url.pathname === "/healthz") {
    cleanupProofs();
    const proofs = Array.from(listenerProofs.values());
    const lastProof = proofs.sort((a, b) => b.atUnixMs - a.atUnixMs)[0];
    json(res, 200, {
      ok: true,
      room: defaultRoom,
      ttlSeconds: tokenTtlSeconds,
      security: "passcode + subscribe-only listener token",
      proofCount: proofs.length,
      lastProof: lastProof
        ? {
            ageSeconds: Math.round((Date.now() - lastProof.atUnixMs) / 1000),
            device: lastProof.device,
            proof: lastProof.proof,
            player: lastProof.player,
            track: lastProof.track,
            packets: lastProof.packets,
            bytes: lastProof.bytes,
            level: lastProof.level,
            loss: lastProof.loss,
          }
        : null,
    });
    return;
  }

  if (url.pathname === "/proofs") {
    cleanupProofs();
    if (!isLocalRequest(req)) {
      json(res, 403, { error: "local_only" });
      return;
    }
    json(res, 200, {
      ok: true,
      proofs: Array.from(listenerProofs.values()).map((proof) => ({
        ...proof,
        ageSeconds: Math.round((Date.now() - proof.atUnixMs) / 1000),
      })),
    });
    return;
  }

  if (url.pathname === "/proof") {
    cleanupProofs();
    if (req.method !== "POST") {
      json(res, 405, { error: "method_not_allowed" });
      return;
    }

    let body;
    try {
      body = await readJson(req);
    } catch (err) {
      json(res, 400, { error: "bad_json" });
      return;
    }

    const identity = String(body.identity || "").replace(/[^a-zA-Z0-9_.-]/g, "").slice(0, 36);
    const proofKey = String(body.proofKey || "");
    const session = proofSessions.get(identity);
    if (!session || session.expiresAt <= Date.now() || !safeEqual(proofKey, session.proofKey)) {
      json(res, 403, { error: "proof_not_allowed" });
      return;
    }

    const proof = {
      atUnixMs: Date.now(),
      identity,
      room: session.room,
      device: deviceLabel(req.headers["user-agent"]),
      proof: cleanText(body.proof),
      status: cleanText(body.status),
      player: cleanText(body.player),
      publisher: cleanText(body.publisher),
      track: cleanText(body.track),
      packets: cleanNumber(body.packets),
      bytes: cleanNumber(body.bytes),
      packetDelta: cleanText(body.packetDelta, 32),
      byteDelta: cleanText(body.byteDelta, 32),
      level: cleanText(body.level, 32),
      jitter: cleanText(body.jitter, 32),
      loss: cleanNumber(body.loss),
    };
    listenerProofs.set(identity, proof);
    console.log(
      `listener proof identity=${identity} device=${proof.device} proof=${proof.proof} player=${proof.player} packets=${proof.packets} bytes=${proof.bytes}`,
    );
    json(res, 200, { ok: true });
    return;
  }

  if (url.pathname !== "/token") {
    json(res, 404, { error: "not_found" });
    return;
  }

  if (req.method !== "POST") {
    json(res, 405, { error: "method_not_allowed" });
    return;
  }

  if (!rateLimitToken(req)) {
    json(res, 429, { error: "rate_limited" });
    return;
  }

  let body;
  try {
    body = await readJson(req);
  } catch (err) {
    json(res, 400, { error: "bad_json" });
    return;
  }

  if (!safeEqual(body.passcode, listenPasscode)) {
    json(res, 401, { error: "bad_passcode" });
    return;
  }

  const room = body.room || defaultRoom;
  if (room !== defaultRoom) {
    json(res, 403, { error: "room_not_allowed" });
    return;
  }

  const suffix = crypto.randomBytes(4).toString("hex");
  const requestedIdentity = String(body.identity || "").replace(/[^a-zA-Z0-9_.-]/g, "").slice(0, 28);
  const identity = requestedIdentity.startsWith("listener-")
    ? requestedIdentity
    : `listener-${requestedIdentity || suffix}`.slice(0, 36);
  const token = signToken({ room, identity });
  const proofKey = crypto.randomBytes(16).toString("hex");
  proofSessions.set(identity, {
    proofKey,
    room,
    expiresAt: Date.now() + tokenTtlSeconds * 1000,
  });
  json(res, 200, {
    url: env.LIVEKIT_URL,
    room,
    identity,
    expiresInSeconds: tokenTtlSeconds,
    permissions: { canPublish: false, canSubscribe: true },
    proofKey,
    token,
  });
});

server.listen(8094, "127.0.0.1", () => {
  console.log("livekit token service listening on 127.0.0.1:8094");
});
