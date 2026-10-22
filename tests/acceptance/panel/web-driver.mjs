import https from "node:https";
import http from "node:http";
import tls from "node:tls";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HOST = "orbit-e2e.test";
const CALLBACK_PATH = "/orivelle-orbit/callback";
const ULID = /^[0-9A-HJKMNP-TV-Z]{26}$/i;
const SAFE_CODES = new Set([
  "invalid_input",
  "panel_transport_failed",
  "panel_response_invalid",
  "login_page_unavailable",
  "login_csrf_not_rejected",
  "login_failed",
  "consent_unavailable",
  "consent_csrf_not_rejected",
  "consent_failed",
  "callback_invalid",
  "callback_failed",
]);

function fail(code) {
  throw new Error(code);
}

function emptyResult() {
  return {
    ok: false,
    code: "invalid_input",
    loginCsrfRejected: false,
    consentCsrfRejected: false,
    consentApproved: false,
    callbackDelivered: false,
  };
}

function validateInput(input) {
  const keys =
    input && typeof input === "object" && !Array.isArray(input)
      ? Object.keys(input).sort()
      : [];
  if (
    keys.join(",") !==
      "authorizationUrl,caPem,email,expectedCallbackUri,password,port" ||
    typeof input.caPem !== "string" ||
    input.caPem.length > 65_536 ||
    !input.caPem.includes("-----BEGIN CERTIFICATE-----") ||
    !Number.isInteger(input.port) ||
    input.port < 1 ||
    input.port > 65_535 ||
    typeof input.email !== "string" ||
    !/^[a-z0-9._+-]{1,100}@example\.test$/.test(input.email) ||
    typeof input.password !== "string" ||
    !/^[\x21-\x7e]{24,256}$/.test(input.password) ||
    typeof input.expectedCallbackUri !== "string" ||
    typeof input.authorizationUrl !== "string" ||
    input.authorizationUrl.length > 2048
  ) {
    fail("invalid_input");
  }
  const origin = new URL(`https://${HOST}:${input.port}`).origin;
  const authorization = sameOriginUrl(input.authorizationUrl, origin);
  if (
    authorization.pathname !== "/orbit/authorize" ||
    authorization.searchParams.size !== 1 ||
    !ULID.test(authorization.searchParams.get("request") ?? "")
  ) {
    fail("invalid_input");
  }
  const expectedCallback = expectedCallbackTarget(input.expectedCallbackUri);
  return { ...input, origin, authorization, expectedCallback };
}

function expectedCallbackTarget(value) {
  const match =
    /^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})\/orivelle-orbit\/callback$/.exec(
      value,
    );
  if (!match || match[0] !== value || Number(match[1]) > 65_535)
    fail("invalid_input");
  return { url: new URL(value), port: Number(match[1]) };
}

function sameOriginUrl(value, origin, base = origin) {
  let url;
  try {
    url = new URL(value, base);
  } catch {
    fail("panel_response_invalid");
  }
  if (
    url.origin !== origin ||
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.hash
  ) {
    fail("panel_response_invalid");
  }
  return url;
}

class CookieJar {
  #cookies = new Map();

