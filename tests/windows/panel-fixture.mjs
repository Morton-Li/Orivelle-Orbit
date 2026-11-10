import assert from "node:assert/strict";
import { createHash, createPublicKey, randomUUID, verify } from "node:crypto";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:https";

export const panelOrigin = "https://orivelle.powerbyte.io";
const api = "/api/v1/orbit";
const hash = (bytes, encoding = "hex") =>
  createHash("sha256").update(bytes).digest(encoding);

async function readBody(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    assert(size <= 1024 * 1024, "Fixture request exceeded its size limit.");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function verifyProof(request, body, installation, binding, nonce, token) {
  const headers = request.headers;
  assert.equal(
    headers["x-orbit-installation-id"],
    installation.installation_id,
  );
  assert.equal(headers["x-orbit-proof-binding"], binding);
  assert.equal(headers["x-orbit-proof-nonce"], nonce);
  assert.equal(headers.authorization, token ? "Bearer " + token : undefined);
  const canonical = [
    "orbit-installation-proof-v1",
    request.method,
    request.url,
    panelOrigin,
    installation.installation_id,
    binding,
    headers["x-orbit-proof-timestamp"],
    nonce,
    headers["x-orbit-proof-request-id"],
    hash(body),
    token ? hash(token) : "",
    "",
  ].join("\n");
  const key = createPublicKey({
    format: "jwk",
    key: { kty: "OKP", crv: "Ed25519", x: installation.public_key },
  });
  assert(
    verify(
      null,
      Buffer.from(canonical),
      key,
      Buffer.from(headers["x-orbit-proof-signature"] ?? "", "base64url"),
    ),
    "The installation proof was invalid.",
  );
}

export async function startPanelFixture() {
  const wire = JSON.parse(
    await readFile(
      new URL(
        "../../crates/orbit-core/tests/fixtures/public-wire.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const png = await readFile(
    new URL("../../src-tauri/icons/32x32.png", import.meta.url),
  );
  const start = wire.start_reply.data;
  const token = wire.token_reply.data;
  const credential = wire.credential_reply.data;
  const node = wire.bootstrap_reply.data.nodes[0];
  node.connection_url = panelOrigin + "/v1";
  node.claude_base_url = panelOrigin;
  node.latency_probe_url = panelOrigin + "/healthz";
  const model = wire.models_reply.data.find(
    (item) => item.model_category === "image",
  );
  const code = "C".repeat(43);
  const authorizationUrl =
    panelOrigin + "/orbit/authorize?request=" + start.authorization_request_id;
  const imageRequests = new Set();
  const requests = new Map();
  const failures = [];
  let installation;
  let approved = false;
  let revoked = false;

  function reply(response, data, headers = {}) {
    response.writeHead(200, { "Content-Type": "application/json", ...headers });
    response.end(JSON.stringify(data));
  }

  const server = createServer(
    {
      pfx: await readFile(process.env.ORBIT_FIXTURE_PFX),
      passphrase: process.env.ORBIT_FIXTURE_PASSPHRASE,
    },
    async (request, response) => {
      const url = new URL(request.url, panelOrigin);
      const route = request.method + " " + url.pathname;
      try {
        assert.equal(request.headers.host, new URL(panelOrigin).host);
        const body = await readBody(request);
        if (route === "GET /favicon.ico" || route === "GET /healthz") {
          response.writeHead(204).end();
          return;
        }
        requests.set(route, (requests.get(route) ?? 0) + 1);
        if (route === "POST " + api + "/authorization-requests") {
          installation = JSON.parse(body);
          const callback = new URL(installation.redirect_uri);
          assert.match(callback.origin, /^http:\/\/127\.0\.0\.1:\d+$/);
          assert.equal(callback.pathname, "/orivelle-orbit/callback");
          assert.equal(callback.search, "");
          assert.equal(installation.code_challenge_method, "S256");
          assert.equal(installation.client_id, "orivelle-orbit");
          wire.bootstrap_reply.data.meta.installation_id =
            installation.installation_id;
          approved = false;
          reply(response, {
            data: {
              ...start,
              authorization_url: authorizationUrl,
              expires_at: new Date(Date.now() + 120_000).toISOString(),
            },
          });
          return;
        }
        if (
          route === "GET /orbit/authorize" ||
          route === "GET /orbit/approve"
        ) {
          assert(installation);
          assert.equal(
            url.searchParams.get("request"),
            start.authorization_request_id,
          );
          if (route === "GET /orbit/authorize") {
            response.writeHead(200, {
              "Content-Type": "text/html; charset=utf-8",
            });
            response.end(
              '<a href="/orbit/approve?request=' +
                start.authorization_request_id +
                '">Approve test account</a>',
            );
          } else {
            approved = true;
            const callback = new URL(installation.redirect_uri);
            callback.search = new URLSearchParams({
              code,
              state: installation.state,
              iss: panelOrigin,
            });
            response.writeHead(302, { Location: callback.href }).end();
          }
          return;
        }
        if (route === "POST " + api + "/token") {
          assert(approved, "The browser did not approve the request.");
          verifyProof(
            request,
            body,
            installation,
            start.authorization_request_id,
            start.proof_nonce,
          );
          const grant = JSON.parse(body);
          assert.equal(grant.grant_type, "authorization_code");
          assert.equal(grant.code, code);
          assert.equal(grant.redirect_uri, installation.redirect_uri);
          assert.equal(
            hash(grant.code_verifier, "base64url"),
            installation.code_challenge,
          );
          revoked = false;
          reply(response, {
            data: {
              ...token,
              installation_id: installation.installation_id,
              session_expires_at: new Date(
                Date.now() + 3_600_000,
              ).toISOString(),
            },
          });
          return;
        }
        if (
          route === "POST /v1/images/generations" ||
          route === "POST /v1/images/edits"
        ) {
          assert(!revoked);
          assert.equal(
            request.headers.authorization,
            "Bearer " + credential.plain_text_token,
          );
          const image = JSON.parse(body);
          assert.equal(image.model, model.id);
          assert.equal(image.response_format, "b64_json");
          if (route.endsWith("/edits")) {
            assert.equal(image.images.length, 1);
            assert.match(image.images[0].image_url, /^data:image\/png;base64,/);
          }
          const id = randomUUID();
          imageRequests.add(id);
          reply(
            response,
            { created: 1, data: [{ b64_json: png.toString("base64") }] },
            { "X-Orivelle-Request-Id": id },
          );
          return;
        }

        assert(!revoked);
        verifyProof(
          request,
          body,
          installation,
          token.session_id,
          token.proof_nonce,
          token.access_token,
        );
        switch (route) {
          case "POST " + api + "/installations/current/inference-credential":
            assert.equal(
              JSON.parse(body).credential_delivery_ticket,
              token.credential_delivery_ticket,
            );
            reply(response, wire.credential_reply);
            break;
          case "GET " + api + "/bootstrap":
            reply(response, wire.bootstrap_reply);
            break;
          case "GET " + api + "/models":
            assert.equal(url.searchParams.get("node_id"), node.id);
            reply(response, wire.models_reply);
            break;
          case "POST " + api + "/sessions/current/revoke":
            revoked = true;
            reply(response, wire.logout_reply);
            break;
          default: {
            const prefix = "GET " + api + "/usage/records/";
            assert(route.startsWith(prefix), "Unexpected Panel route.");
            const requestId = route.slice(prefix.length);
            assert(imageRequests.has(requestId));
            const fact = wire.usage_lookup_reply;
            reply(
              response,
              JSON.parse(
                JSON.stringify(fact).replaceAll(
                  fact.data.request_id,
                  requestId,
                ),
              ),
            );
          }
        }
      } catch (error) {
        failures.push(route);
        console.error(
          "Panel fixture rejected " +
            route +
            ": " +
            String(error.message).split("\n")[0],
        );
        response.writeHead(400).end();
      }
    },
  );
  server.listen(443, "127.0.0.1");
  await once(server, "listening");
  return {
    authorizationUrl,
    nodeLabel: node.name,
    modelLabel: model.display_name,
    png,
    requests,
    failures,
    close: () => {
      server.closeAllConnections();
      server.close();
    },
  };
}
