import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { request as httpRequest } from "node:http";
import { connect } from "node:net";
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
    assert.equal(server.snapshot().live_socket_promises, 0);
  }
}
function send(
  server,
  route,
  body,
  { token = server.fixtureToken, headers = {}, rawBody } = {},
) {
  const bytes =
    rawBody ??
    (body === undefined ? undefined : Buffer.from(JSON.stringify(body)));
  return new Promise((resolve, reject) => {
    const request = httpRequest(
      server.origin + route,
      {
        agent: false,
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
          assert.equal(server.fixtureToken !== other.fixtureToken, true);
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
          for (const entry of entries.filter((entry) =>
            [
              "response.output_text.delta",
              "response.output_text.done",
            ].includes(entry.name),
          ))
            assert.deepEqual(entry.data.logprobs, []);
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

async function until(predicate) {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
  assert.fail("fixture_condition_timeout");
}
for (const protocol of ["responses", "messages"]) {
  test(
    protocol +
      " rejects incomplete, foreign, reordered and role-invalid tool history without consuming the real calls",
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
          const variants = [];
          if (protocol === "responses") {
            const noHistory = structuredClone(valid);
            delete noHistory.previous_response_id;
            noHistory.input = noHistory.input.filter(
              (item) => item.type === "function_call_output",
            );
            variants.push(noHistory);
            const wrongPrevious = structuredClone(valid);
            wrongPrevious.previous_response_id = "resp_fixture_foreign";
            variants.push(wrongPrevious);
            const reordered = structuredClone(valid);
            reordered.input.reverse();
            variants.push(reordered);
            const badName = structuredClone(valid);
            badName.input[0].name = "unapproved_tool";
            variants.push(badName);
            const missing = structuredClone(valid);
            missing.input.pop();
            variants.push(missing);
            const foreign = structuredClone(valid);
            foreign.input.at(-1).call_id = "call_fixture_foreign";
            variants.push(foreign);
          } else {
            const badAssistant = structuredClone(valid);
            badAssistant.messages[0].role = "user";
            variants.push(badAssistant);
            const badUser = structuredClone(valid);
            badUser.messages[1].role = "assistant";
            variants.push(badUser);
            const reordered = structuredClone(valid);
            reordered.messages.reverse();
            variants.push(reordered);
            const noHistory = structuredClone(valid);
            noHistory.messages.shift();
            variants.push(noHistory);
            const gap = structuredClone(valid);
            gap.messages.splice(1, 0, {
              role: "user",
              content: "synthetic interruption",
            });
            variants.push(gap);
            const prefix = structuredClone(valid);
            prefix.messages[1].content.unshift({
              type: "text",
              text: "synthetic prefix",
            });
            variants.push(prefix);
            const missing = structuredClone(valid);
            missing.messages[1].content.pop();
            variants.push(missing);
            const foreign = structuredClone(valid);
            foreign.messages[1].content[0].tool_use_id = "call_fixture_foreign";
            variants.push(foreign);
          }
          for (const variant of variants) {
            assert.equal(
              (await send(server, "/v1/" + protocol, variant)).status,
              409,
            );
            assert.equal(server.snapshot().pending_tool_roles, 1);
          }
          const last = structuredClone(valid);
          if (protocol === "responses")
            last.input = last.input.filter(
              (item) => item.type === "function_call_output",
            );
          assert.equal(
            (await send(server, "/v1/" + protocol, last)).status,
            200,
          );
          const state = server.snapshot();
          assert.equal(state.pending_tool_roles, 0);
          assert.equal(state.request_count, variants.length + 2);
          assert.ok(
            state.records
              .slice(1, variants.length + 1)
              .every(
                (row) => row.status === 409 && row.success_terminal_count === 0,
              ),
          );
        },
      );
    },
  );
  test(
    protocol +
      " JSON response budget failure releases only undelivered tool state",
    { timeout: 5_000 },
    async () => {
      await gateway(
        {
          protocol,
          scenario: "single-tool",
          limits: { maxResponseBytes: 256 },
        },
        async (server) => {
          for (let index = 0; index < 2; index++) {
            const result = await send(server, "/v1/" + protocol, {
              ...bodyFor(protocol),
              tools: toolsFor(protocol),
            });
            assert.equal(result.status, 500);
            assert.ok(Buffer.byteLength(result.text) <= 256);
            assert.equal(server.snapshot().pending_tool_roles, 0);
          }
          const state = server.snapshot();
          assert.equal(state.request_count, 2);
          assert.ok(
            state.records.every(
              (row) =>
                row.outcome === "fixture_response_limit" &&
                row.success_terminal_count === 0,
            ),
          );
        },
      );
    },
  );
  test(
    protocol +
      " rejects malformed UTF-8 rather than silently replacing request bytes",
    { timeout: 5_000 },
    async () => {
      await gateway({ protocol }, async (server) => {
        const rawBody = Buffer.concat([
          Buffer.from('{"model":"'),
          Buffer.from([0xc3]),
          Buffer.from('"}'),
        ]);
        assert.equal(
          (await send(server, "/v1/" + protocol, {}, { rawBody })).status,
          400,
        );
        assert.equal(server.snapshot().request_count, 1);
        assert.equal(server.snapshot().records[0].success_terminal_count, 0);
      });
    },
  );
}
test(
  "empty and malformed socket churn cannot retain promises or exceed the cumulative connection budget",
  { timeout: 5_000 },
  async () => {
    await gateway({ limits: { maxRequests: 6 } }, async (server) => {
      const address = new URL(server.origin);
      for (let index = 0; index < 6; index++) {
        await new Promise((resolve, reject) => {
          const socket = connect(
            { host: "127.0.0.1", port: Number(address.port) },
            () => {
              socket.end(index % 2 === 0 ? undefined : "INVALID HTTP\r\n\r\n");
            },
          );
          socket.once("error", reject);
          socket.once("close", resolve);
        });
        await until(() => server.snapshot().live_socket_promises === 0);
      }
      await until(() => server.snapshot().closed_reason !== null);
      const state = server.snapshot();
      assert.equal(state.accepted_connections, 6);
      assert.equal(state.request_count, 0);
      assert.equal(state.active_timers, 0);
      assert.equal(state.open_sockets, 0);
      assert.equal(state.live_socket_promises, 0);
      assert.ok(state.parser_rejected > 0);
      await assert.rejects(send(server, "/v1/models"));
    });
  },
);

function noSuccess(server, count) {
  const state = server.snapshot();
  assert.equal(state.request_count, count);
  assert.ok(
    state.records.every(
      (row) =>
        row.success_terminal_count === 0 &&
        !["completed", "tool_call"].includes(row.outcome),
    ),
  );
}

for (const protocol of ["responses", "messages"]) {
  test(
    protocol +
      " keeps main/aux tool ownership independent and accepts a matching error result without leaking it",
    { timeout: 5_000 },
    async () => {
      await gateway({ protocol, scenario: "single-tool" }, async (server) => {
        const issued = new Map();
        for (const model of [main, aux]) {
          const result = await send(server, "/v1/" + protocol, {
            ...bodyFor(protocol, model),
            tools: toolsFor(protocol),
          });
          assert.equal(result.status, 200);
          issued.set(model, JSON.parse(result.text));
        }
        assert.equal(server.snapshot().pending_tool_roles, 2);
        const mainDone = continuation(protocol, main, issued.get(main), false);
        assert.equal(
          (await send(server, "/v1/" + protocol, mainDone)).status,
          200,
        );
        assert.equal(server.snapshot().pending_tool_roles, 1);
        const foreign = { ...mainDone, model: aux };
        assert.equal(
          (await send(server, "/v1/" + protocol, foreign)).status,
          409,
        );
        assert.equal(server.snapshot().pending_tool_roles, 1);
        const errorResult = continuation(protocol, aux, issued.get(aux), false);
        if (protocol === "messages") {
          errorResult.messages[1].content[0].is_error = true;
          errorResult.messages[1].content[0].content =
            "PRIVATE_TOOL_ERROR_CANARY";
        } else {
          errorResult.input.at(-1).output = "PRIVATE_TOOL_ERROR_CANARY";
        }
        const recovered = await send(server, "/v1/" + protocol, errorResult);
        assert.equal(recovered.status, 200);
        assert.match(recovered.text, /Synthetic auxiliary fixture complete/);
        assert.equal(
          JSON.stringify(server.snapshot()).includes(
            "PRIVATE_TOOL_ERROR_CANARY",
          ),
          false,
        );
        assert.equal(
          recovered.text.includes("PRIVATE_TOOL_ERROR_CANARY"),
          false,
        );
        assert.equal(server.snapshot().pending_tool_roles, 0);
        assert.equal(server.snapshot().request_count, 5);
      });
    },
  );
}

test(
  "Messages requires a complete result prefix but permits a safe trailing text reminder",
  { timeout: 5_000 },
  async () => {
    await gateway(
      { protocol: "messages", scenario: "parallel-tools" },
      async (server) => {
        const first = JSON.parse(
          (
            await send(server, "/v1/messages", {
              ...bodyFor("messages"),
              tools: toolsFor("messages"),
            })
          ).text,
        );
        const valid = continuation("messages", main, first, false);
        const interleaved = structuredClone(valid);
        interleaved.messages[1].content.splice(1, 0, {
          type: "text",
          text: "PRIVATE_REMINDER_CANARY",
        });
        assert.equal(
          (await send(server, "/v1/messages", interleaved)).status,
          409,
        );
        assert.equal(server.snapshot().pending_tool_roles, 1);
        valid.messages[1].content.push({
          type: "text",
          text: "PRIVATE_REMINDER_CANARY",
        });
        const complete = await send(server, "/v1/messages", valid);
        assert.equal(complete.status, 200);
        assert.equal(complete.text.includes("PRIVATE_REMINDER_CANARY"), false);
        assert.equal(
          JSON.stringify(server.snapshot()).includes("PRIVATE_REMINDER_CANARY"),
          false,
        );
        assert.equal(server.snapshot().pending_tool_roles, 0);
        assert.equal(server.snapshot().request_count, 3);
        assert.equal(server.snapshot().records[1].success_terminal_count, 0);
      },
    );
  },
);

// Use actual response data and close/abort events as barriers, not a Desktop.
function openPartial(server, body) {
  return new Promise((resolve, reject) => {
    const parts = [];
    let started = false;
    let settleClosed;
    const closed = new Promise((done) => {
      settleClosed = done;
    });
    const bytes = Buffer.from(JSON.stringify(body));
    const request = httpRequest(
      server.origin + "/v1/" + server.fixtureIdentity.protocol,
      {
        agent: false,
        method: "POST",
        headers: {
          Authorization: "Bearer " + server.fixtureToken,
          "Content-Type": "application/json",
          "Content-Length": bytes.length,
        },
      },
      (response) => {
        response.on("error", () => {});
        response.once("close", settleClosed);
        response.on("data", (part) => {
          parts.push(part);
          if (!started) {
            started = true;
            resolve({
              request,
              response,
              closed,
              text: () => Buffer.concat(parts).toString("utf8"),
              abort() {
                response.destroy();
                request.destroy();
              },
            });
          }
        });
        response.once("end", () => {
          if (!started) reject(new Error("test_missing_partial_data"));
        });
      },
    );
    request.on("error", () => {
      settleClosed();
      if (!started) reject(new Error("test_partial_request_failed"));
    });
    request.end(bytes);
  });
}

function unfinishedToolPayload(protocol, entries) {
  if (protocol === "responses")
    return {
      id: entries[0].data.response.id,
      output: entries
        .filter((entry) => entry.name === "response.output_item.done")
        .map((entry) => entry.data.item),
    };
  const content = entries
    .filter((entry) => entry.name === "content_block_start")
    .map((entry) => ({ ...entry.data.content_block }));
  for (const [index, block] of content.entries())
    block.input = JSON.parse(
      entries
        .filter(
          (entry) =>
            entry.name === "content_block_delta" && entry.data.index === index,
        )
        .map((entry) => entry.data.delta.partial_json)
        .join(""),
    );
  return { id: entries[0].data.message.id, content };
}

for (const protocol of ["responses", "messages"]) {
  for (const [scenario, status, type] of [
    ["auth-error", 401, "authentication_error"],
    ["rate-limit", 429, "rate_limit_error"],
    [
      "upstream-error",
      502,
      protocol === "messages" ? "api_error" : "server_error",
    ],
  ]) {
    for (const stream of [false, true])
      test(
        protocol +
          " " +
          scenario +
          (stream ? " for stream request" : " JSON") +
          " is one request with the protocol error type",
        { timeout: 5_000 },
        async () => {
          await gateway({ protocol, scenario }, async (server) => {
            const result = await send(
              server,
              "/v1/" + protocol,
              bodyFor(protocol, main, stream),
            );
            assert.equal(result.status, status);
            assert.match(result.headers["content-type"], /^application\/json/);
            const payload = JSON.parse(result.text);
            assert.equal(payload.error.type, type);
            if (protocol === "messages") assert.equal(payload.type, "error");
            assert.equal(result.text.includes("response.completed"), false);
            assert.equal(result.text.includes("message_stop"), false);
            noSuccess(server, 1);
          });
        },
      );
  }
  for (const scenario of ["stream-error", "malformed-stream", "disconnect"]) {
    test(
      protocol +
        " " +
        scenario +
        " ends without inventing a success terminal or retry",
      { timeout: 5_000 },
      async () => {
        await gateway({ protocol, scenario }, async (server) => {
          const result = await send(
            server,
            "/v1/" + protocol,
            bodyFor(protocol, aux, true),
          );
          assert.equal(result.status, 200);
          assert.equal(result.text.includes("response.completed"), false);
          assert.equal(result.text.includes("message_stop"), false);
          if (scenario === "malformed-stream") {
            assert.match(result.text, /data: \{not-json/);
            assert.throws(() => events(result.text));
          } else {
            const entries = events(result.text);
            if (scenario === "stream-error") {
              assert.equal(
                entries.at(-1).name,
                protocol === "responses" ? "response.failed" : "error",
              );
              if (protocol === "responses") {
                assert.equal(entries.at(-1).data.response.status, "failed");
                assert.deepEqual(
                  entries.map((entry) => entry.data.sequence_number),
                  [0, 1],
                );
              } else {
                assert.equal(
                  entries.at(-1).data.error.type,
                  "overloaded_error",
                );
              }
            }
          }
          noSuccess(server, 1);
          assert.equal(
            server.snapshot().records[0].outcome,
            "fixture_" + scenario.replaceAll("-", "_"),
          );
        });
      },
    );
  }
  for (const stream of [false, true]) {
    test(
      protocol +
        (stream ? " SSE" : " JSON") +
        " abort releases unissued tool calls and old abort cannot clear the next owner",
      { timeout: 5_000 },
      async () => {
        await gateway({ protocol, scenario: "stall" }, async (server) => {
          const body = {
            ...bodyFor(protocol, main, stream),
            tools: toolsFor(protocol),
          };
          const first = await openPartial(server, body);
          let firstId;
          if (stream) {
            await until(() =>
              first
                .text()
                .includes(
                  protocol === "responses"
                    ? "response.output_item.done"
                    : "content_block_stop",
                ),
            );
            const partial = unfinishedToolPayload(
              protocol,
              events(first.text()),
            );
            firstId = payloadOf(protocol, partial)[0][
              protocol === "responses" ? "call_id" : "id"
            ];
            assert.equal(
              (
                await send(
                  server,
                  "/v1/" + protocol,
                  continuation(protocol, main, partial, false),
                )
              ).status,
              409,
            );
          }
          assert.equal(server.snapshot().pending_tool_roles, 1);
          first.abort();
          await first.closed;
          await until(
            () =>
              server.snapshot().pending_tool_roles === 0 &&
              server.snapshot().active_requests === 0,
          );
          const next = await openPartial(server, body);
          if (stream) {
            await until(() =>
              next
                .text()
                .includes(
                  protocol === "responses"
                    ? "response.output_item.done"
                    : "content_block_stop",
                ),
            );
            const nextPayload = unfinishedToolPayload(
              protocol,
              events(next.text()),
            );
            assert.equal(
              payloadOf(protocol, nextPayload)[0][
                protocol === "responses" ? "call_id" : "id"
              ] === firstId,
              false,
            );
          }
          first.abort();
          assert.equal(server.snapshot().pending_tool_roles, 1);
          next.abort();
          await next.closed;
          await until(() => server.snapshot().active_requests === 0);
          assert.equal(server.snapshot().pending_tool_roles, 0);
          noSuccess(server, stream ? 3 : 2);
        });
      },
    );
  }
  test(
    protocol +
      " timeout and explicit close settle stalled work with no success",
    { timeout: 5_000 },
    async () => {
      await gateway(
        { protocol, scenario: "stall", limits: { requestTimeoutMs: 250 } },
        async (server) => {
          const partial = await openPartial(
            server,
            bodyFor(protocol, main, true),
          );
          await partial.closed;
          await until(() => server.snapshot().active_requests === 0);
          assert.equal(server.snapshot().records[0].outcome, "request_timeout");
          noSuccess(server, 1);
        },
      );
      await gateway({ protocol, scenario: "stall" }, async (server) => {
        const partial = await openPartial(
          server,
          bodyFor(protocol, main, true),
        );
        await server.close();
        await partial.closed;
        assert.equal(server.snapshot().closed_reason, "manual");
        assert.equal(server.snapshot().active_timers, 0);
        assert.equal(server.snapshot().pending_responses, 0);
        noSuccess(server, 1);
      });
    },
  );
  test(
    protocol +
      " response/event budgets include the fixed rejection and never exceed reduced caps",
    { timeout: 5_000 },
    async () => {
      for (const limits of [{ maxEvents: 1 }, { maxResponseBytes: 256 }]) {
        await gateway({ protocol, limits }, async (server) => {
          const result = await send(
            server,
            "/v1/" + protocol,
            bodyFor(protocol, main, true),
          );
          assert.equal(result.status, 500);
          const state = server.snapshot();
          assert.ok(state.records[0].event_count <= state.limits.maxEvents);
          assert.ok(
            Buffer.byteLength(result.text) <= state.limits.maxResponseBytes,
          );
          noSuccess(server, 1);
        });
      }
      await gateway(
        { protocol, limits: { maxResponseBytes: 1 } },
        async (server) => {
          await assert.rejects(
            send(server, "/v1/" + protocol, bodyFor(protocol)),
          );
          await until(() => server.snapshot().active_requests === 0);
          assert.equal(server.snapshot().records[0].response_bytes, 0);
          noSuccess(server, 1);
        },
      );
    },
  );
  test(
    protocol + " request body has a byte budget before JSON interpretation",
    { timeout: 5_000 },
    async () => {
      await gateway(
        { protocol, limits: { maxRequestBytes: 256 } },
        async (server) => {
          const base = { ...bodyFor(protocol), unused: "" };
          base.unused = "x".repeat(
            256 - Buffer.byteLength(JSON.stringify(base)),
          );
          assert.equal(Buffer.byteLength(JSON.stringify(base)), 256);
          assert.equal(
            (await send(server, "/v1/" + protocol, base)).status,
            200,
          );
          assert.equal(server.snapshot().request_count, 1);
        },
      );
      await gateway(
        { protocol, limits: { maxRequestBytes: 256 } },
        async (server) => {
          const result = await send(
            server,
            "/v1/" + protocol,
            bodyFor(protocol),
            {
              rawBody: Buffer.from(
                JSON.stringify({
                  ...bodyFor(protocol),
                  unused: "x".repeat(256),
                }),
              ),
            },
          );
          assert.equal(result.status, 413);
          noSuccess(server, 1);
        },
      );
    },
  );
}

test(
  "four stalled admissions reject the fifth request without increasing admitted concurrency",
  { timeout: 5_000 },
  async () => {
    await gateway({ scenario: "stall" }, async (server) => {
      const held = await Promise.all(
        Array.from({ length: 4 }, () =>
          openPartial(server, bodyFor("responses", main, true)),
        ),
      );
      assert.equal(server.snapshot().active_requests, 4);
      const fifth = await send(
        server,
        "/v1/responses",
        bodyFor("responses", aux, true),
      );
      assert.equal(fifth.status, 429);
      assert.equal(JSON.parse(fifth.text).error.type, "rate_limit_error");
      assert.equal(server.snapshot().active_requests, 4);
      await server.close();
      await Promise.all(held.map((partial) => partial.closed));
      noSuccess(server, 5);
      assert.equal(server.snapshot().active_requests, 0);
    });
  },
);
test(
  "request budget and TTL stop accepting connections and clear all timers",
  { timeout: 5_000 },
  async () => {
    await gateway({ limits: { maxRequests: 2 } }, async (server) => {
      assert.equal((await send(server, "/v1/models")).status, 200);
      assert.equal(
        (await send(server, "/v1/responses", bodyFor("responses"))).status,
        200,
      );
      await until(() => server.snapshot().closed_reason !== null);
      await server.close();
      assert.equal(server.snapshot().request_count, 2);
      assert.equal(server.snapshot().accepted_connections, 2);
      await assert.rejects(send(server, "/v1/models"));
    });
    await gateway({ scenario: "stall", ttlMs: 400 }, async (server) => {
      const partial = await openPartial(
        server,
        bodyFor("responses", main, true),
      );
      await partial.closed;
      await until(() => server.snapshot().closed_reason === "ttl");
      await server.close();
      assert.equal(server.snapshot().active_timers, 0);
      noSuccess(server, 1);
      await assert.rejects(send(server, "/v1/models"));
    });
  },
);
function rawExchange(server, bytes) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    const socket = connect(
      { host: "127.0.0.1", port: Number(new URL(server.origin).port) },
      () => socket.write(bytes),
    );
    socket.on("data", (part) => chunks.push(part));
    socket.on("error", (error) => {
      if (error.code !== "ECONNRESET")
        reject(new Error("test_raw_socket_failed"));
    });
    socket.once("close", () => resolve(Buffer.concat(chunks).toString("utf8")));
  });
}
test(
  "pipelined requests and Expect cannot bypass request accounting with Node automatic responses",
  { timeout: 5_000 },
  async () => {
    await gateway({}, async (server) => {
      const address = new URL(server.origin).host;
      const request =
        "GET /v1/models HTTP/1.1\r\nHost: " +
        address +
        "\r\nAuthorization: Bearer " +
        server.fixtureToken +
        "\r\n\r\n";
      const result = await rawExchange(server, request.repeat(3));
      await until(() => server.snapshot().open_sockets === 0);
      assert.equal(server.snapshot().request_count, 1);
      assert.ok(server.snapshot().pipeline_rejected > 0);
      assert.ok((result.match(/HTTP\/1\.1 /g) ?? []).length <= 1);
      assert.equal(result.includes("503 Service Unavailable"), false);
    });
    for (const expectation of ["100-continue", "synthetic-expectation"]) {
      await gateway({}, async (server) => {
        const address = new URL(server.origin).host;
        const result = await rawExchange(
          server,
          "POST /v1/responses HTTP/1.1\r\nHost: " +
            address +
            "\r\nExpect: " +
            expectation +
            "\r\nContent-Length: 50\r\n\r\n",
        );
        assert.match(result, /^HTTP\/1\.1 417 /);
        assert.equal(result.includes("100 Continue"), false);
        noSuccess(server, 1);
      });
    }
  },
);
test(
  "scenario and scope remain immutable after creation and request headers cannot select faults",
  { timeout: 5_000 },
  async () => {
    const options = {
      protocol: "responses",
      scenario: "text",
      ttlMs: 5_000,
      limits: { maxRequests: 6 },
    };
    const server = await createDesktopFakeGateway(options);
    try {
      options.scenario = "upstream-error";
      options.limits.maxRequests = 1;
      const result = await send(server, "/v1/responses", bodyFor("responses"), {
        headers: { "X-Fixture-Scenario": "upstream-error" },
      });
      assert.equal(result.status, 200);
      const snapshot = server.snapshot();
      assert.equal(snapshot.scenario, "text");
      assert.equal(snapshot.limits.maxRequests, 6);
      snapshot.records[0].model_id = "foreign";
      assert.equal(server.snapshot().records[0].model_id, main);
    } finally {
      await server.close();
    }
  },
);

