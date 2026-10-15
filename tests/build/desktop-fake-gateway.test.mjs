import assert from "node:assert/strict";
import { request as httpRequest } from "node:http";
import test from "node:test";
import { createDesktopFakeGateway } from "../../scripts/desktop-fake-gateway.mjs";

const main = "orbit-fixture-main";
const aux = "orbit-fixture-aux";
const tool = "orbit_fixture_read";
async function gateway(options, action) {
  const server = await createDesktopFakeGateway({
    protocol: "responses",
    scenario: "text",
    ttlMs: 5_000,
    ...options,
  });
  try {
    return await action(server);
  } finally {
    await server.close();
    await server.close();
    assert.equal(server.snapshot().active_requests, 0);
    assert.equal(server.snapshot().active_timers, 0);
    assert.equal(server.snapshot().open_sockets, 0);
  }
}
function send(
  server,
  route,
  body,
  { token = server.fixtureToken, headers = {} } = {},
) {
  const bytes =
    body === undefined ? undefined : Buffer.from(JSON.stringify(body));
  return new Promise((resolve, reject) => {
    const request = httpRequest(
      server.origin + route,
      {
        method: body === undefined ? "GET" : "POST",
        headers: {
          Authorization: "Bearer " + token,
          ...(bytes
            ? {
                "Content-Type": "application/json",
                "Content-Length": bytes.length,
              }
            : {}),
          ...headers,
        },
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("end", () =>
          resolve({
            status: response.statusCode,
            headers: response.headers,
            text: Buffer.concat(chunks).toString("utf8"),
            chunks: chunks.length,
          }),
        );
        response.on("error", reject);
      },
    );
    request.on("error", reject);
    request.setTimeout(2_000, () =>
      request.destroy(new Error("test_request_timeout")),
    );
    request.end(bytes);
  });
}
const bodyFor = (protocol, model = main, stream = false) =>
  protocol === "responses"
    ? { model, input: "synthetic prompt", stream }
    : {
        model,
        messages: [{ role: "user", content: "synthetic prompt" }],
        max_tokens: 32,
        stream,
      };
const toolsFor = (protocol) => [
  {
    ...(protocol === "responses" ? { type: "function" } : {}),
    name: tool,
    ...(protocol === "responses"
      ? { parameters: { type: "object" } }
      : { input_schema: { type: "object" } }),
  },
];
function events(text) {
  return text
    .trim()
    .split("\n\n")
    .map((block) => {
      const lines = block.split("\n");
      const name = lines.find((line) => line.startsWith("event: ")).slice(7);
      const data = JSON.parse(
        lines.find((line) => line.startsWith("data: ")).slice(6),
      );
      assert.equal(data.type, name);
      return { name, data };
    });
}
const payloadOf = (protocol, response) =>
  protocol === "responses" ? response.output : response.content;
const continuation = (protocol, model, payload, stream) => {
  const calls = payloadOf(protocol, payload);
  return protocol === "responses"
    ? {
        model,
        stream,
        previous_response_id: payload.id,
        input: [
          ...calls,
          ...calls.map((call) => ({
            type: "function_call_output",
            call_id: call.call_id,
            output: "synthetic tool output",
          })),
        ],
      }
    : {
        model,
        stream,
        max_tokens: 32,
        messages: [
          { role: "assistant", content: calls },
          {
            role: "user",
            content: calls.map((call) => ({
              type: "tool_result",
              tool_use_id: call.id,
              content: "synthetic tool output",
            })),
          },
        ],
      };
};