  store(headers, url) {
    for (const raw of headers ?? []) {
      if (
        typeof raw !== "string" ||
        raw.length > 8192 ||
        /[^\x20-\x7e]/.test(raw)
      ) {
        fail("panel_response_invalid");
      }
      const [pair, ...attributes] = raw.split(";");
      const equals = pair.indexOf("=");
      const name = pair.slice(0, equals).trim();
      const value = pair.slice(equals + 1).trim();
      if (equals < 1 || !/^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/.test(name)) {
        fail("panel_response_invalid");
      }
      let cookiePath =
        url.pathname.slice(0, url.pathname.lastIndexOf("/") + 1) || "/";
      let expires = null;
      for (const attribute of attributes) {
        const [key, ...parts] = attribute.trim().split("=");
        const content = parts.join("=");
        if (
          key.toLowerCase() === "domain" &&
          content.toLowerCase().replace(/^\./, "") !== HOST
        ) {
          fail("panel_response_invalid");
        }
        if (key.toLowerCase() === "path") {
          if (!content.startsWith("/")) fail("panel_response_invalid");
          cookiePath = content;
        }
        if (key.toLowerCase() === "max-age") {
          if (!/^-?\d+$/.test(content)) fail("panel_response_invalid");
          expires = Date.now() + Number(content) * 1000;
        } else if (key.toLowerCase() === "expires" && expires === null) {
          expires = Date.parse(content);
          if (!Number.isFinite(expires)) fail("panel_response_invalid");
        }
      }
      const id = `${name}\n${cookiePath}`;
      if (expires !== null && expires <= Date.now()) this.#cookies.delete(id);
      else this.#cookies.set(id, { name, value, path: cookiePath, expires });
      if (this.#cookies.size > 16) fail("panel_response_invalid");
    }
  }

  #matching(url) {
    return [...this.#cookies.values()]
      .filter(
        (cookie) =>
          (cookie.expires === null || cookie.expires > Date.now()) &&
          (url.pathname === cookie.path ||
            (url.pathname.startsWith(cookie.path) &&
              (cookie.path.endsWith("/") ||
                url.pathname[cookie.path.length] === "/"))),
      )
      .sort((a, b) => b.path.length - a.path.length);
  }

  header(url) {
    const header = this.#matching(url)
      .map(({ name, value }) => `${name}=${value}`)
      .join("; ");
    if (header.length > 16_384) fail("panel_response_invalid");
    return header;
  }

  csrf(url) {
    const tokens = this.#matching(url).filter(
      ({ name }) => name === "XSRF-TOKEN",
    );
    if (tokens.length !== 1) fail("panel_response_invalid");
    let token;
    try {
      token = decodeURIComponent(tokens[0].value);
    } catch {
      fail("panel_response_invalid");
    }
    if (!/^[\x21-\x7e]{1,8192}$/.test(token)) fail("panel_response_invalid");
    return token;
  }

  clear() {
    this.#cookies.clear();
  }
}

function request(
  client,
  url,
  agent,
  headers,
  body,
  errorCode,
  limit = 2_097_152,
) {
  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (error, result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      if (error) reject(new Error(errorCode));
      else resolve(result);
    };
    const req = client.request(
      url,
      {
        method: body === null ? "GET" : "POST",
        agent,
        headers,
        maxHeaderSize: 32_768,
      },
      (res) => {
        let bytes = 0;
        res.on("data", (chunk) => {
          bytes += chunk.length;
          if (bytes > limit) {
            finish(true);
            res.destroy();
            req.destroy();
          }
        });
        res.on("end", () =>
          finish(false, { status: res.statusCode, headers: res.headers }),
        );
        res.on("error", () => finish(true));
        res.on("aborted", () => finish(true));
      },
    );
    const timer = setTimeout(() => {
      finish(true);
      req.destroy();
    }, 10_000);
    req.on("error", () => finish(true));
    if (body !== null) req.write(body);
    req.end();
  });
}

function callbackUrl(location, origin, expected) {
  if (
    typeof location !== "string" ||
    location.length > 2048 ||
    !/^http:\/\/127\.0\.0\.1:([1-9][0-9]{0,4})\/orivelle-orbit\/callback\?/.test(
      location,
    )
  ) {
    fail("callback_invalid");
  }
  const url = new URL(location);
  const port = Number(location.match(/^http:\/\/127\.0\.0\.1:([0-9]+)/)[1]);
  const entries = [...url.searchParams.entries()];
  const keys = entries
    .map(([key]) => key)
    .sort()
    .join(",");
  if (
    port > 65_535 ||
    url.pathname !== CALLBACK_PATH ||
    url.username ||
    url.password ||
    url.hash ||
    url.origin !== expected.url.origin ||
    url.pathname !== expected.url.pathname ||
    port !== expected.port ||
    entries.length !== 3 ||
    keys !== "code,iss,state" ||
    !/^[A-Za-z0-9_-]{43}$/.test(url.searchParams.get("code") ?? "") ||
    !/^[A-Za-z0-9_-]{32,128}$/.test(url.searchParams.get("state") ?? "") ||
    url.searchParams.get("iss") !== origin
  ) {
    fail("callback_invalid");
  }
  return { url, port };
}

