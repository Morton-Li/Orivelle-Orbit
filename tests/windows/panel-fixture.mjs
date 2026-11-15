import assert from "node:assert/strict";
import { createHash, createPublicKey, randomUUID, verify } from "node:crypto";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { createServer } from "node:https";
import path from "node:path";
import { responseEvents } from "../../scripts/desktop-fake-gateway.mjs";

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
  if (process.env.ORBIT_WINDOWS_TEST_SCOPE === "disposable-vm") {
    for (const catalog of [
      wire.bootstrap_reply.data.models,
      wire.models_reply.data,
    ]) {
      for (const item of catalog.filter(
        (entry) => entry.model_category === "text",
      )) {
        item.codex_catalog_binding = {
          state: "declared",
          verified: false,
          profile_id: "codex-cli-0.163.0-alpha.10.2",
          cli_version: "0.163.0-alpha.10.2",
          source_sha256:
            "943ca7fe1d19ed019054158303f0dbbd80b3bef43b3aa425a71aa7cb3fb52c2b",
          template_slug: "gpt-5.5",
        };
        item.reasoning = {
          state: "restricted",
          capability_status: "supported",
          supported_efforts: ["medium", "high"],
          allowed_efforts: ["medium", "high"],
          default_effort: "medium",
          revision: 1,
        };
      }
    }
  }
  const model = wire.models_reply.data.find(
    (item) => item.model_category === "image",
  );
  const contracts = JSON.parse(
    await readFile(
      new URL("../contracts/images-parameters.json", import.meta.url),
      "utf8",
    ),
  );
  for (const catalog of [
    wire.bootstrap_reply.data.models,
    wire.models_reply.data,
  ]) {
    for (const item of catalog.filter(
      (entry) => entry.model_category === "image",
    )) {
      // Deliberately advertise the legacy wider catalog. The current product
      // must restrict new requests while keeping old request facts readable.
      item.image_parameters = structuredClone(contracts.models["gpt-image-2"]);
    }
  }
  const code = "C".repeat(43);
  const authorizationUrl =
    panelOrigin + "/orbit/authorize?request=" + start.authorization_request_id;
  const imageRequests = new Set();
  const requests = new Map();
  const failures = [];
  const imageBodies = [];
  let nextImage;
  let installation;
  if (process.env.ORBIT_WINDOWS_TEST_SCOPE === "disposable-vm") {
    // Reruns can reopen a previously authenticated synthetic session before
    // starting a new authorization. Verify it with the persisted public key;
    // never read signing secrets or relax proof validation for those requests.
    const { DatabaseSync } = await import("node:sqlite");
    let database;
    try {
      database = new DatabaseSync(
        path.join(process.env.APPDATA, "io.orivelle.orbit", "orbit.sqlite3"),
        { readOnly: true },
      );
      const row = database
        .prepare(
          "SELECT json_extract(payload,'$.id') installation_id,json_extract(payload,'$.public_key') public_key FROM records WHERE kind='installation' AND json_extract(payload,'$.origin')=?",
        )
        .get(panelOrigin);
      if (row) installation = row;
    } catch (error) {
      if (!String(error.message).includes("unable to open database"))
        throw error;
    } finally {
      database?.close();
    }
  }
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
          const grant = JSON.parse(body);
          if (grant.grant_type === "refresh_token") {
            assert(!revoked);
            verifyProof(
              request,
              body,
              installation,
              token.session_id,
              token.proof_nonce,
            );
            assert.equal(grant.refresh_token, token.refresh_token);
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
          assert(approved, "The browser did not approve the request.");
          verifyProof(
            request,
            body,
            installation,
            start.authorization_request_id,
            start.proof_nonce,
          );
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
        if (route === "POST /v1/responses") {
          assert.equal(
            request.headers.authorization,
            "Bearer " + credential.plain_text_token,
          );
          const input = JSON.parse(body);
          assert.equal(input.model, "synthetic-responses");
          const output = [
            {
              type: "message",
              id: "msg_windows_fixture",
              status: "completed",
              role: "assistant",
              content: [
                {
                  type: "output_text",
                  text: "Windows native configuration verified.",
                  annotations: [],
                },
              ],
            },
          ];
          const result = {
            id: "resp_windows_fixture",
            object: "response",
            created_at: Math.floor(Date.now() / 1000),
            status: "completed",
            model: input.model,
            output,
            usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
            error: null,
            incomplete_details: null,
          };
          response.writeHead(200, {
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache",
          });
          for (const event of responseEvents(result))
            response.write(
              `event: ${event.name}\ndata: ${JSON.stringify(event.data)}\n\n`,
            );
          response.end();
          return;
        }
        if (route === "POST /v1/messages") {
          assert.equal(
            request.headers.authorization,
            "Bearer " + credential.plain_text_token,
          );
          const input = JSON.parse(body);
          assert.equal(input.model, "synthetic-responses");
          if (input.stream === true) {
            response.writeHead(200, {
              "Content-Type": "text/event-stream",
              "Cache-Control": "no-cache",
            });
            for (const [event, data] of [
              [
                "message_start",
                {
                  type: "message_start",
                  message: {
                    id: "msg_orbit_windows_fixture",
                    type: "message",
                    role: "assistant",
                    model: input.model,
                    content: [],
                    stop_reason: null,
                    stop_sequence: null,
                    usage: { input_tokens: 1, output_tokens: 0 },
                  },
                },
              ],
              [
                "content_block_start",
                {
                  type: "content_block_start",
                  index: 0,
                  content_block: { type: "text", text: "" },
                },
              ],
              [
                "content_block_delta",
                {
                  type: "content_block_delta",
                  index: 0,
                  delta: {
                    type: "text_delta",
                    text: "Windows native configuration verified.",
                  },
                },
              ],
              ["content_block_stop", { type: "content_block_stop", index: 0 }],
              [
                "message_delta",
                {
                  type: "message_delta",
                  delta: { stop_reason: "end_turn", stop_sequence: null },
                  usage: { output_tokens: 1 },
                },
              ],
              ["message_stop", { type: "message_stop" }],
            ])
              response.write(
                `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`,
              );
            response.end();
            return;
          }
          reply(response, {
            id: "msg_orbit_windows_fixture",
            type: "message",
            role: "assistant",
            model: input.model,
            content: [
              { type: "text", text: "Windows native configuration verified." },
            ],
            stop_reason: "end_turn",
            stop_sequence: null,
            usage: { input_tokens: 1, output_tokens: 1 },
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
          imageBodies.push({ route, body: image });
          assert.equal(image.model, model.id);
          assert.equal(image.response_format, "b64_json");
          if (route.endsWith("/edits")) {
            assert.equal(image.images.length, 1);
            assert.match(image.images[0].image_url, /^data:image\/png;base64,/);
          }
          if (nextImage) {
            const pending = nextImage;
            nextImage = undefined;
            pending.received();
            await pending.released;
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

        if (revoked) {
          // A background read can arrive after logout completed. This is an
          // expected expired session, not a fixture or proof-validation error.
          response.writeHead(401, { "Content-Type": "application/json" });
          response.end(JSON.stringify({ message: "Unauthenticated." }));
          return;
        }
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
          case "GET " + api + "/usage/summary": {
            const dateFrom = url.searchParams.get("date_from");
            const dateTo = url.searchParams.get("date_to");
            assert.match(dateFrom, /^\d{4}-\d{2}-\d{2}$/);
            assert.match(dateTo, /^\d{4}-\d{2}-\d{2}$/);
            const quantity = { value: 0, status: "known" };
            reply(response, {
              data: {
                range: {
                  date_from: dateFrom,
                  date_to: dateTo,
                  timezone: url.searchParams.get("timezone"),
                  defaulted: false,
                  max_days: 90,
                },
                summary: {
                  requests: 0,
                  succeeded: 0,
                  failed: 0,
                  cancelled: 0,
                  rejected: 0,
                  interrupted: 0,
                  usage: {
                    tokens: {
                      state: "complete",
                      total: quantity,
                      input: quantity,
                      cached_input: quantity,
                      cache_write: quantity,
                      output: quantity,
                      reasoning: quantity,
                    },
                    image_count: quantity,
                    reconciliation_pending_count: 0,
                  },
                  charges: [],
                },
                days: [],
                has_legacy_semantics: false,
                updated_at: null,
                generated_at: new Date().toISOString(),
              },
            });
            break;
          }
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
    imageBodies,
    holdNextImage: () => {
      assert.equal(nextImage, undefined);
      let received;
      let release;
      const seen = new Promise((resolve) => {
        received = resolve;
      });
      const released = new Promise((resolve) => {
        release = resolve;
      });
      nextImage = { received, released };
      return { received: seen, release };
    },
    close: () => {
      server.closeAllConnections();
      server.close();
    },
  };
}