test(
  "default lifetime and protocol-specific non-generating routes remain explicit fixture facts",
  { timeout: 5_000 },
  async () => {
    const server = await createDesktopFakeGateway({
      protocol: "messages",
      scenario: "text",
    });
    try {
      assert.equal(server.snapshot().ttl_ms, 120_000);
      const result = await send(
        server,
        "/v1/messages/count_tokens",
        bodyFor("messages", aux),
      );
      assert.equal(result.status, 200);
      assert.deepEqual(JSON.parse(result.text), { input_tokens: 1 });
      assert.equal(server.snapshot().records[0].outcome, "count_tokens");
      assert.equal(server.snapshot().request_count, 1);
      assert.equal(server.snapshot().records[0].success_terminal_count, 0);
    } finally {
      await server.close();
    }
  },
);

test(
  "isolated Node stdout/stderr contain only safe facts, never request headers, body, tool results or either token",
  { timeout: 5_000 },
  async () => {
    const program = [
      'import {request} from "node:http";',
      "const {createDesktopFakeGateway} = await import(process.argv[1]);",
      'const server = await createDesktopFakeGateway({protocol:"responses",scenario:"single-tool",ttlMs:1000});',
      "async function one(body,token=server.fixtureToken){return await new Promise((resolve,reject)=>{",
      "const bytes=Buffer.from(JSON.stringify(body));",
      'const req=request(server.origin+"/v1/responses",{agent:false,method:"POST",headers:{Authorization:"Bearer "+token,"Content-Type":"application/json","Content-Length":bytes.length,"X-Private-Fixture":"PRIVATE_HEADER_CANARY"}},res=>{',
      'const parts=[];res.on("data",part=>parts.push(part));res.on("end",()=>resolve(JSON.parse(Buffer.concat(parts).toString())));res.on("error",()=>reject(new Error("fixture_client_failed")));});',
      'req.on("error",()=>reject(new Error("fixture_client_failed")));req.end(bytes);});}',
      "try {",
      'const first=await one({model:"orbit-fixture-main",input:"PRIVATE_PROMPT_CANARY",metadata:{secret:"PRIVATE_BODY_CANARY",url:"https://never-forward.invalid"},tools:[{type:"function",name:"orbit_fixture_read",parameters:{type:"object"}}]});',
      "const call=first.output[0];",
      'await one({model:"orbit-fixture-main",previous_response_id:first.id,input:[call,{type:"function_call_output",call_id:call.call_id,output:"PRIVATE_TOOL_ARGUMENT_CANARY"}]});',
      'await one({model:"orbit-fixture-main",input:"PRIVATE_PROMPT_CANARY"},"PRIVATE_TOKEN_CANARY");',
      "await server.close();process.stdout.write(JSON.stringify(server.snapshot()));",
      '} catch {process.stderr.write("fixture_child_failed");process.exitCode=1;} finally {await server.close();}',
    ].join("\n");
    const env = Object.fromEntries(
      Object.entries(process.env).filter(
        ([key]) =>
          ![
            "NODE_DEBUG",
            "NODE_DEBUG_NATIVE",
            "NODE_OPTIONS",
            "NODE_USE_ENV_PROXY",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
          ].includes(key.toUpperCase()),
      ),
    );
    Object.assign(env, {
      HTTP_PROXY: "http://127.0.0.1:1",
      HTTPS_PROXY: "http://127.0.0.1:1",
      NO_PROXY: "",
    });
    if (process.platform !== "win32")
      Object.assign(env, {
        http_proxy: "http://127.0.0.1:1",
        https_proxy: "http://127.0.0.1:1",
        no_proxy: "",
      });
    const captured = await new Promise((resolve, reject) => {
      const child = spawn(
        process.execPath,
        [
          "--use-env-proxy",
          "--input-type=module",
          "-e",
          program,
          new URL("../../scripts/desktop-fake-gateway.mjs", import.meta.url)
            .href,
        ],
        {
          env,
          shell: false,
          stdio: ["ignore", "pipe", "pipe"],
          windowsHide: true,
        },
      );
      const stdout = [];
      const stderr = [];
      let length = 0;
      let exceeded = false;
      const stop = () => {
        exceeded = true;
        child.kill("SIGKILL");
      };
      const timer = setTimeout(stop, 2_000);
      const collect = (target) => (part) => {
        length += part.length;
        if (length > 16 * 1024) stop();
        else target.push(part);
      };
      child.stdout.on("data", collect(stdout));
      child.stderr.on("data", collect(stderr));
      child.once("error", () => {
        clearTimeout(timer);
        reject(new Error("fixture_child_spawn_failed"));
      });
      child.once("close", (code) => {
        clearTimeout(timer);
        if (exceeded) reject(new Error("fixture_child_resource_limit"));
        else
          resolve({
            code,
            stdout: Buffer.concat(stdout).toString(),
            stderr: Buffer.concat(stderr).toString(),
          });
      });
    });
    assert.equal(captured.code === 0, true, "fixture_child_exit_failed");
    assert.equal(
      captured.stderr.length === 0,
      true,
      "fixture_child_stderr_not_empty",
    );
    for (const canary of [
      "PRIVATE_HEADER_CANARY",
      "PRIVATE_BODY_CANARY",
      "PRIVATE_PROMPT_CANARY",
      "PRIVATE_TOOL_ARGUMENT_CANARY",
      "PRIVATE_TOKEN_CANARY",
      "orbit-fixture-token-",
    ])
      assert.equal((captured.stdout + captured.stderr).includes(canary), false);
    let summary;
    try {
      summary = JSON.parse(captured.stdout);
    } catch {
      throw new Error("fixture_child_summary_invalid");
    }
    assert.equal(summary.request_count, 3);
    assert.equal(summary.open_sockets, 0);
    assert.equal(summary.active_timers, 0);
    assert.equal(summary.pending_tool_roles, 0);
    assert.equal(summary.evidence_level, "synthetic_protocol_fixture");
  },
);
