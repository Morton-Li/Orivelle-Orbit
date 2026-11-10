//! Claude-only loopback transport. This is not a configurable HTTP proxy.

use crate::{
    claude_mapping::{
        ForwardOutcome, ForwardPermit, LocalRoute, MappingAuthority, MappingServer,
        ProtocolHeaders, allowed_alias,
    },
    error::{PublicError, Result, fail},
    network::{Http, MappingResponse},
};
use futures_util::{StreamExt, stream};
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Body, Bytes, Frame, Incoming},
    header::{self, HeaderMap, HeaderValue},
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::{
    Deserialize,
    de::{MapAccess, Visitor},
};
use serde_json::value::RawValue;
use std::{
    collections::HashSet,
    convert::Infallible,
    net::{Ipv4Addr, TcpListener},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    net::TcpListener as AsyncListener,
    sync::{OwnedSemaphorePermit, Semaphore, watch},
    time::Instant,
};
use zeroize::{Zeroize, Zeroizing};

const HEADER_LIMIT: usize = 32 * 1024;
const BODY_LIMIT: usize = 8 * 1024 * 1024;
const OUTPUT_LIMIT: u64 = 64 * 1024 * 1024;
const HEADER_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const IDLE_TIMEOUT: Duration = Duration::from_secs(120);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
type LocalBody = UnsyncBoxBody<Bytes, PublicError>;

struct ServerState {
    route: LocalRoute,
    authority: Arc<dyn MappingAuthority>,
    http: Http,
    running: AtomicBool,
    inflight: AtomicUsize,
    upstream: Arc<Semaphore>,
    connections: Arc<Semaphore>,
    stop: watch::Sender<bool>,
}

struct ServerHandle(Arc<ServerState>);
impl MappingServer for ServerHandle {
    fn stop(&self) {
        self.0.running.store(false, Ordering::SeqCst);
        let _ = self.0.stop.send(true);
    }
    fn is_running(&self) -> bool {
        self.0.running.load(Ordering::SeqCst)
    }
    fn inflight(&self) -> usize {
        self.0.inflight.load(Ordering::SeqCst)
    }
}
impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
struct RunningGuard(Arc<ServerState>);
impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.running.store(false, Ordering::SeqCst);
        let _ = self.0.stop.send(true);
    }
}

/// Start only inside the application's existing Tokio runtime. Domain code has
/// already reserved this exact route; no ephemeral replacement port is chosen.
pub(crate) fn start(
    listener: TcpListener,
    route: LocalRoute,
    authority: Arc<dyn MappingAuthority>,
    http: Http,
) -> Result<Box<dyn MappingServer>> {
    Ok(Box::new(start_local(listener, route, authority, http)?))
}

fn start_local(
    listener: TcpListener,
    route: LocalRoute,
    authority: Arc<dyn MappingAuthority>,
    http: Http,
) -> Result<ServerHandle> {
    let runtime =
        tokio::runtime::Handle::try_current().map_err(|_| fail("claude_mapping_unavailable"))?;
    let binding = uuid::Uuid::parse_str(&route.binding_id)
        .map_err(|_| fail("claude_mapping_route_invalid"))?;
    let address = listener
        .local_addr()
        .map_err(|_| fail("claude_mapping_unavailable"))?;
    if route.binding_id != binding.to_string()
        || route.base_path != format!("/orivelle-orbit/claude/{}", route.binding_id)
        || route.port == 0
        || address != std::net::SocketAddr::from((Ipv4Addr::LOCALHOST, route.port))
        || route.aliases.is_empty()
    {
        return Err(fail("claude_mapping_route_invalid"));
    }
    listener
        .set_nonblocking(true)
        .map_err(|_| fail("claude_mapping_unavailable"))?;
    let listener =
        AsyncListener::from_std(listener).map_err(|_| fail("claude_mapping_unavailable"))?;
    let (stop, mut stopped) = watch::channel(false);
    let state = Arc::new(ServerState {
        route,
        authority,
        http,
        running: AtomicBool::new(true),
        inflight: AtomicUsize::new(0),
        upstream: Arc::new(Semaphore::new(2)),
        connections: Arc::new(Semaphore::new(16)),
        stop,
    });
    let serving = state.clone();
    runtime.spawn(async move {
        let _running = RunningGuard(serving.clone());
        loop {
            let accepted = tokio::select! {
                biased;
                _ = wait_stop(&mut stopped) => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((socket, peer)) = accepted else { break };
            if peer.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) {
                continue;
            }
            let Ok(slot) = serving.connections.clone().try_acquire_owned() else {
                continue;
            };
            let state = serving.clone();
            let mut stopped = state.stop.subscribe();
            tokio::spawn(async move {
                let slot = Arc::new(slot);
                let service_state = state.clone();
                let service = service_fn(move |request| {
                    let state = service_state.clone();
                    let slot = slot.clone();
                    async move { Ok::<_, Infallible>(handle(request, state, slot).await) }
                });
                let mut builder = http1::Builder::new();
                builder
                    .keep_alive(false)
                    .max_buf_size(HEADER_LIMIT)
                    .max_headers(128)
                    .timer(TokioTimer::new())
                    .header_read_timeout(HEADER_TIMEOUT);
                let connection = builder.serve_connection(TokioIo::new(socket), service);
                tokio::select! {
                    biased;
                    _ = wait_stop(&mut stopped) => {},
                    _ = tokio::time::sleep(TOTAL_TIMEOUT) => {},
                    _ = connection => {},
                }
            });
        }
    });
    Ok(ServerHandle(state))
}

