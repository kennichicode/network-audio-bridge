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
    json(res, 200, {
      ok: true,
      room: defaultRoom,
      ttlSeconds: tokenTtlSeconds,
      security: "passcode + subscribe-only listener token",
    });
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
  json(res, 200, {
    url: env.LIVEKIT_URL,
    room,
    identity,
    expiresInSeconds: tokenTtlSeconds,
    permissions: { canPublish: false, canSubscribe: true },
    token,
  });
});

server.listen(8094, "127.0.0.1", () => {
  console.log("livekit token service listening on 127.0.0.1:8094");
});