/** Real Panel Web requests only; importing this module does not start a flow. */
export async function approveOrbitAuthorization(input) {
  const result = emptyResult();
  const jar = new CookieJar();
  let panelAgent;
  let callbackAgent;
  try {
    const options = validateInput(input);
    panelAgent = new https.Agent({ keepAlive: false, maxSockets: 1 });
    // Fixed transport remap with an explicit CA and original TLS hostname verification.
    panelAgent.createConnection = () =>
      tls.connect({
        host: "127.0.0.1",
        port: options.port,
        servername: HOST,
        ca: options.caPem,
        rejectUnauthorized: true,
        minVersion: "TLSv1.2",
        checkServerIdentity: (_host, cert) =>
          tls.checkServerIdentity(HOST, cert),
      });
    const panelRequest = async (url, fields = null, csrf = false) => {
      const target = sameOriginUrl(url, options.origin);
      const body =
        fields === null ? null : new URLSearchParams(fields).toString();
      const headers = { Accept: "text/html", Cookie: jar.header(target) };
      if (body !== null) {
        headers["Content-Type"] = "application/x-www-form-urlencoded";
        headers["Content-Length"] = Buffer.byteLength(body);
        if (csrf) headers["X-XSRF-TOKEN"] = jar.csrf(target);
      }
      // No Sec-Fetch-Site: both negative and positive POSTs exercise CSRF token matching.
      const response = await request(
        https,
        target,
        panelAgent,
        headers,
        body,
        "panel_transport_failed",
      );
      jar.store(response.headers["set-cookie"], target);
      return response;
    };
    const getFollowing = async (initial) => {
      let url = sameOriginUrl(initial, options.origin);
      for (let count = 0; count < 6; count += 1) {
        const response = await panelRequest(url);
        if (![301, 302, 303, 307, 308].includes(response.status))
          return { ...response, url };
        if (typeof response.headers.location !== "string")
          fail("panel_response_invalid");
        url = sameOriginUrl(response.headers.location, options.origin, url);
      }
      fail("panel_response_invalid");
    };

    const initial = await getFollowing(options.authorization);
    if (initial.status !== 200 || initial.url.pathname !== "/login")
      fail("login_page_unavailable");
    const loginUrl = new URL("/login", options.origin);
    const credentials = { email: options.email, password: options.password };
    const rejectedLogin = await panelRequest(loginUrl, credentials);
    if (rejectedLogin.status !== 419) fail("login_csrf_not_rejected");
    result.loginCsrfRejected = true;
    if ((await getFollowing(loginUrl)).status !== 200)
      fail("login_page_unavailable");
    const login = await panelRequest(loginUrl, credentials, true);
    if (
      ![302, 303].includes(login.status) ||
      typeof login.headers.location !== "string"
    )
      fail("login_failed");
    await getFollowing(
      sameOriginUrl(login.headers.location, options.origin, loginUrl),
    );

    const consent = await getFollowing(options.authorization);
    if (
      consent.status !== 200 ||
      consent.url.href !== options.authorization.href
    )
      fail("consent_unavailable");
    const consentUrl = new URL("/orbit/authorize", options.origin);
    const approval = {
      authorization_request_id:
        options.authorization.searchParams.get("request"),
      approve: "1",
    };
    const rejectedConsent = await panelRequest(consentUrl, approval);
    if (rejectedConsent.status !== 419) fail("consent_csrf_not_rejected");
    result.consentCsrfRejected = true;
    const refreshed = await getFollowing(options.authorization);
    if (
      refreshed.status !== 200 ||
      refreshed.url.href !== options.authorization.href
    )
      fail("consent_unavailable");
    const approved = await panelRequest(consentUrl, approval, true);
    if (approved.status !== 303) fail("consent_failed");
    const callback = callbackUrl(
      approved.headers.location,
      options.origin,
      options.expectedCallback,
    );
    result.consentApproved = true;
    callbackAgent = new http.Agent({ keepAlive: false, maxSockets: 1 });
    callbackAgent.createConnection = () =>
      net.connect({ host: "127.0.0.1", port: callback.port });
    // A new agent and empty headers ensure no Panel cookies or authentication cross this boundary.
    const delivered = await request(
      http,
      callback.url,
      callbackAgent,
      {},
      null,
      "callback_failed",
      65_536,
    );
    if (delivered.status !== 200) fail("callback_failed");
    result.callbackDelivered = true;
    result.ok = true;
    result.code = "ok";
  } catch (error) {
    result.code = SAFE_CODES.has(error?.message)
      ? error.message
      : "driver_failed";
  } finally {
    panelAgent?.destroy();
    callbackAgent?.destroy();
    jar.clear();
  }
  return result;
}

async function main() {
  let result = emptyResult();
  try {
    const chunks = [];
    let bytes = 0;
    for await (const chunk of process.stdin) {
      bytes += chunk.length;
      if (bytes > 98_304) fail("invalid_input");
      chunks.push(chunk);
    }
    result = await approveOrbitAuthorization(
      JSON.parse(Buffer.concat(chunks).toString("utf8")),
    );
  } catch {
    // Never print an exception, URL, response, password, cookie, or authorization material.
  }
  process.stdout.write(`${JSON.stringify(result)}\n`);
  process.exitCode = result.ok ? 0 : 1;
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  await main();
}
