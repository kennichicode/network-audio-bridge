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
    exp: now + 6 * 60 * 60,
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

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Cache-Control", "no-store");

  if (url.pathname === "/healthz") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ ok: true }));
    return;
  }

  if (url.pathname !== "/token") {
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: "not_found" }));
    return;
  }

  const room = url.searchParams.get("room") || defaultRoom;
  const suffix = crypto.randomBytes(4).toString("hex");
  const identity = url.searchParams.get("identity") || `listener-${suffix}`;
  const token = signToken({ room, identity });
  res.writeHead(200, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ url: env.LIVEKIT_URL, room, identity, token }));
});

server.listen(8094, "127.0.0.1", () => {
  console.log("livekit token service listening on 127.0.0.1:8094");
});