async fn wait_stop(stopped: &mut watch::Receiver<bool>) {
    if *stopped.borrow() {
        return;
    }
    while stopped.changed().await.is_ok() {
        if *stopped.borrow() {
            return;
        }
    }
}

enum RouteKind {
    Models,
    Messages { beta: bool },
}
fn classify(request: &Request<Incoming>, route: &LocalRoute) -> Result<RouteKind> {
    let headers = request.headers();
    let host = format!("127.0.0.1:{}", route.port);
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || headers.get_all(header::HOST).iter().count() != 1
        || headers
            .get(header::HOST)
            .and_then(|value| value.to_str().ok())
            != Some(host.as_str())
        || headers.contains_key(header::ORIGIN)
        || headers.contains_key(header::REFERER)
        || headers.contains_key(header::UPGRADE)
        || headers
            .keys()
            .any(|name| name.as_str().starts_with("sec-fetch-"))
        || headers.get_all(header::CONNECTION).iter().any(|value| {
            value.to_str().map_or(true, |value| {
                value
                    .split(',')
                    .any(|v| v.trim().eq_ignore_ascii_case("upgrade"))
            })
        })
        || headers
            .iter()
            .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
            .sum::<usize>()
            > HEADER_LIMIT
    {
        return Err(fail("claude_mapping_request_invalid"));
    }
    let target = request
        .uri()
        .path_and_query()
        .map(|target| target.as_str())
        .unwrap_or("");
    let messages = format!("{}/v1/messages", route.base_path);
    let models = format!("{}/v1/models", route.base_path);
    if request.method() == Method::POST && target == messages {
        Ok(RouteKind::Messages { beta: false })
    } else if request.method() == Method::POST && target == format!("{messages}?beta=true") {
        Ok(RouteKind::Messages { beta: true })
    } else if request.method() == Method::GET && target == models {
        Ok(RouteKind::Models)
    } else {
        Err(fail("claude_mapping_route_invalid"))
    }
}

fn bearer(headers: &HeaderMap) -> Result<Zeroizing<Vec<u8>>> {
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1 {
        return Err(fail("claude_mapping_unauthorized"));
    }
    let raw = headers.get(header::AUTHORIZATION).unwrap().as_bytes();
    let token = raw
        .strip_prefix(b"Bearer ")
        .ok_or_else(|| fail("claude_mapping_unauthorized"))?;
    if token.is_empty() || token.len() > 4096 || token.iter().any(|b| !b.is_ascii_graphic()) {
        return Err(fail("claude_mapping_unauthorized"));
    }
    Ok(Zeroizing::new(token.to_vec()))
}

fn protocol_headers(headers: &HeaderMap) -> Result<ProtocolHeaders> {
    fn one(headers: &HeaderMap, name: &'static str) -> Result<Option<String>> {
        if headers.get_all(name).iter().count() > 1 {
            return Err(fail("claude_mapping_headers_invalid"));
        }
        headers
            .get(name)
            .map(|value| {
                let value = value
                    .to_str()
                    .map_err(|_| fail("claude_mapping_headers_invalid"))?;
                if value.len() > 8192
                    || value.bytes().any(|b| !b.is_ascii() || b.is_ascii_control())
                {
                    return Err(fail("claude_mapping_headers_invalid"));
                }
                Ok(value.to_owned())
            })
            .transpose()
    }
    Ok(ProtocolHeaders {
        anthropic_version: one(headers, "anthropic-version")?,
        anthropic_beta: one(headers, "anthropic-beta")?,
    })
}

