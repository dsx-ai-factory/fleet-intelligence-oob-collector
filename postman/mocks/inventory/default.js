// Local-only mock of the proposed envelope and existing enrollment/result shapes.
// Enrollment and per-node result shapes follow the existing backend models.
const http = require("http");

http.createServer(async (req, res) => {
  const reply = (status, body) => {
    res.writeHead(status, { "Content-Type": "application/json" });
    res.end(JSON.stringify(body));
  };
  // @endpoint POST /v1/agent/enroll
  if (req.method === "POST" && req.url === "/v1/agent/enroll") {
    return reply(200, { jwtAssertion: "local-fixture-jwt" });
  }
  // @endpoint PUT /v1/agent/oob_nodes
  if (req.method === "PUT" && req.url === "/v1/agent/oob_nodes") {
    const body = req.body;
    const config = body?.collector?.config;
    if (body?.agent || !body?.collector?.version || !config?.runtime ||
        !Array.isArray(body.data) || body.data.length === 0) {
      return reply(400, { error: "Expected collector version/config and inventory data" });
    }
    await pm.state.set("inventory.lastReport", config);
    // The smoke test uses an unreachable loopback BMC. Its observations have no
    // UUID, matching the current backend's per-item MISSING_UUID behavior.
    return reply(200, {
      results: body.data.map(() => ({ status: "MISSING_UUID", message: "payload uuid missing" }))
    });
  }
  // Local test inspection only; this is not a proposed product endpoint.
  // @endpoint GET /__checks
  if (req.method === "GET" && req.url === "/__checks") {
    return reply(200, { config: await pm.state.get("inventory.lastReport") });
  }
  return reply(501, { error: "Endpoint outside this simulation" });
}).listen(process.env.PORT || 4905);