for (const protocol of ["responses", "messages"]) {
  test(
    protocol + " exposes only synthetic main/aux models and protocol routes",
    { timeout: 5_000 },
    async () => {
      await gateway({ protocol }, async (server) => {
        assert.match(server.origin, /^http:\/\/127\.0\.0\.1:\d+$/);
        assert.equal(server.fixtureIdentity.real_desktop_verified, false);
        assert.equal(server.fixtureIdentity.complete_desktop_catalog, false);
        const models = await send(server, "/v1/models");
        assert.deepEqual(
          JSON.parse(models.text).data.map((model) => model.id),
          [main, aux],
        );
        assert.equal(JSON.parse(models.text).object, "list");
        assert.equal(
          (
            await send(
              server,
              "/v1/" + (protocol === "responses" ? "messages" : "responses"),
              bodyFor(protocol),
            )
          ).status,
          404,
        );
        assert.equal(
          (
            await send(
              server,
              "/v1/" + protocol + "?scenario=other",
              bodyFor(protocol),
            )
          ).status,
          404,
        );
        for (const model of [main, aux]) {
          const result = await send(
            server,
            "/v1/" + protocol,
            bodyFor(protocol, model),
          );
          assert.equal(result.status, 200);
          const payload = JSON.parse(result.text);
          assert.equal(payload.model, model);
          assert.match(
            JSON.stringify(payloadOf(protocol, payload)),
            model === main ? /main/ : /auxiliary/,
          );
        }
        assert.equal(
          (
            await send(
              server,
              "/v1/" + protocol,
              bodyFor(protocol, "private-real-model"),
            )
          ).status,
          422,
        );
        assert.equal(server.snapshot().request_count, 6);
        assert.deepEqual(
          server
            .snapshot()
            .records.filter((row) => row.outcome === "completed")
            .map((row) => row.role),
          ["main", "aux"],
        );
      });
    },
  );
  test(
    protocol +
      " authenticates each instance and never stores sensitive request fields",
    { timeout: 5_000 },
    async () => {
      await gateway({ protocol }, async (server) => {
        await gateway({ protocol }, async (other) => {
          assert.notEqual(server.fixtureToken, other.fixtureToken);
          const privatePrompt = "PRIVATE_PROMPT_SENTINEL";
          const bad = await send(
            server,
            "/v1/" + protocol,
            {
              ...bodyFor(protocol),
              input: privatePrompt,
              messages: [{ role: "user", content: privatePrompt }],
            },
            {
              token: other.fixtureToken,
              headers: { "X-Private-Header": "PRIVATE_HEADER_SENTINEL" },
            },
          );
          assert.equal(bad.status, 401);
          const accepted = await send(server, "/v1/" + protocol, {
            ...bodyFor(protocol),
            input: privatePrompt,
            messages: [{ role: "user", content: privatePrompt }],
            metadata: {
              url: "https://never-contact.invalid",
              token: "PRIVATE_BODY_SENTINEL",
            },
          });
          assert.equal(accepted.status, 200);
          const serialized = JSON.stringify(server.snapshot());
          for (const secret of [
            privatePrompt,
            "PRIVATE_HEADER_SENTINEL",
            "PRIVATE_BODY_SENTINEL",
            server.fixtureToken,
            other.fixtureToken,
          ])
            assert.equal(serialized.includes(secret), false);
          assert.equal(server.snapshot().request_count, 2);
          assert.equal(other.snapshot().request_count, 0);
        });
      });
    },
  );
  test(
    protocol + " emits incremental text SSE with exactly one success terminal",
    { timeout: 5_000 },
    async () => {
      await gateway({ protocol }, async (server) => {
        const result = await send(
          server,
          "/v1/" + protocol,
          bodyFor(protocol, aux, true),
        );
        assert.match(result.headers["content-type"], /^text\/event-stream/);
        const entries = events(result.text);
        const names = entries.map((entry) => entry.name);
        if (protocol === "responses") {
          assert.equal(names[0], "response.created");
          assert.ok(
            names.indexOf("response.output_text.delta") <
              names.indexOf("response.completed"),
          );
          assert.equal(
            names.filter((name) => name === "response.completed").length,
            1,
          );
          assert.equal(entries.at(-1).data.response.model, aux);
          assert.deepEqual(
            entries.map((entry) => entry.data.sequence_number),
            entries.map((_, index) => index),
          );
        } else {
          assert.deepEqual(names, [
            "message_start",
            "content_block_start",
            "content_block_delta",
            "content_block_stop",
            "message_delta",
            "message_stop",
          ]);
          assert.equal(entries[0].data.message.model, aux);
          assert.equal(entries.at(-2).data.delta.stop_reason, "end_turn");
        }
        assert.ok(result.chunks > 1);
        assert.equal(server.snapshot().request_count, 1);
        assert.equal(server.snapshot().records[0].success_terminal_count, 1);
      });
    },
  );
  for (const scenario of ["single-tool", "parallel-tools"]) {
    for (const stream of [false, true]) {
      test(
        protocol +
          " " +
          scenario +
          (stream ? " SSE" : " JSON") +
          " completes an exact call-id replay loop",
        { timeout: 5_000 },
        async () => {
          await gateway({ protocol, scenario }, async (server) => {
            const first = await send(server, "/v1/" + protocol, {
              ...bodyFor(protocol, main, stream),
              tools: toolsFor(protocol),
            });
            assert.equal(first.status, 200);
            let payload;
            if (stream) {
              const entries = events(first.text);
              if (protocol === "responses") {
                payload = entries.at(-1).data.response;
                const fragments = new Map();
                for (const entry of entries.filter(
                  (entry) =>
                    entry.name === "response.function_call_arguments.delta",
                ))
                  fragments.set(
                    entry.data.item_id,
                    (fragments.get(entry.data.item_id) ?? "") +
                      entry.data.delta,
                  );
                for (const call of payload.output)
                  assert.equal(fragments.get(call.id), call.arguments);
              } else {
                const content = entries
                  .filter((entry) => entry.name === "content_block_start")
                  .map((entry) => ({ ...entry.data.content_block, input: {} }));
                for (const [index, block] of content.entries()) {
                  const json = entries
                    .filter(
                      (entry) =>
                        entry.name === "content_block_delta" &&
                        entry.data.index === index,
                    )
                    .map((entry) => entry.data.delta.partial_json)
                    .join("");
                  block.input = JSON.parse(json);
                }
                payload = { id: entries[0].data.message.id, content };
                assert.equal(entries.at(-2).data.delta.stop_reason, "tool_use");
              }
            } else {
              payload = JSON.parse(first.text);
            }
            assert.equal(
              payloadOf(protocol, payload).length,
              scenario === "single-tool" ? 1 : 2,
            );
            const next = continuation(protocol, main, payload, stream);
            const done = await send(server, "/v1/" + protocol, next);
            assert.equal(done.status, 200);
            assert.match(done.text, /Synthetic main fixture complete/);
            const replay = await send(server, "/v1/" + protocol, next);
            assert.equal(replay.status, 409);
            assert.equal(server.snapshot().request_count, 3);
            assert.deepEqual(
              server.snapshot().records.map((row) => row.outcome),
              ["tool_call", "completed", "fixture_tool_state_conflict"],
            );
          });
        },
      );
    }
  }
  test(
    protocol +
      " binds pending tool calls to model role and checks duplicate/foreign call IDs",
    { timeout: 5_000 },
    async () => {
      await gateway(
        { protocol, scenario: "parallel-tools" },
        async (server) => {
          const first = JSON.parse(
            (
              await send(server, "/v1/" + protocol, {
                ...bodyFor(protocol),
                tools: toolsFor(protocol),
              })
            ).text,
          );
          const valid = continuation(protocol, main, first, false);
          const wrongRole = { ...valid, model: aux };
          assert.equal(
            (await send(server, "/v1/" + protocol, wrongRole)).status,
            409,
          );
          const duplicate = structuredClone(valid);
          if (protocol === "responses") {
            duplicate.input.at(-1).call_id = duplicate.input.at(-2).call_id;
          } else {
            duplicate.messages.at(-1).content[1].tool_use_id =
              duplicate.messages.at(-1).content[0].tool_use_id;
          }
          assert.equal(
            (await send(server, "/v1/" + protocol, duplicate)).status,
            409,
          );
          assert.equal(
            (await send(server, "/v1/" + protocol, valid)).status,
            200,
          );
          assert.equal(server.snapshot().request_count, 4);
        },
      );
    },
  );
}
test("factory rejects mutable/external scope and increased resource budgets before listening", async () => {
  for (const options of [
    { endpoint: "https://upstream.invalid" },
    { token: "real-secret" },
    { models: [main] },
    { scenario: "from-header" },
    { protocol: "http-proxy" },
    { ttlMs: 600_001 },
    { ttlMs: 0 },
    { limits: { maxRequests: 65 } },
    { limits: { maxConcurrent: 5 } },
    { limits: { maxRequestBytes: 2 * 1024 * 1024 + 1 } },
  ])
    await assert.rejects(
      createDesktopFakeGateway({
        protocol: "responses",
        scenario: "text",
        ...options,
      }),
      /fixture_invalid/,
    );
});