async fn handle(
    mut request: Request<Incoming>,
    state: Arc<ServerState>,
    connection: Arc<OwnedSemaphorePermit>,
) -> Response<LocalBody> {
    let kind = match classify(&request, &state.route) {
        Ok(kind) => kind,
        Err(error) => return local_error(StatusCode::BAD_REQUEST, &error.code),
    };
    if !state.running.load(Ordering::SeqCst) {
        return local_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "claude_mapping_unavailable",
        );
    }
    let token = match bearer(request.headers()) {
        Ok(token) => token,
        Err(error) => return local_error(StatusCode::UNAUTHORIZED, &error.code),
    };
    // Erase local authorization from the request before any forwarding work.
    request.headers_mut().remove(header::AUTHORIZATION);
    let authority = state.authority.clone();
    let native_connection = connection.clone();
    let access = match tokio::task::spawn_blocking(move || {
        (authority.authenticate(&token), native_connection)
    })
    .await
    {
        Ok((Ok(access), _connection)) if access.binding_id == state.route.binding_id => access,
        _ => return local_error(StatusCode::UNAUTHORIZED, "claude_mapping_unauthorized"),
    };
    if matches!(kind, RouteKind::Models) {
        if request.headers().contains_key(header::TRANSFER_ENCODING)
            || request
                .body()
                .size_hint()
                .upper()
                .is_some_and(|length| length > 0)
        {
            return local_error(StatusCode::BAD_REQUEST, "claude_mapping_request_invalid");
        }
        drop(access);
        return models_response(&state.route);
    }
    let RouteKind::Messages { beta } = kind else {
        unreachable!()
    };
    let protocol = match protocol_headers(request.headers()) {
        Ok(protocol) => protocol,
        Err(error) => return local_error(StatusCode::BAD_REQUEST, &error.code),
    };
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| {
            !value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        || request
            .headers()
            .get(header::CONTENT_ENCODING)
            .is_some_and(|value| value != "identity")
    {
        return local_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "claude_mapping_content_type_invalid",
        );
    }
    let body = match tokio::time::timeout(BODY_TIMEOUT, read_body(request.into_body())).await {
        Ok(Ok(body)) => body,
        Ok(Err(error)) => return local_error(StatusCode::PAYLOAD_TOO_LARGE, &error.code),
        Err(_) => return local_error(StatusCode::REQUEST_TIMEOUT, "claude_mapping_body_timeout"),
    };
    let (object, alias) = match RawObject::parse(&body, &state.route) {
        Ok(object) => object,
        Err(error) => return local_error(StatusCode::BAD_REQUEST, &error.code),
    };
    let Ok(slot) = state.upstream.clone().try_acquire_owned() else {
        return local_error(StatusCode::TOO_MANY_REQUESTS, "claude_mapping_busy");
    };
    let inflight = Arc::new(Inflight::new(state.clone(), slot));
    let work = NativeWork {
        _connection: connection,
        _inflight: inflight,
    };
    let authority = state.authority.clone();
    let native_work = work.clone();
    let permit = match tokio::task::spawn_blocking(move || {
        (authority.authorize_messages(access, alias), native_work)
    })
    .await
    {
        Ok((Ok(permit), _work)) => permit,
        _ => {
            return local_error(
                StatusCode::FORBIDDEN,
                "claude_mapping_authorization_changed",
            );
        }
    };
    let body = match object.rewrite(&permit.upstream_model_id) {
        Ok(body) => body,
        Err(error) => return local_error(StatusCode::BAD_REQUEST, &error.code),
    };
    let authority = state.authority.clone();
    let native_work = work.clone();
    let permit =
        match tokio::task::spawn_blocking(move || (authority.before_send(permit), native_work))
            .await
        {
            Ok((Ok(permit), _work)) => permit,
            _ => {
                return local_error(
                    StatusCode::FORBIDDEN,
                    "claude_mapping_authorization_changed",
                );
            }
        };
    let mut observation = Observation::new(permit, state.authority.clone(), work);
    if !state.running.load(Ordering::SeqCst) {
        observation.outcome = ForwardOutcome::NotSent;
        return local_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "claude_mapping_unavailable",
        );
    }
    let started = Instant::now();
    let response = state
        .http
        .claude_mapping_messages(observation.permit.as_mut().unwrap(), body, protocol, beta)
        .await;
    observation.permit.as_mut().unwrap().api_key.zeroize();
    match response {
        Ok(response) => streaming_response(response, observation, state.stop.subscribe(), started),
        Err(_) => local_error(StatusCode::BAD_GATEWAY, "result_unconfirmed"),
    }
}

async fn read_body(mut body: Incoming) -> Result<Vec<u8>> {
    if body
        .size_hint()
        .upper()
        .is_some_and(|size| size > BODY_LIMIT as u64)
    {
        return Err(fail("claude_mapping_body_too_large"));
    }
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| fail("claude_mapping_body_invalid"))?;
        if let Ok(data) = frame.into_data() {
            if bytes.len().saturating_add(data.len()) > BODY_LIMIT {
                return Err(fail("claude_mapping_body_too_large"));
            }
            bytes.extend_from_slice(&data);
        }
    }
    Ok(bytes)
}

struct RawObject(Vec<(String, Box<RawValue>)>);
impl<'de> Deserialize<'de> for RawObject {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = RawObject;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON object with unique top-level fields")
            }
            fn visit_map<M: MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<RawObject, M::Error> {
                let mut fields = Vec::new();
                let mut keys = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(serde::de::Error::custom("duplicate field"));
                    }
                    fields.push((key, map.next_value::<Box<RawValue>>()?));
                }
                Ok(RawObject(fields))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}
impl RawObject {
    fn parse(body: &[u8], route: &LocalRoute) -> Result<(Self, String)> {
        let object: Self =
            serde_json::from_slice(body).map_err(|_| fail("claude_mapping_json_invalid"))?;
        let model = object
            .0
            .iter()
            .find(|(key, _)| key == "model")
            .ok_or_else(|| fail("claude_mapping_model_invalid"))?;
        let model: String = serde_json::from_str(model.1.get())
            .map_err(|_| fail("claude_mapping_model_invalid"))?;
        if !allowed_alias(route, &model) {
            return Err(fail("claude_mapping_model_invalid"));
        }
        Ok((object, model))
    }
    fn rewrite(self, model: &str) -> Result<Vec<u8>> {
        let replacement =
            serde_json::to_vec(model).map_err(|_| fail("claude_mapping_model_invalid"))?;
        let mut bytes = vec![b'{'];
        for (index, (key, value)) in self.0.into_iter().enumerate() {
            if index != 0 {
                bytes.push(b',');
            }
            serde_json::to_writer(&mut bytes, &key)
                .map_err(|_| fail("claude_mapping_json_invalid"))?;
            bytes.push(b':');
            bytes.extend_from_slice(if key == "model" {
                &replacement
            } else {
                value.get().as_bytes()
            });
        }
        bytes.push(b'}');
        Ok(bytes)
    }
}

