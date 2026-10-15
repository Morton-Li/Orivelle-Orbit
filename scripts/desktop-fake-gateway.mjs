import { randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";

// This module never forwards a request, executes a tool or contacts an upstream.
// Its HTTP loopback evidence is not Desktop configuration or HTTPS acceptance.
export const DESKTOP_FAKE_GATEWAY_LIMITS = Object.freeze({
  maxRequests: 64,
  maxConcurrent: 4,
  maxRequestBytes: 2 * 1024 * 1024,
  maxResponseBytes: 128 * 1024,
  maxEvents: 128,
  requestTimeoutMs: 5_000,
});
export const DESKTOP_FAKE_GATEWAY_SCENARIOS = Object.freeze([
  "text",
  "single-tool",
  "parallel-tools",
]);
const protocols = new Set(["responses", "messages"]);
const fixtureUrl = new URL(
  "../tests/fixtures/desktop-gateway/models.json",
  import.meta.url,
);
const fault = (code) => Object.assign(new Error(code), { code });
const copy = (value) => structuredClone(value);
const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
const equal = (left, right) => {
  const a = Buffer.from(left);
  const b = Buffer.from(right);
  return a.length === b.length && timingSafeEqual(a, b);
};

function optionsFor(options) {
  if (
    !options ||
    typeof options !== "object" ||
    Array.isArray(options) ||
    Object.keys(options).some(
      (key) => !["protocol", "scenario", "ttlMs", "limits"].includes(key),
    ) ||
    !protocols.has(options.protocol) ||
    !DESKTOP_FAKE_GATEWAY_SCENARIOS.includes(options.scenario)
  )
    throw fault("fixture_invalid_options");
  const ttlMs = options.ttlMs ?? 120_000;
  if (!Number.isSafeInteger(ttlMs) || ttlMs <= 0 || ttlMs > 600_000)
    throw fault("fixture_invalid_ttl");
  const lowered = options.limits ?? {};
  if (
    !lowered ||
    typeof lowered !== "object" ||
    Array.isArray(lowered) ||
    Object.keys(lowered).some((key) => !own(DESKTOP_FAKE_GATEWAY_LIMITS, key))
  )
    throw fault("fixture_invalid_limits");
  const limits = { ...DESKTOP_FAKE_GATEWAY_LIMITS, ...lowered };
  for (const [key, value] of Object.entries(limits)) {
    if (
      !Number.isSafeInteger(value) ||
      value <= 0 ||
      value > DESKTOP_FAKE_GATEWAY_LIMITS[key]
    )
      throw fault("fixture_invalid_limits");
  }
  return Object.freeze({
    protocol: options.protocol,
    scenario: options.scenario,
    ttlMs,
    limits: Object.freeze(limits),
  });
}

async function loadFixture() {
  const bytes = await readFile(fixtureUrl);
  if (bytes.length > 16 * 1024) throw fault("fixture_invalid_models");
  const fixture = JSON.parse(bytes.toString("utf8"));
  if (
    fixture.schema_version !== 1 ||
    fixture.evidence_level !== "synthetic_protocol_fixture" ||
    fixture.complete_desktop_catalog !== false ||
    !Array.isArray(fixture.models) ||
    fixture.models.length !== 2 ||
    fixture.models.some(
      (model, index) =>
        model.id !== ["orbit-fixture-main", "orbit-fixture-aux"][index] ||
        model.role !== ["main", "aux"][index] ||
        typeof model.text !== "string" ||
        !model.text.startsWith("Synthetic ") ||
        Buffer.byteLength(model.text) > 256,
    ) ||
    fixture.tool?.name !== "orbit_fixture_read" ||
    JSON.stringify(fixture.tool.arguments) !== '{"fixture":"synthetic-only"}'
  )
    throw fault("fixture_invalid_models");
  return fixture;
}

function errorBody(protocol, code) {
  return protocol === "messages"
    ? {
        type: "error",
        error: {
          type:
            code === "fixture_authentication_failed"
              ? "authentication_error"
              : "invalid_request_error",
          message: code,
        },
      }
    : { error: { type: "invalid_request_error", code, message: code } };
}

function authorized(request, token, protocol) {
  const names = request.rawHeaders.filter((_, index) => index % 2 === 0);
  const count = (name) =>
    names.filter((value) => value.toLowerCase() === name).length;
  if (count("authorization") > 1 || count("x-api-key") > 1) return false;
  const bearer = request.headers.authorization;
  const apiKey = request.headers["x-api-key"];
  if (apiKey !== undefined && protocol !== "messages") return false;
  if (bearer !== undefined && !equal(bearer, "Bearer " + token)) return false;
  if (apiKey !== undefined && !equal(apiKey, token)) return false;
  return bearer !== undefined || apiKey !== undefined;
}

function event(name, data) {
  return { name, data: { ...data, type: name } };
}

function responseObject(model, output, responseId, status = "completed") {
  return {
    id: responseId,
    object: "response",
    created_at: 0,
    status,
    model: model.id,
    output,
    usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
  };
}

function outputFor(protocol, model, calls) {
  if (calls)
    return calls.map((call) =>
      protocol === "responses"
        ? {
            type: "function_call",
            id: call.itemId,
            call_id: call.id,
            name: call.name,
            arguments: call.arguments,
            status: "completed",
          }
        : {
            type: "tool_use",
            id: call.id,
            name: call.name,
            input: JSON.parse(call.arguments),
          },
    );
  return protocol === "responses"
    ? [
        {
          type: "message",
          id: "msg_fixture_" + randomUUID(),
          status: "completed",
          role: "assistant",
          content: [{ type: "output_text", text: model.text, annotations: [] }],
        },
      ]
    : [{ type: "text", text: model.text }];
}

function responseEvents(response) {
  const events = [
    event("response.created", {
      response: { ...response, status: "in_progress", output: [] },
    }),
  ];
  for (const [index, item] of response.output.entries()) {
    events.push(
      event("response.output_item.added", {
        output_index: index,
        item: {
          ...item,
          status: "in_progress",
          ...(item.type === "function_call"
            ? { arguments: "" }
            : { content: [] }),
        },
      }),
    );
  }
  if (response.output[0].type === "function_call") {
    // Interleave argument fragments for parallel calls; do not execute them.
    for (const half of [0, 1]) {
      for (const [index, item] of response.output.entries()) {
        const at = Math.floor(item.arguments.length / 2);
        events.push(
          event("response.function_call_arguments.delta", {
            item_id: item.id,
            output_index: index,
            delta:
              half === 0
                ? item.arguments.slice(0, at)
                : item.arguments.slice(at),
          }),
        );
      }
    }
    for (const [index, item] of response.output.entries())
      events.push(
        event("response.function_call_arguments.done", {
          item_id: item.id,
          output_index: index,
          arguments: item.arguments,
        }),
      );
  } else {
    const item = response.output[0];
    const part = item.content[0];
    events.push(
      event("response.content_part.added", {
        item_id: item.id,
        output_index: 0,
        content_index: 0,
        part: { ...part, text: "" },
      }),
      event("response.output_text.delta", {
        item_id: item.id,
        output_index: 0,
        content_index: 0,
        delta: part.text,
      }),
      event("response.output_text.done", {
        item_id: item.id,
        output_index: 0,
        content_index: 0,
        text: part.text,
      }),
      event("response.content_part.done", {
        item_id: item.id,
        output_index: 0,
        content_index: 0,
        part,
      }),
    );
  }
  for (const [index, item] of response.output.entries())
    events.push(
      event("response.output_item.done", { output_index: index, item }),
    );
  events.push(event("response.completed", { response }));
  return events.map((entry, index) => ({
    ...entry,
    data: { ...entry.data, sequence_number: index },
  }));
}

function messageObject(model, output, responseId) {
  return {
    id: responseId,
    type: "message",
    role: "assistant",
    model: model.id,
    content: output,
    stop_reason: output[0].type === "tool_use" ? "tool_use" : "end_turn",
    stop_sequence: null,
    usage: {
      input_tokens: 1,
      output_tokens: 1,
      cache_creation_input_tokens: 0,
      cache_read_input_tokens: 0,
    },
  };
}

function messageEvents(message) {
  const events = [
    event("message_start", {
      message: {
        ...message,
        content: [],
        stop_reason: null,
        usage: { ...message.usage, output_tokens: 0 },
      },
    }),
  ];
  for (const [index, block] of message.content.entries()) {
    events.push(
      event("content_block_start", {
        index,
        content_block:
          block.type === "tool_use"
            ? { ...block, input: {} }
            : { ...block, text: "" },
      }),
    );
    if (block.type === "tool_use") {
      const argumentsText = JSON.stringify(block.input);
      const split = Math.floor(argumentsText.length / 2);
      for (const partial of [
        argumentsText.slice(0, split),
        argumentsText.slice(split),
      ])
        events.push(
          event("content_block_delta", {
            index,
            delta: { type: "input_json_delta", partial_json: partial },
          }),
        );
    } else {
      events.push(
        event("content_block_delta", {
          index,
          delta: { type: "text_delta", text: block.text },
        }),
      );
    }
    events.push(event("content_block_stop", { index }));
  }
  events.push(
    event("message_delta", {
      delta: { stop_reason: message.stop_reason, stop_sequence: null },
      usage: { output_tokens: 1 },
    }),
    event("message_stop", {}),
  );
  return events;
}

function resultsFor(protocol, body) {
  return protocol === "responses"
    ? (Array.isArray(body.input) ? body.input : [])
        .filter((item) => item?.type === "function_call_output")
        .map((item) => ({ id: item.call_id, output: item.output }))
    : body.messages.flatMap((message) =>
        (Array.isArray(message?.content) ? message.content : [])
          .filter((block) => block?.type === "tool_result")
          .map((block) => ({ id: block.tool_use_id, output: block.content })),
      );
}

function validateContinuation(protocol, body, pending, results) {
  if (
    !pending ||
    results.length !== pending.calls.length ||
    new Set(results.map((result) => result.id)).size !== results.length ||
    results.some(
      (result) =>
        typeof result.output !== "string" ||
        !pending.calls.some((call) => call.id === result.id),
    ) ||
    (protocol === "responses" &&
      body.previous_response_id !== undefined &&
      body.previous_response_id !== pending.responseId)
  )
    throw fault("fixture_tool_state_conflict");
  if (protocol === "responses") {
    const replayed = body.input.filter(
      (item) => item?.type === "function_call",
    );
    if (
      replayed.length &&
      (replayed.length !== pending.calls.length ||
        new Set(replayed.map((item) => item.call_id)).size !==
          replayed.length ||
        replayed.some(
          (item) =>
            !pending.calls.some(
              (call) =>
                call.id === item.call_id &&
                call.name === item.name &&
                call.arguments === item.arguments,
            ),
        ))
    )
      throw fault("fixture_tool_state_conflict");
  } else {
    const replayed = body.messages.flatMap((message) =>
      (Array.isArray(message?.content) ? message.content : []).filter(
        (block) => block?.type === "tool_use",
      ),
    );
    if (
      replayed.length &&
      (replayed.length !== pending.calls.length ||
        new Set(replayed.map((block) => block.id)).size !== replayed.length ||
        replayed.some(
          (block) =>
            !pending.calls.some(
              (call) =>
                call.id === block.id &&
                call.name === block.name &&
                JSON.stringify(block.input) === call.arguments,
            ),
        ))
    )
      throw fault("fixture_tool_state_conflict");
  }
}

export async function createDesktopFakeGateway(options) {
  const config = optionsFor(options);
  const fixture = await loadFixture();
  const models = new Map(fixture.models.map((model) => [model.id, model]));
  const runId = "fixture-run-" + randomUUID();
  const token = "orbit-fixture-token-" + randomBytes(32).toString("base64url");
  const identity = Object.freeze({
    run_id: runId,
    account_id: "fixture-account-" + randomUUID(),
    connection_id: "fixture-connection-" + randomUUID(),
    protocol: config.protocol,
    model_ids: Object.freeze([...models.keys()]),
    evidence_level: "synthetic_protocol_fixture",
    complete_desktop_catalog: false,
    real_desktop_verified: false,
  });
  const records = [];
  const pendingTools = new Map();
  const sockets = new Set();
  const socketClosures = new Map();
  const requests = new Map();
  const tasks = new Set();
  let origin;
  let closing = false;
  let accepting = true;
  let closePromise;
  let ttlTimer;
  let closedReason = null;
  let budgetRejected = 0;
  let parserRejected = 0;
  const server = createServer(
    { maxHeaderSize: 16 * 1024 },
    (request, response) => {
      const task = handle(request, response);
      tasks.add(task);
      void task.finally(() => tasks.delete(task));
    },
  );
  const serverClosed = new Promise((resolve) => server.once("close", resolve));
  server.maxConnections = config.limits.maxRequests;
  server.maxRequestsPerSocket = 1;
  server.headersTimeout = 5_000;
  server.requestTimeout = 5_000;
  server.keepAliveTimeout = 1;
  server.on("connection", (socket) => {
    sockets.add(socket);
    socket.on("error", () => {});
    socketClosures.set(
      socket,
      new Promise((resolve) => socket.once("close", resolve)),
    );
    socket.once("close", () => sockets.delete(socket));
    if (closing) socket.destroy();
  });
  server.on("clientError", (_, socket) => {
    parserRejected = Math.min(parserRejected + 1, config.limits.maxRequests);
    socket.destroy();
  });
  const stopAccepting = () => {
    if (accepting) {
      accepting = false;
      server.close();
    }
  };
  async function close(reason = "manual") {
    if (closePromise) return closePromise;
    closing = true;
    closedReason = reason;
    clearTimeout(ttlTimer);
    ttlTimer = undefined;
    stopAccepting();
    for (const state of requests.values()) {
      state.controller.abort();
      state.request.destroy();
      state.response.destroy();
    }
    for (const socket of sockets) socket.destroy();
    closePromise = (async () => {
      await Promise.allSettled([...tasks]);
      await serverClosed;
      await Promise.all([...socketClosures.values()]);
      socketClosures.clear();
      pendingTools.clear();
    })();
    return closePromise;
  }
  function snapshot() {
    return copy({
      schema_version: 1,
      evidence_level: "synthetic_protocol_fixture",
      run_id: runId,
      protocol: config.protocol,
      scenario: config.scenario,
      real_desktop_verified: false,
      complete_desktop_catalog: false,
      limits: config.limits,
      request_count: records.length,
      active_requests: requests.size,
      active_timers: requests.size + (ttlTimer === undefined ? 0 : 1),
      open_sockets: sockets.size,
      budget_rejected: budgetRejected,
      parser_rejected: parserRejected,
      closed_reason: closedReason,
      records,
    });
  }
  async function handle(request, response) {
    if (closing || records.length >= config.limits.maxRequests) {
      budgetRejected = Math.min(budgetRejected + 1, config.limits.maxRequests);
      response.destroy();
      return;
    }
    const record = {
      request_id: "fixture-request-" + randomUUID(),
      route: "unknown",
      model_id: null,
      role: null,
      status: null,
      outcome: "pending",
      event_count: 0,
      response_bytes: 0,
      success_terminal_count: 0,
    };
    records.push(record);
    if (records.length === config.limits.maxRequests) stopAccepting();
    const controller = new AbortController();
    const state = { request, response, controller };
    const activeBefore = requests.size;
    requests.set(response, state);
    const timer = setTimeout(() => {
      record.outcome = "request_timeout";
      controller.abort();
      request.destroy();
      response.destroy();
    }, config.limits.requestTimeoutMs);
    const ended = (bytes) =>
      new Promise((resolve) => {
        const finish = () => {
          response.off("finish", complete);
          response.off("close", incomplete);
          resolve(!response.destroyed || response.writableFinished);
        };
        const complete = () => finish();
        const incomplete = () => finish();
        response.once("finish", complete);
        response.once("close", incomplete);
        response.end(bytes);
      });
    const json = async (status, body, outcome) => {
      let bytes = Buffer.from(JSON.stringify(body));
      if (bytes.length > config.limits.maxResponseBytes) {
        status = 500;
        outcome = "fixture_response_limit";
        bytes = Buffer.from(
          JSON.stringify(errorBody(config.protocol, outcome)),
        );
        if (bytes.length > config.limits.maxResponseBytes) {
          record.outcome = outcome;
          response.destroy();
          return;
        }
      }
      record.status = status;
      record.outcome = outcome;
      record.response_bytes = bytes.length;
      response.writeHead(status, {
        "Content-Type": "application/json",
        "Cache-Control": "no-store",
        Connection: "close",
        "X-Orivelle-Request-ID": record.request_id,
      });
      if (!(await ended(bytes))) record.outcome = "client_cancelled";
    };
    const reject = (status, code) =>
      json(status, errorBody(config.protocol, code), code);
    try {
      if (activeBefore >= config.limits.maxConcurrent) {
        await reject(429, "fixture_concurrency_limit");
        return;
      }
      if (request.headers.host !== new URL(origin).host) {
        await reject(400, "fixture_invalid_host");
        return;
      }
      if (!authorized(request, token, config.protocol)) {
        await reject(401, "fixture_authentication_failed");
        return;
      }
      const routes = {
        "GET /v1/models": "models",
        ["POST /v1/" + config.protocol]: "inference",
        ...(config.protocol === "messages"
          ? { "POST /v1/messages/count_tokens": "count_tokens" }
          : {}),
      };
      record.route = routes[request.method + " " + request.url] ?? "unknown";
      if (record.route === "unknown") {
        await reject(404, "fixture_route_not_found");
        return;
      }
      if (record.route === "models") {
        await json(
          200,
          {
            object: "list",
            data: [...models.values()].map((model) => ({
              id: model.id,
              object: "model",
              owned_by: "orbit-fixture",
            })),
          },
          "models",
        );
        return;
      }
      if (
        request.headers["content-type"]?.split(";")[0].trim() !==
          "application/json" ||
        request.headers["content-encoding"] !== undefined
      ) {
        await reject(415, "fixture_json_required");
        return;
      }
      const parts = [];
      let length = 0;
      for await (const part of request) {
        length += part.length;
        if (length > config.limits.maxRequestBytes) {
          await reject(413, "fixture_request_limit");
          return;
        }
        parts.push(part);
      }
      let body;
      try {
        body = JSON.parse(Buffer.concat(parts).toString("utf8"));
      } catch {
        await reject(400, "fixture_invalid_json");
        return;
      }
      if (
        !body ||
        typeof body !== "object" ||
        Array.isArray(body) ||
        (body.stream !== undefined && typeof body.stream !== "boolean") ||
        (config.protocol === "responses"
          ? !(typeof body.input === "string" || Array.isArray(body.input))
          : !Array.isArray(body.messages) || body.messages.length === 0)
      ) {
        await reject(400, "fixture_invalid_request");
        return;
      }
      const model = models.get(body.model);
      if (!model) {
        await reject(422, "fixture_model_not_allowed");
        return;
      }
      record.model_id = model.id;
      record.role = model.role;
      if (record.route === "count_tokens") {
        await json(200, { input_tokens: 1 }, "count_tokens");
        return;
      }
      const results = resultsFor(config.protocol, body);
      const pending = pendingTools.get(model.id);
      const wantsTools = config.scenario !== "text";
      let calls = null;
      if (results.length) {
        if (!wantsTools) throw fault("fixture_tool_state_conflict");
        validateContinuation(config.protocol, body, pending, results);
        pendingTools.delete(model.id);
      } else if (wantsTools) {
        const declared =
          Array.isArray(body.tools) &&
          body.tools.some(
            (tool) =>
              tool?.name === fixture.tool.name &&
              (config.protocol === "messages" || tool.type === "function"),
          );
        if (!declared || pending) throw fault("fixture_tool_state_conflict");
        calls = Array.from(
          {
            length: config.scenario === "parallel-tools" ? 2 : 1,
          },
          (_, index) => ({
            id: "call_fixture_" + randomUUID(),
            itemId: "fc_fixture_" + randomUUID(),
            name: fixture.tool.name,
            arguments: JSON.stringify({
              ...fixture.tool.arguments,
              slot: index,
            }),
          }),
        );
      }
      const responseId =
        (config.protocol === "responses" ? "resp_" : "msg_") +
        "fixture_" +
        randomUUID();
      const output = outputFor(config.protocol, model, calls);
      const payload =
        config.protocol === "responses"
          ? responseObject(model, output, responseId)
          : messageObject(model, output, responseId);
      if (calls) pendingTools.set(model.id, { responseId, calls });
      if (!body.stream) {
        await json(200, payload, calls ? "tool_call" : "completed");
      } else {
        const entries =
          config.protocol === "responses"
            ? responseEvents(payload)
            : messageEvents(payload);
        const chunks = entries.map((entry) =>
          Buffer.from(
            "event: " +
              entry.name +
              "\ndata: " +
              JSON.stringify(entry.data) +
              "\n\n",
          ),
        );
        const total = chunks.reduce((sum, part) => sum + part.length, 0);
        if (
          entries.length > config.limits.maxEvents ||
          total > config.limits.maxResponseBytes
        ) {
          pendingTools.delete(model.id);
          await reject(500, "fixture_response_limit");
          return;
        }
        record.status = 200;
        record.outcome = calls ? "tool_call" : "completed";
        response.writeHead(200, {
          "Content-Type": "text/event-stream",
          "Cache-Control": "no-store",
          Connection: "close",
          "X-Orivelle-Request-ID": record.request_id,
        });
        for (const [index, chunk] of chunks.entries()) {
          if (controller.signal.aborted || response.destroyed) break;
          if (!response.write(chunk)) {
            await new Promise((resolve) => {
              const finish = () => {
                response.off("drain", finish);
                response.off("close", finish);
                controller.signal.removeEventListener("abort", finish);
                resolve();
              };
              response.once("drain", finish);
              response.once("close", finish);
              controller.signal.addEventListener("abort", finish, {
                once: true,
              });
            });
          }
          record.event_count++;
          record.response_bytes += chunk.length;
          if (
            ["response.completed", "message_stop"].includes(entries[index].name)
          )
            record.success_terminal_count++;
          await new Promise((resolve) => setImmediate(resolve));
        }
        if (response.destroyed || controller.signal.aborted) {
          record.outcome = "client_cancelled";
          pendingTools.delete(model.id);
        } else {
          await ended();
        }
      }
    } catch (error) {
      if (!response.destroyed && !response.headersSent)
        await reject(
          409,
          error?.code === "fixture_tool_state_conflict"
            ? "fixture_tool_state_conflict"
            : "fixture_invalid_request",
        );
      else if (record.outcome === "pending")
        record.outcome = "client_cancelled";
    } finally {
      clearTimeout(timer);
      requests.delete(response);
      if (record.outcome === "pending") record.outcome = "client_cancelled";
      if (records.length >= config.limits.maxRequests && requests.size === 0)
        void close("request_limit");
    }
  }
  await new Promise((resolve, reject) => {
    const unavailable = (error) => {
      const failure = fault("fixture_listen_unavailable");
      if (["EPERM", "EACCES", "EADDRINUSE"].includes(error.code))
        failure.listen_code = error.code;
      reject(failure);
    };
    server.once("error", unavailable);
    server.listen(0, "127.0.0.1", () => {
      server.off("error", unavailable);
      resolve();
    });
  });
  server.on("error", () => {
    void close("server_error");
  });
  origin = "http://127.0.0.1:" + server.address().port;
  ttlTimer = setTimeout(() => {
    void close("ttl");
  }, config.ttlMs);
  return Object.freeze({
    origin,
    baseUrl: origin + "/v1",
    fixtureIdentity: identity,
    fixtureToken: token,
    snapshot,
    close: () => close(),
  });
}
