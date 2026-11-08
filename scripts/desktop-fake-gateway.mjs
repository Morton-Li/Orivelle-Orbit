import { randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { readFile } from "node:fs/promises";
import { createServer } from "node:http";
import { isDeepStrictEqual } from "node:util";

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
  "auth-error",
  "rate-limit",
  "upstream-error",
  "stream-error",
  "malformed-stream",
  "disconnect",
  "stall",
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

function errorBody(protocol, code, status = 400) {
  const type =
    status === 401
      ? "authentication_error"
      : status === 429
        ? "rate_limit_error"
        : status === 529
          ? "overloaded_error"
          : status >= 500
            ? protocol === "messages"
              ? "api_error"
              : "server_error"
            : "invalid_request_error";
  return protocol === "messages"
    ? {
        type: "error",
        error: {
          type,
          message: code,
        },
      }
    : { error: { type, code, message: code } };
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
        logprobs: [],
      }),
      event("response.output_text.done", {
        item_id: item.id,
        output_index: 0,
        content_index: 0,
        text: part.text,
        logprobs: [],
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
    !pending.ready ||
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
      (body.previous_response_id === undefined && replayed.length === 0) ||
      (replayed.length > 0 &&
        (replayed.length !== pending.calls.length ||
          new Set(replayed.map((item) => item.call_id)).size !==
            replayed.length ||
          replayed.some(
            (item) =>
              !pending.calls.some(
                (call) =>
                  call.id === item.call_id &&
                  call.name === item.name &&
                  call.arguments === item.arguments &&
                  body.input.indexOf(item) <
                    body.input.findIndex(
                      (candidate) =>
                        candidate?.type === "function_call_output" &&
                        candidate.call_id === call.id,
                    ),
              ),
          )))
    )
      throw fault("fixture_tool_state_conflict");
  } else {
    const turns = body.messages.map((message, index) => ({
      message,
      index,
      calls: (Array.isArray(message?.content) ? message.content : []).filter(
        (block) => block?.type === "tool_use",
      ),
      results: (Array.isArray(message?.content) ? message.content : []).filter(
        (block) => block?.type === "tool_result",
      ),
    }));
    const callTurns = turns.filter((turn) => turn.calls.length > 0);
    const resultTurns = turns.filter((turn) => turn.results.length > 0);
    const replayed = callTurns.flatMap((turn) => turn.calls);
    if (
      callTurns.length !== 1 ||
      resultTurns.length !== 1 ||
      callTurns[0].message.role !== "assistant" ||
      resultTurns[0].message.role !== "user" ||
      callTurns[0].index + 1 !== resultTurns[0].index ||
      resultTurns[0].index !== body.messages.length - 1 ||
      resultTurns[0].message.content.some((block, index) =>
        index < pending.calls.length
          ? block?.type !== "tool_result"
          : block?.type !== "text" || typeof block.text !== "string",
      ) ||
      replayed.length !== pending.calls.length ||
      new Set(replayed.map((block) => block.id)).size !== replayed.length ||
      replayed.some(
        (block) =>
          !pending.calls.some(
            (call) =>
              call.id === block.id &&
              call.name === block.name &&
              isDeepStrictEqual(block.input, JSON.parse(call.arguments)),
          ),
      )
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
  let acceptedConnections = 0;
  let pipelineRejected = 0;
  function dispatch(request, response) {
    const task = handle(request, response);
    tasks.add(task);
    void task.finally(() => tasks.delete(task));
  }
  const server = createServer({ maxHeaderSize: 16 * 1024 }, dispatch);
  server.on("checkContinue", dispatch);
  server.on("checkExpectation", dispatch);
  server.on("dropRequest", (_, socket) => {
    pipelineRejected = Math.min(
      pipelineRejected + 1,
      config.limits.maxRequests,
    );
    socket.destroy();
  });
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
      new Promise((resolve) => {
        socket.once("close", () => {
          sockets.delete(socket);
          socketClosures.delete(socket);
          resolve();
          if (
            acceptedConnections >= config.limits.maxRequests &&
            sockets.size === 0
          )
            void close("connection_limit");
        });
      }),
    );
    if (closing || acceptedConnections >= config.limits.maxRequests) {
      socket.destroy();
      return;
    }
    acceptedConnections++;
    if (acceptedConnections === config.limits.maxRequests) stopAccepting();
  });
  server.on("clientError", (_, socket) => {
    parserRejected = Math.min(parserRejected + 1, config.limits.maxRequests);
    socket.destroy();
    if (parserRejected >= config.limits.maxRequests) void close("parser_limit");
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
      ttl_ms: config.ttlMs,
      real_desktop_verified: false,
      complete_desktop_catalog: false,
      limits: config.limits,
      request_count: records.length,
      active_requests: [...requests.values()].filter((state) => state.admitted)
        .length,
      pending_responses: requests.size,
      active_timers: requests.size + (ttlTimer === undefined ? 0 : 1),
      open_sockets: sockets.size,
      live_socket_promises: socketClosures.size,
      accepted_connections: acceptedConnections,
      pending_tool_roles: pendingTools.size,
      budget_rejected: budgetRejected,
      parser_rejected: parserRejected,
      pipeline_rejected: pipelineRejected,
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
    let issuedPending;
    let issuedModel;
    const activeBefore = [...requests.values()].filter(
      (item) => item.admitted,
    ).length;
    const state = {
      request,
      response,
      controller,
      record,
      admitted: activeBefore < config.limits.maxConcurrent,
    };
    requests.set(response, state);
    response.once("close", () => {
      if (!response.writableFinished) {
        if (
          ["pending", "completed", "tool_call", "fixture_stall"].includes(
            record.outcome,
          )
        )
          record.outcome = "client_cancelled";
        controller.abort();
      }
    });
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
          JSON.stringify(errorBody(config.protocol, outcome, 500)),
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
      json(status, errorBody(config.protocol, code, status), code);
    const waitForAbort = () =>
      new Promise((resolve) => {
        if (controller.signal.aborted) resolve();
        else
          controller.signal.addEventListener("abort", resolve, { once: true });
      });
    const stream = async (
      entries,
      outcome,
      { malformed = false, stall = false } = {},
    ) => {
      const chunks = entries.map((entry, index) =>
        Buffer.from(
          "event: " +
            entry.name +
            "\ndata: " +
            (malformed && index === entries.length - 1
              ? "{not-json"
              : JSON.stringify(entry.data)) +
            "\n\n",
        ),
      );
      const total = chunks.reduce((sum, part) => sum + part.length, 0);
      if (
        entries.length > config.limits.maxEvents ||
        total > config.limits.maxResponseBytes
      ) {
        await reject(500, "fixture_response_limit");
        return;
      }
      record.status = 200;
      record.outcome = outcome;
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
            controller.signal.addEventListener("abort", finish, { once: true });
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
      if (!controller.signal.aborted && !response.destroyed) {
        if (stall) await waitForAbort();
        else if (!(await ended())) record.outcome = "client_cancelled";
      }
    };
    try {
      if (request.headers.expect !== undefined) {
        await reject(417, "fixture_expectation_not_supported");
        return;
      }
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
      if (config.scenario === "auth-error") {
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
        body = JSON.parse(
          new TextDecoder("utf-8", { fatal: true }).decode(
            Buffer.concat(parts),
          ),
        );
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
      if (config.scenario === "rate-limit") {
        await reject(429, "fixture_rate_limit");
        return;
      }
      if (config.scenario === "upstream-error") {
        await reject(502, "fixture_upstream_error");
        return;
      }
      const results = resultsFor(config.protocol, body);
      const pending = pendingTools.get(model.id);
      const wantsTools =
        ["single-tool", "parallel-tools"].includes(config.scenario) ||
        (config.scenario === "stall" &&
          Array.isArray(body.tools) &&
          body.tools.length > 0);
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
      if (calls) {
        issuedPending = { responseId, calls, ready: false };
        issuedModel = model.id;
        pendingTools.set(model.id, issuedPending);
      }
      if (
        ["stream-error", "malformed-stream", "disconnect", "stall"].includes(
          config.scenario,
        )
      ) {
        if (!body.stream && config.scenario !== "stall") {
          await reject(400, "fixture_stream_required");
          return;
        }
        if (config.scenario === "stall" && !body.stream) {
          const prefix = Buffer.from('{"fixture_incomplete":');
          if (prefix.length > config.limits.maxResponseBytes) {
            await reject(500, "fixture_response_limit");
            return;
          }
          record.status = 200;
          record.outcome = "fixture_stall";
          record.response_bytes = prefix.length;
          response.writeHead(200, {
            "Content-Type": "application/json",
            "Cache-Control": "no-store",
            Connection: "close",
          });
          response.write(prefix);
          await waitForAbort();
          return;
        }
        const full =
          config.protocol === "responses"
            ? responseEvents(payload)
            : messageEvents(payload);
        let limited = full.filter(
          (entry) =>
            !["response.completed", "message_delta", "message_stop"].includes(
              entry.name,
            ),
        );
        if (config.scenario === "stream-error") {
          limited = [
            full[0],
            config.protocol === "responses"
              ? event("response.failed", {
                  sequence_number: 1,
                  response: {
                    ...payload,
                    status: "failed",
                    output: [],
                    error: {
                      code: "server_error",
                      message: "fixture_stream_error",
                    },
                  },
                })
              : event("error", {
                  error: {
                    type: "overloaded_error",
                    message: "fixture_stream_error",
                  },
                }),
          ];
        } else if (config.scenario === "malformed-stream") {
          limited = [full[0], event("fixture_malformed", {})];
        } else if (config.scenario === "disconnect") {
          limited = limited.slice(0, 2);
        }
        await stream(
          limited,
          "fixture_" + config.scenario.replaceAll("-", "_"),
          {
            malformed: config.scenario === "malformed-stream",
            stall: config.scenario === "stall",
          },
        );
        return;
      }
      if (!body.stream) {
        await json(200, payload, calls ? "tool_call" : "completed");
      } else {
        const entries =
          config.protocol === "responses"
            ? responseEvents(payload)
            : messageEvents(payload);
        await stream(entries, calls ? "tool_call" : "completed");
      }
      if (
        issuedPending &&
        record.outcome === "tool_call" &&
        response.writableFinished &&
        !controller.signal.aborted
      )
        issuedPending.ready = true;
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
      if (
        issuedPending &&
        !issuedPending.ready &&
        pendingTools.get(issuedModel) === issuedPending
      )
        pendingTools.delete(issuedModel);
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