struct Inflight {
    state: Arc<ServerState>,
    _slot: OwnedSemaphorePermit,
}
impl Inflight {
    fn new(state: Arc<ServerState>, slot: OwnedSemaphorePermit) -> Self {
        state.inflight.fetch_add(1, Ordering::SeqCst);
        Self { state, _slot: slot }
    }
}
impl Drop for Inflight {
    fn drop(&mut self) {
        self.state.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Observation {
    permit: Option<ForwardPermit>,
    authority: Arc<dyn MappingAuthority>,
    outcome: ForwardOutcome,
    runtime: tokio::runtime::Handle,
    work: Option<NativeWork>,
}
#[derive(Clone)]
struct NativeWork {
    _connection: Arc<OwnedSemaphorePermit>,
    _inflight: Arc<Inflight>,
}
impl Observation {
    fn new(permit: ForwardPermit, authority: Arc<dyn MappingAuthority>, work: NativeWork) -> Self {
        Self {
            permit: Some(permit),
            authority,
            outcome: ForwardOutcome::Unconfirmed,
            runtime: tokio::runtime::Handle::current(),
            work: Some(work),
        }
    }
    fn complete(&mut self, status: u16) {
        self.outcome = ForwardOutcome::Complete { status };
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        if let Some(mut permit) = self.permit.take() {
            permit.api_key.zeroize();
            let authority = self.authority.clone();
            let outcome = self.outcome;
            let work = self.work.take();
            // Observation may touch the Vault/database, including on a dropped
            // body. It must not run on the HTTP task or delay stream progress.
            self.runtime.spawn_blocking(move || {
                let _ = authority.observe(permit, outcome);
                drop(work);
            });
        }
    }
}

fn streaming_response(
    response: MappingResponse,
    observation: Observation,
    stopped: watch::Receiver<bool>,
    started: Instant,
) -> Response<LocalBody> {
    let status = match StatusCode::from_u16(response.status) {
        Ok(status) if !status.is_informational() => status,
        _ => return local_error(StatusCode::BAD_GATEWAY, "result_unconfirmed"),
    };
    if response
        .headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
        .sum::<usize>()
        > HEADER_LIMIT
        || response
            .headers
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > OUTPUT_LIMIT)
    {
        return local_error(StatusCode::BAD_GATEWAY, "claude_mapping_output_too_large");
    }
    struct StreamState {
        response: MappingResponse,
        observation: Observation,
        stopped: watch::Receiver<bool>,
        deadline: Instant,
        bytes: u64,
        done: bool,
    }
    let mut headers = HeaderMap::new();
    for (name, value) in &response.headers {
        let name_str = name.as_str();
        if matches!(
            name_str,
            "content-type"
                | "content-encoding"
                | "request-id"
                | "x-request-id"
                | "x-orivelle-request-id"
        ) || name_str.starts_with("anthropic-ratelimit-")
        {
            headers.append(name.clone(), value.clone());
        }
    }
    // Never pass Location, cookies, hop-by-hop fields, or a provider retry hint.
    terminal_headers(&mut headers);
    let state = StreamState {
        response,
        observation,
        stopped,
        deadline: started + TOTAL_TIMEOUT,
        bytes: 0,
        done: false,
    };
    let stream = stream::unfold(state, |mut state| async move {
        if state.done {
            return None;
        }
        let deadline = std::cmp::min(state.deadline, Instant::now() + IDLE_TIMEOUT);
        let chunk = tokio::select! {
            biased;
            _ = wait_stop(&mut state.stopped) => Err(fail("result_unconfirmed")),
            chunk = tokio::time::timeout_at(deadline, state.response.stream.next()) => {
                match chunk {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => {
                        state.observation.complete(state.response.status);
                        return None;
                    },
                    Err(_) => Err(fail("result_unconfirmed")),
                }
            },
        };
        match chunk {
            Ok(chunk) if state.bytes.saturating_add(chunk.len() as u64) <= OUTPUT_LIMIT => {
                state.bytes += chunk.len() as u64;
                Some((Ok(Frame::data(chunk)), state))
            }
            _ => {
                state.done = true;
                Some((Err(fail("result_unconfirmed")), state))
            }
        }
    });
    let mut response = Response::new(StreamBody::new(stream).boxed_unsync());
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

fn terminal_headers(headers: &mut HeaderMap) {
    headers.insert("x-should-retry", HeaderValue::from_static("false"));
    headers.insert(header::CONNECTION, HeaderValue::from_static("close"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}
fn local_error(status: StatusCode, code: &str) -> Response<LocalBody> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "type": "error", "error": { "type": code, "message": code }
    }))
    .unwrap();
    let mut response = Response::new(
        Full::new(Bytes::from(bytes))
            .map_err(|never| match never {})
            .boxed_unsync(),
    );
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    terminal_headers(response.headers_mut());
    response
}
fn models_response(route: &LocalRoute) -> Response<LocalBody> {
    let data: Vec<_> = route
        .aliases
        .iter()
        .map(|alias| {
            serde_json::json!({
                "id": alias.name, "type": "model", "display_name": alias.label_override,
                "created_at": route.created_at,
            })
        })
        .collect();
    let bytes = serde_json::to_vec(&serde_json::json!({
        "data": data, "has_more": false,
        "first_id": route.aliases.first().map(|alias| &alias.name),
        "last_id": route.aliases.last().map(|alias| &alias.name),
    }))
    .unwrap();
    let mut response = Response::new(
        Full::new(Bytes::from(bytes))
            .map_err(|never| match never {})
            .boxed_unsync(),
    );
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    terminal_headers(response.headers_mut());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        claude_mapping::{MappingAccess, ModelAlias},
        network::FixtureRequest,
    };
    use std::sync::{Condvar, Mutex};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpStream,
        sync::Notify,
    };

    const BINDING: &str = "11111111-1111-4111-8111-111111111111";
    const ALIAS: &str = "claude-sonnet-4-6";
    const LOCAL: &str = "synthetic-local-mapping-token";
    const REMOTE: &str = "synthetic-panel-inference-key";
    const MODEL: &str = "gpt-6-luna";

    struct TestAuthority {
        reject_at: AtomicUsize,
        observed: Mutex<Vec<ForwardOutcome>>,
        leases: Arc<AtomicUsize>,
        block: Arc<NativeBlock>,
    }
    impl TestAuthority {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                reject_at: AtomicUsize::new(0),
                observed: Mutex::new(vec![]),
                leases: Arc::new(AtomicUsize::new(0)),
                block: Arc::new(NativeBlock::default()),
            })
        }
    }
    #[derive(Default)]
    struct NativeBlock {
        phase: AtomicUsize,
        entered: AtomicUsize,
        completed: AtomicUsize,
        open: Mutex<bool>,
        released: Condvar,
    }
    impl NativeBlock {
        fn wait(&self, phase: usize) {
            if self.phase.load(Ordering::SeqCst) != phase {
                return;
            }
            self.entered.fetch_add(1, Ordering::SeqCst);
            let mut open = self.open.lock().unwrap();
            while !*open {
                open = self.released.wait(open).unwrap();
            }
            self.completed.fetch_add(1, Ordering::SeqCst);
        }
        fn release(&self) {
            *self.open.lock().unwrap() = true;
            self.released.notify_all();
        }
    }
    struct UnblockOnDrop(Arc<NativeBlock>);
    impl Drop for UnblockOnDrop {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    struct Lease(Arc<AtomicUsize>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    impl MappingAuthority for TestAuthority {
        fn authenticate(&self, bearer: &[u8]) -> Result<MappingAccess> {
            if bearer != LOCAL.as_bytes() {
                return Err(fail("binding_disabled"));
            }
            self.block.wait(1);
            self.leases.fetch_add(1, Ordering::SeqCst);
            Ok(MappingAccess {
                binding_id: BINDING.into(),
                route_revision: "synthetic-revision".into(),
                _lease: Box::new(Lease(self.leases.clone())),
            })
        }
        fn authorize_messages(
            &self,
            access: MappingAccess,
            alias: String,
        ) -> Result<ForwardPermit> {
            self.block.wait(2);
            if self.reject_at.load(Ordering::SeqCst) == 1 {
                return Err(fail("binding_disabled"));
            }
            Ok(ForwardPermit {
                access,
                messages_url: url::Url::parse("https://edge.example.test/anthropic/v1/messages")
                    .unwrap(),
                upstream_model_id: MODEL.into(),
                requested_alias: alias,
                api_key: Zeroizing::new(REMOTE.into()),
            })
        }
        fn before_send(&self, permit: ForwardPermit) -> Result<ForwardPermit> {
            self.block.wait(3);
            if self.reject_at.load(Ordering::SeqCst) == 2 {
                return Err(fail("credential_missing"));
            }
            assert_eq!(permit.access.route_revision, "synthetic-revision");
            Ok(permit)
        }
        fn observe(&self, permit: ForwardPermit, outcome: ForwardOutcome) -> Result<()> {
            self.block.wait(4);
            assert!(permit.api_key.is_empty());
            self.observed.lock().unwrap().push(outcome);
            Ok(())
        }
    }
    fn route(port: u16) -> LocalRoute {
        LocalRoute {
            binding_id: BINDING.into(),
            port,
            base_path: format!("/orivelle-orbit/claude/{BINDING}"),
            aliases: vec![ModelAlias {
                name: ALIAS.into(),
                label_override: "gpt-6-luna (Panel)".into(),
            }],
            created_at: "2026-11-06T00:00:00Z".into(),
        }
    }
    fn setup(http: Http, authority: Arc<TestAuthority>) -> (ServerHandle, LocalRoute) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let route = route(listener.local_addr().unwrap().port());
        let handle = start_local(listener, route.clone(), authority, http).unwrap();
        (handle, route)
    }
    fn url(route: &LocalRoute) -> String {
        format!(
            "http://127.0.0.1:{}{}/v1/messages",
            route.port, route.base_path
        )
    }
    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap()
    }
    fn body() -> String {
        format!(
            r#"{{"model":"{ALIAS}","messages":[{{"role":"user","content":"synthetic"}}],"max_tokens":1}}"#
        )
    }
    async fn raw(route: &LocalRoute, request: String) -> String {
        let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, route.port))
            .await
            .unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        String::from_utf8(response).unwrap()
    }
    async fn eventually(mut condition: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    fn response(status: u16, bytes: &'static [u8]) -> MappingResponse {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert("x-should-retry", HeaderValue::from_static("true"));
        headers.insert(
            header::LOCATION,
            HeaderValue::from_static("https://foreign.example.test/replay"),
        );
        headers.insert(
            header::SET_COOKIE,
            HeaderValue::from_static("synthetic=never-forward"),
        );
        MappingResponse {
            status,
            headers,
            stream: Box::pin(stream::iter([Ok(Bytes::from_static(bytes))])),
        }
    }

    #[tokio::test]
    async fn mapping_rejects_auth_host_browser_method_path_and_query_before_upstream() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sent = attempts.clone();
        let http = Http::fixture();
        http.scripted_mapping(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(response(200, b"{}")) })
        }));
        let authority = TestAuthority::new();
        let (handle, route) = setup(http, authority.clone());
        let target = format!("{}/v1/messages", route.base_path);
        let host = format!("127.0.0.1:{}", route.port);
        let valid_auth = format!("Authorization: Bearer {LOCAL}\r\n");
        for (method, target, host, extra, auth) in [
            (
                "POST",
                target.clone(),
                host.clone(),
                "".to_owned(),
                "".to_owned(),
            ),
            (
                "POST",
                target.clone(),
                host.clone(),
                "".to_owned(),
                "Authorization: Bearer wrong\r\n".to_owned(),
            ),
            (
                "POST",
                target.clone(),
                format!("localhost:{}", route.port),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                target.clone(),
                "foreign.example.test".to_owned(),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                target.clone(),
                host.clone(),
                "Origin: null\r\n".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                target.clone(),
                host.clone(),
                "Sec-Fetch-Mode: cors\r\n".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                target.clone(),
                host.clone(),
                "Connection: upgrade\r\nUpgrade: websocket\r\n".to_owned(),
                valid_auth.clone(),
            ),
            (
                "PUT",
                target.clone(),
                host.clone(),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                "/v1/messages".to_owned(),
                host.clone(),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                format!("{target}?beta=false"),
                host.clone(),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                format!("{target}?beta=true&beta=true"),
                host.clone(),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "POST",
                format!("http://{host}{target}"),
                host.clone(),
                "".to_owned(),
                valid_auth.clone(),
            ),
            (
                "CONNECT",
                host.clone(),
                host.clone(),
                "".to_owned(),
                valid_auth.clone(),
            ),
        ] {
            let request = format!(
                "{method} {target} HTTP/1.1\r\nHost: {host}\r\n{auth}{extra}Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
            );
            let response = raw(&route, request).await;
            assert!(response.starts_with("HTTP/1.1 4"));
            assert!(
                response
                    .to_ascii_lowercase()
                    .contains("x-should-retry: false")
            );
            assert_eq!(attempts.load(Ordering::SeqCst), 0);
        }
        let models = client()
            .get(format!(
                "http://127.0.0.1:{}{}/v1/models",
                route.port, route.base_path
            ))
            .bearer_auth(LOCAL)
            .send()
            .await
            .unwrap();
        assert_eq!(models.status(), StatusCode::OK);
        let models: serde_json::Value = models.json().await.unwrap();
        assert_eq!(models["data"][0]["id"], ALIAS);
        assert_eq!(models["data"][0]["display_name"], "gpt-6-luna (Panel)");
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        eventually(|| authority.leases.load(Ordering::SeqCst) == 0).await;
        handle.stop();
    }

    #[test]
    fn mapping_rewrites_only_top_level_model_and_preserves_raw_numbers() {
        let route = route(1234);
        let input = format!(
            r#"{{"model":"{ALIAS}","max_tokens":900719925474099312345678901234567890,"temperature":1.234567890123456789e-42,"metadata":{{"model":"{ALIAS}"}},"tools":[{{"input_schema":{{"default":184467440737095516161}}}}],"system":"model: {ALIAS}"}}"#
        );
        let rewritten = RawObject::parse(input.as_bytes(), &route)
            .unwrap()
            .0
            .rewrite(MODEL)
            .unwrap();
        let rewritten = String::from_utf8(rewritten).unwrap();
        assert!(rewritten.contains(r#""model":"gpt-6-luna""#));
        for raw in [
            "900719925474099312345678901234567890",
            "1.234567890123456789e-42",
            "184467440737095516161",
        ] {
            assert!(rewritten.contains(raw));
        }
        assert!(rewritten.contains(&format!(r#""metadata":{{"model":"{ALIAS}"}}"#)));
        assert!(rewritten.contains(&format!(r#""system":"model: {ALIAS}""#)));
        for invalid in [
            r#"{"model":"gpt-6-luna"}"#,
            r#"{"model":"claude-sonnet-4-6","model":"claude-sonnet-4-6"}"#,
            r#"{"model":42}"#,
            r#"[]"#,
        ] {
            assert!(RawObject::parse(invalid.as_bytes(), &route).is_err());
        }
    }

    #[tokio::test]
    async fn mapping_streams_tool_sse_with_backpressure_and_only_upstream_credentials() {
        const FIRST: &[u8] = b"event: content_block_start\ndata: {\"type\":\"tool_use\",\"id\":\"synthetic-tool\"}\n\n";
        const LAST: &[u8] = b"event: content_block_delta\ndata: {\"partial_json\":\"{\\\"x\\\":1}\"}\n\nevent: message_stop\ndata: {}\n\n";
        let gate = Arc::new(Notify::new());
        let completed = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let http = Http::fixture();
        let send_gate = gate.clone();
        let send_completed = completed.clone();
        let sent = attempts.clone();
        http.scripted_mapping(Arc::new(move |request: FixtureRequest| {
            sent.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.method, Method::POST);
            assert_eq!(
                request.url,
                "https://edge.example.test/anthropic/v1/messages?beta=true"
            );
            assert!(
                String::from_utf8(request.body)
                    .unwrap()
                    .contains(r#""model":"gpt-6-luna""#)
            );
            for (name, value) in request.headers {
                assert!(!value.contains(LOCAL));
                assert!(matches!(
                    name.as_str(),
                    "authorization"
                        | "content-type"
                        | "accept"
                        | "anthropic-version"
                        | "anthropic-beta"
                ));
                if name == "authorization" {
                    assert_eq!(value, format!("Bearer {REMOTE}"));
                }
            }
            let gate = send_gate.clone();
            let completed = send_completed.clone();
            Box::pin(async move {
                let chunks =
                    stream::unfold((0, gate, completed), |(step, gate, completed)| async move {
                        match step {
                            0 => Some((Ok(Bytes::from_static(FIRST)), (1, gate, completed))),
                            1 => {
                                gate.notified().await;
                                Some((Ok(Bytes::from_static(LAST)), (2, gate, completed)))
                            }
                            _ => {
                                completed.store(true, Ordering::SeqCst);
                                None
                            }
                        }
                    });
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/event-stream"),
                );
                headers.insert("x-should-retry", HeaderValue::from_static("true"));
                Ok(MappingResponse {
                    status: 200,
                    headers,
                    stream: Box::pin(chunks),
                })
            })
        }));
        let authority = TestAuthority::new();
        let (handle, route) = setup(http, authority.clone());
        let response = client()
            .post(format!("{}?beta=true", url(&route)))
            .bearer_auth(LOCAL)
            .header(header::CONTENT_TYPE, "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "synthetic-beta")
            .header(header::COOKIE, "never-forward")
            .header("x-api-key", "never-forward")
            .header("x-stainless-lang", "js")
            .body(body())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-should-retry"], "false");
        let mut chunks = response.bytes_stream();
        let first = tokio::time::timeout(Duration::from_secs(1), chunks.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first, FIRST);
        assert!(!completed.load(Ordering::SeqCst));
        assert_eq!(handle.inflight(), 1);
        gate.notify_one();
        let mut rest = Vec::new();
        while let Some(chunk) = chunks.next().await {
            rest.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(rest, LAST);
        eventually(|| authority.observed.lock().unwrap().len() == 1 && handle.inflight() == 0)
            .await;
        assert!(matches!(
            authority.observed.lock().unwrap()[0],
            ForwardOutcome::Complete { status: 200 }
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        handle.stop();
    }

    #[tokio::test]
    async fn mapping_preserves_error_status_and_bytes_without_redirect_or_retry() {
        const ERROR: &[u8] = b"{ \"error\": {\"type\":\"synthetic_error\",\"message\":\"raw \\u4e2d\"}, \"n\":90071992547409931234 }\n";
        let attempts = Arc::new(AtomicUsize::new(0));
        let sent = attempts.clone();
        let http = Http::fixture();
        http.scripted_mapping(Arc::new(move |_| {
            let index = sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { Ok(response([429, 529, 307][index], ERROR)) })
        }));
        let authority = TestAuthority::new();
        let (handle, route) = setup(http, authority.clone());
        for status in [429, 529, 307] {
            let response = client()
                .post(url(&route))
                .bearer_auth(LOCAL)
                .header(header::CONTENT_TYPE, "application/json")
                .body(body())
                .send()
                .await
                .unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()["x-should-retry"], "false");
            assert!(!response.headers().contains_key(header::LOCATION));
            assert!(!response.headers().contains_key(header::SET_COOKIE));
            assert_eq!(response.bytes().await.unwrap(), ERROR);
        }
        eventually(|| authority.observed.lock().unwrap().len() == 3).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        handle.stop();
    }

    #[tokio::test]
    async fn mapping_late_authority_or_credential_change_fails_before_send() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sent = attempts.clone();
        let http = Http::fixture();
        http.scripted_mapping(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(response(200, b"{}")) })
        }));
        let authority = TestAuthority::new();
        let (handle, route) = setup(http, authority.clone());
        for stage in [1, 2] {
            authority.reject_at.store(stage, Ordering::SeqCst);
            let response = client()
                .post(url(&route))
                .bearer_auth(LOCAL)
                .header(header::CONTENT_TYPE, "application/json")
                .body(body())
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert_eq!(response.headers()["x-should-retry"], "false");
            let _ = response.bytes().await.unwrap();
            assert_eq!(attempts.load(Ordering::SeqCst), 0);
        }
        eventually(|| authority.leases.load(Ordering::SeqCst) == 0 && handle.inflight() == 0).await;
        handle.stop();
    }

    #[tokio::test]
    async fn mapping_interruption_and_stop_never_replay_and_concurrency_is_bounded() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sent = attempts.clone();
        let http = Http::fixture();
        http.scripted_mapping(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                let chunks = stream::unfold((), |()| async {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    Some((Ok(Bytes::from(vec![b'x'; 128 * 1024])), ()))
                });
                let mut headers = HeaderMap::new();
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/event-stream"),
                );
                Ok(MappingResponse {
                    status: 200,
                    headers,
                    stream: Box::pin(chunks),
                })
            })
        }));
        let authority = TestAuthority::new();
        let (handle, route) = setup(http, authority.clone());
        let response = client()
            .post(url(&route))
            .bearer_auth(LOCAL)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body())
            .send()
            .await
            .unwrap();
        let mut chunks = response.bytes_stream();
        assert!(chunks.next().await.unwrap().is_ok());
        drop(chunks);
        eventually(|| handle.inflight() == 0 && authority.observed.lock().unwrap().len() == 1)
            .await;
        assert!(matches!(
            authority.observed.lock().unwrap()[0],
            ForwardOutcome::Unconfirmed
        ));
        assert_eq!(attempts.load(Ordering::SeqCst), 1);

        let first = client()
            .post(url(&route))
            .bearer_auth(LOCAL)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body())
            .send()
            .await
            .unwrap();
        let second = client()
            .post(url(&route))
            .bearer_auth(LOCAL)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body())
            .send()
            .await
            .unwrap();
        assert_eq!(handle.inflight(), 2);
        let busy = client()
            .post(url(&route))
            .bearer_auth(LOCAL)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body())
            .send()
            .await
            .unwrap();
        assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        handle.stop();
        assert!(!handle.is_running());
        drop((first, second));
        eventually(|| handle.inflight() == 0 && authority.observed.lock().unwrap().len() == 3)
            .await;
        assert!(
            authority
                .observed
                .lock()
                .unwrap()
                .iter()
                .all(|outcome| matches!(outcome, ForwardOutcome::Unconfirmed))
        );
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        eventually(|| authority.leases.load(Ordering::SeqCst) == 0).await;
    }

    #[tokio::test]
    async fn mapping_malformed_hyper_request_closes_without_upstream() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let sent = attempts.clone();
        let http = Http::fixture();
        http.scripted_mapping(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(response(200, b"{}")) })
        }));
        let authority = TestAuthority::new();
        let (handle, route) = setup(http, authority.clone());
        let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, route.port))
            .await
            .unwrap();
        socket.write_all(format!(
            "POST {}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nBad Header: invalid\r\nContent-Length: 0\r\n\r\n",
            route.base_path, route.port,
        ).as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        let closed = tokio::time::timeout(Duration::from_secs(2), socket.read_to_end(&mut bytes))
            .await
            .unwrap();
        assert!(
            closed.is_ok()
                || matches!(
                    closed.unwrap_err().kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
                )
        );
        // Hyper owns pre-service syntax errors. No custom wire parser/header
        // rewriting is added, and its response need not have service headers.
        assert!(bytes.is_empty() || bytes.starts_with(b"HTTP/1.1 4"));
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert_eq!(authority.leases.load(Ordering::SeqCst), 0);
        handle.stop();
    }

    #[tokio::test]
    async fn mapping_blocking_native_work_retains_limits_after_disconnect_and_stop() {
        for phase in [1, 2, 3, 4] {
            let attempts = Arc::new(AtomicUsize::new(0));
            let sent = attempts.clone();
            let http = Http::fixture();
            http.scripted_mapping(Arc::new(move |_| {
                sent.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(response(200, b"{}")) })
            }));
            let authority = TestAuthority::new();
            authority.block.phase.store(phase, Ordering::SeqCst);
            let _unblock_on_failure = UnblockOnDrop(authority.block.clone());
            let (handle, route) = setup(http, authority.clone());
            let count = if phase == 1 { 16 } else { 2 };
            let mut sockets = Vec::new();
            for _ in 0..count {
                if phase == 4 {
                    let response = client()
                        .post(url(&route))
                        .bearer_auth(LOCAL)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(body())
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(response.bytes().await.unwrap(), b"{}".as_slice());
                } else {
                    let bytes = body();
                    let mut socket = TcpStream::connect((Ipv4Addr::LOCALHOST, route.port))
                        .await
                        .unwrap();
                    socket.write_all(format!(
                        "POST {}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {LOCAL}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{bytes}",
                        route.base_path, route.port, bytes.len(),
                    ).as_bytes()).await.unwrap();
                    sockets.push(socket);
                }
            }
            eventually(|| authority.block.entered.load(Ordering::SeqCst) == count).await;
            if phase != 1 {
                let busy = client()
                    .post(url(&route))
                    .bearer_auth(LOCAL)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body())
                    .send()
                    .await
                    .unwrap();
                assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
                let _ = busy.bytes().await.unwrap();
            }
            drop(sockets);
            handle.stop();
            // Let the HTTP futures cancel while the controlled native calls
            // remain blocked. Their owned slots must survive those futures.
            tokio::time::sleep(Duration::from_millis(20)).await;
            assert_eq!(authority.block.completed.load(Ordering::SeqCst), 0);
            assert_eq!(handle.0.connections.available_permits(), 16 - count);
            assert_eq!(handle.inflight(), if phase == 1 { 0 } else { 2 });
            assert_eq!(
                handle.0.upstream.available_permits(),
                if phase == 1 { 2 } else { 0 }
            );
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                if phase == 4 { 2 } else { 0 }
            );
            authority.block.release();
            eventually(|| {
                authority.block.completed.load(Ordering::SeqCst) == count
                    && authority.leases.load(Ordering::SeqCst) == 0
                    && handle.inflight() == 0
                    && handle.0.connections.available_permits() == 16
                    && handle.0.upstream.available_permits() == 2
            })
            .await;
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                if phase == 4 { 2 } else { 0 }
            );
        }
    }
}
