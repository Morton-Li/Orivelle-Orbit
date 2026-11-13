use crate::{
    claude_mapping::{ForwardPermit, ProtocolHeaders},
    error::{Result, fail},
    identity::{Installation, ProofInput, canonical_origin, sign_proof, uuid},
    vault::Vault,
};
use futures_util::StreamExt;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;
use url::Url;

pub const PRODUCTION_PANEL_ORIGIN: &str = "https://orivelle.powerbyte.io";
const USER_AGENT: &str = concat!("Orivelle-Orbit/", env!("CARGO_PKG_VERSION"));

#[derive(Clone)]
pub struct Http {
    client: Client,
    mapping_client: Client,
    allow_loopback: bool,
    #[cfg(test)]
    fixture: std::sync::Arc<std::sync::Mutex<Option<FixtureHandler>>>,
    #[cfg(test)]
    mapping_fixture: std::sync::Arc<std::sync::Mutex<Option<MappingFixtureHandler>>>,
    #[cfg(test)]
    local_acceptance_origin: Option<String>,
    #[cfg(test)]
    local_acceptance_callback: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}
#[cfg(test)]
pub struct FixtureRequest {
    pub method: Method,
    pub url: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}
#[cfg(test)]
pub type FixtureHandler = std::sync::Arc<
    dyn Fn(
            FixtureRequest,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<RemoteResponse>> + Send>>
        + Send
        + Sync,
>;
pub(crate) type MappingStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = Result<hyper::body::Bytes>> + Send>>;
pub(crate) struct MappingResponse {
    pub status: u16,
    pub headers: reqwest::header::HeaderMap,
    pub stream: MappingStream,
}
#[cfg(test)]
pub(crate) type MappingFixtureHandler = std::sync::Arc<
    dyn Fn(
            FixtureRequest,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<MappingResponse>> + Send>>
        + Send
        + Sync,
>;
impl Http {
    pub fn new() -> Result<Self> {
        Self::build(false)
    }
    fn build(allow_loopback: bool) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| fail("network_unavailable"))?;
        // The mapping is an explicit native-only HTTPS route. It does not use
        // environment proxy settings or permit caller-selected redirects.
        let mapping_client = Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| fail("network_unavailable"))?;
        Ok(Self {
            client,
            mapping_client,
            allow_loopback,
            #[cfg(test)]
            fixture: Default::default(),
            #[cfg(test)]
            mapping_fixture: Default::default(),
            #[cfg(test)]
            local_acceptance_origin: None,
            #[cfg(test)]
            local_acceptance_callback: Default::default(),
        })
    }
    /// One process-local CA and one reserved hostname for real local Panel
    /// acceptance. This constructor is absent from production candidates.
    #[cfg(test)]
    pub(crate) fn for_local_tls_acceptance(ca_pem: &[u8], port: u16) -> Result<Self> {
        if port == 0 || ca_pem.is_empty() || ca_pem.len() > 64 * 1024 {
            return Err(fail("local_tls_input_invalid"));
        }
        let certs = reqwest::Certificate::from_pem_bundle(ca_pem)
            .map_err(|_| fail("local_tls_ca_invalid"))?;
        if certs.len() != 1 {
            return Err(fail("local_tls_ca_invalid"));
        }
        let origin = canonical_origin(&format!("https://orbit-e2e.test:{port}"))?;
        let client = Client::builder()
            .tls_certs_only(certs)
            .resolve(
                "orbit-e2e.test",
                std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            )
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|_| fail("local_tls_client_unavailable"))?;
        Ok(Self {
            mapping_client: client.clone(),
            client,
            allow_loopback: false,
            fixture: Default::default(),
            mapping_fixture: Default::default(),
            local_acceptance_origin: Some(origin),
            local_acceptance_callback: Default::default(),
        })
    }
    #[cfg(test)]
    pub(crate) fn local_acceptance_callback_uri(&self) -> Result<String> {
        if self.local_acceptance_origin.is_none() {
            return Err(fail("local_callback_observer_outside_scope"));
        }
        self.local_acceptance_callback
            .lock()
            .map_err(|_| fail("local_callback_receipt_unavailable"))?
            .clone()
            .ok_or_else(|| fail("local_callback_receipt_missing"))
    }
    #[cfg(test)]
    fn observe_local_callback(&self, method: &Method, url: &Url, body: &[u8]) -> Result<()> {
        if self.local_acceptance_origin.is_none()
            || *method != Method::POST
            || url.path() != "/api/v1/orbit/authorization-requests"
        {
            return Ok(());
        }
        if url.query().is_some() || body.len() > 64 * 1024 {
            return Err(fail("local_callback_request_invalid"));
        }
        // Deserialize only this field; state and all other unknown request
        // fields are skipped, never copied into a trace or persisted/logged.
        #[derive(Deserialize)]
        struct Callback {
            redirect_uri: String,
        }
        let receipt: Callback =
            serde_json::from_slice(body).map_err(|_| fail("local_callback_request_invalid"))?;
        let callback = Url::parse(&receipt.redirect_uri)
            .map_err(|_| fail("local_callback_request_invalid"))?;
        let port = callback
            .port_or_known_default()
            .filter(|port| *port != 0)
            .ok_or_else(|| fail("local_callback_request_invalid"))?;
        if callback.scheme() != "http"
            || callback.host_str() != Some("127.0.0.1")
            || callback.path() != "/orivelle-orbit/callback"
            || callback.query().is_some()
            || callback.fragment().is_some()
            || !callback.username().is_empty()
            || callback.password().is_some()
            || receipt.redirect_uri != format!("http://127.0.0.1:{port}/orivelle-orbit/callback")
        {
            return Err(fail("local_callback_request_invalid"));
        }
        *self
            .local_acceptance_callback
            .lock()
            .map_err(|_| fail("local_callback_receipt_unavailable"))? = Some(receipt.redirect_uri);
        Ok(())
    }
    #[cfg(test)]
    pub fn fixture() -> Self {
        Self::build(true).unwrap()
    }
    #[cfg(test)]
    pub fn scripted(&self, handler: FixtureHandler) {
        *self.fixture.lock().unwrap() = Some(handler);
    }
    #[cfg(test)]
    pub(crate) fn scripted_mapping(&self, handler: MappingFixtureHandler) {
        *self.mapping_fixture.lock().unwrap() = Some(handler);
    }

    /// A single Claude mapping attempt, authorized by an opaque native permit.
    /// Unlike `request`, this returns the body stream without collecting it.
    /// The caller owns the permit and its lease until that stream terminates.
    pub(crate) async fn claude_mapping_messages(
        &self,
        permit: &mut ForwardPermit,
        body: Vec<u8>,
        protocol: ProtocolHeaders,
        beta: bool,
    ) -> Result<MappingResponse> {
        let mut url = self.validate_url(permit.messages_url.as_str())?;
        if url.scheme() != "https" || url.query().is_some() || !url.path().ends_with("/v1/messages")
        {
            return Err(fail("claude_mapping_destination_invalid"));
        }
        if beta {
            url.set_query(Some("beta=true"));
        }
        let authorization_text =
            zeroize::Zeroizing::new(format!("Bearer {}", permit.api_key.as_str()));
        let mut authorization = reqwest::header::HeaderValue::from_str(&authorization_text)
            .map_err(|_| fail("credential_missing"))?;
        authorization.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
        );
        for (name, value) in [
            ("anthropic-version", protocol.anthropic_version),
            ("anthropic-beta", protocol.anthropic_beta),
        ] {
            if let Some(value) = value {
                let value = reqwest::header::HeaderValue::from_str(&value)
                    .map_err(|_| fail("claude_mapping_headers_invalid"))?;
                headers.insert(name, value);
            }
        }
        #[cfg(test)]
        {
            let fixture = self.mapping_fixture.lock().unwrap().clone();
            if self.local_acceptance_origin.is_some() && fixture.is_some() {
                return Err(fail("local_tls_scripted_transport_forbidden"));
            }
            if let Some(handler) = fixture {
                return handler(FixtureRequest {
                    method: Method::POST,
                    url: url.to_string(),
                    body,
                    headers: headers
                        .iter()
                        .map(|(name, value)| {
                            (name.as_str().to_owned(), value.to_str().unwrap().to_owned())
                        })
                        .collect(),
                })
                .await;
            }
        }
        let request = self
            .mapping_client
            .post(url)
            .headers(headers)
            .timeout(Duration::from_secs(15 * 60))
            .body(body);
        // Waiting for inference response headers includes provider TTFT. Keep
        // it aligned with stream idle time, separate from the 10s connect and
        // local request-header limits.
        let response = tokio::time::timeout(Duration::from_secs(120), request.send())
            .await
            .map_err(|_| fail("result_unconfirmed"))?
            .map_err(|_| fail("result_unconfirmed"))?;
        Ok(MappingResponse {
            status: response.status().as_u16(),
            headers: response.headers().clone(),
            stream: Box::pin(
                response
                    .bytes_stream()
                    .map(|chunk| chunk.map_err(|_| fail("result_unconfirmed"))),
            ),
        })
    }
    pub(crate) fn require_panel_origin(&self, origin: &str) -> Result<()> {
        // Alternate issuers exist only in explicit, compile-time test clients.
        // Production and ordinary debug builds have no configurable override.
        #[cfg(test)]
        if self.allow_loopback || self.local_acceptance_origin.as_deref() == Some(origin) {
            return Ok(());
        }
        if origin != PRODUCTION_PANEL_ORIGIN {
            return Err(fail("panel_origin_not_allowed"));
        }
        Ok(())
    }
    pub fn validate_url(&self, raw: &str) -> Result<Url> {
        let url = Url::parse(raw).map_err(|_| fail("invalid_remote_url"))?;
        let loopback = url.host_str().is_some_and(|v| {
            v == "localhost"
                || v.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !(url.scheme() == "https" || self.allow_loopback && loopback && url.scheme() == "http")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(fail("invalid_remote_url"));
        }
        #[cfg(test)]
        if self
            .local_acceptance_origin
            .as_ref()
            .is_some_and(|origin| url.origin().ascii_serialization() != *origin)
        {
            return Err(fail("local_tls_origin_outside_scope"));
        }
        Ok(url)
    }
    pub async fn request(
        &self,
        method: Method,
        url: &str,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
        limit: u64,
        timeout: Duration,
    ) -> Result<RemoteResponse> {
        self.request_observed(method, url, body, headers, limit, timeout, None)
            .await
    }
    pub async fn request_observed(
        &self,
        method: Method,
        url: &str,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
        limit: u64,
        timeout: Duration,
        observation: Option<std::sync::Arc<std::sync::Mutex<Option<(u16, Option<String>)>>>>,
    ) -> Result<RemoteResponse> {
        let url = self.validate_url(url)?;
        #[cfg(test)]
        {
            let fixture = self.fixture.lock().unwrap().clone();
            if self.local_acceptance_origin.is_some() && fixture.is_some() {
                return Err(fail("local_tls_scripted_transport_forbidden"));
            }
            if let Some(handler) = fixture {
                let response = handler(FixtureRequest {
                    method,
                    url: url.to_string(),
                    body,
                    headers,
                })
                .await?;
                if response.bytes.len() as u64 > limit {
                    return Err(fail("response_too_large"));
                }
                if let Some(observation) = observation {
                    *observation.lock().map_err(|_| fail("state_unavailable"))? =
                        Some((response.status, response.request_id.clone()));
                }
                return Ok(response);
            }
        }
        #[cfg(test)]
        self.observe_local_callback(&method, &url, &body)?;
        let mut request = self.client.request(method, url).timeout(timeout).body(body);
        for (key, value) in headers {
            request = request.header(key, value);
        }
        let response = request.send().await.map_err(|error| {
            #[cfg(test)]
            if self.local_acceptance_origin.is_some() {
                use std::error::Error;
                let mut source = error.source();
                for _ in 0..8 {
                    let Some(cause) = source else { break };
                    // Locked rustls 0.23.45 Display forms, inspected only in
                    // memory. A refused TCP connection is not a TLS pass.
                    let message = cause.to_string();
                    if message == "invalid peer certificate: UnknownIssuer" {
                        return fail("local_tls_unknown_issuer");
                    }
                    if message
                        .starts_with("invalid peer certificate: certificate not valid for name ")
                        || message == "invalid peer certificate: NotValidForName"
                    {
                        return fail("local_tls_name_mismatch");
                    }
                    source = cause.source();
                }
            }
            let _ = error;
            fail("result_unconfirmed")
        })?;
        // All redirects are terminal. In particular, 307/308 never replay a paid POST.
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("x-orivelle-request-id")
            .and_then(|v| v.to_str().ok())
            .filter(|v| {
                v.len() <= 255
                    && v.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            })
            .map(str::to_owned);
        if let Some(observation) = observation {
            if let Ok(mut value) = observation.lock() {
                *value = Some((status, request_id.clone()));
            }
        }
        Ok(RemoteResponse {
            status,
            request_id,
            bytes: bounded_response_body(response, limit).await?,
        })
    }

    pub(crate) async fn get_update(
        &self,
        mut url: Url,
        accept: &str,
        limit: u64,
        timeout: Duration,
        allow_redirect: impl Fn(&Url, &Url) -> bool,
        progress: Option<&(dyn Fn(u64, Option<u64>) + Send + Sync)>,
    ) -> Result<RemoteResponse> {
        const MAX_REDIRECTS: usize = 3;
        self.validate_url(url.as_str())?;
        let deadline = tokio::time::Instant::now() + timeout;
        for redirects in 0..=MAX_REDIRECTS {
            let remaining = deadline
                .checked_duration_since(tokio::time::Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| fail("update_request_timeout"))?;
            let request = self
                .client
                .get(url.clone())
                .header(reqwest::header::ACCEPT, accept)
                .timeout(remaining)
                .send();
            let response = tokio::time::timeout(remaining.min(Duration::from_secs(30)), request)
                .await
                .map_err(|_| fail("update_request_timeout"))?
                .map_err(update_network_error)?;
            let status = response.status().as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return Ok(RemoteResponse {
                    status,
                    request_id: None,
                    bytes: update_response_body(response, limit, progress).await?,
                });
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| fail("invalid_update_redirect"))?;
            let next = url
                .join(location)
                .map_err(|_| fail("invalid_update_redirect"))?;
            self.validate_url(next.as_str())?;
            if redirects == MAX_REDIRECTS || !allow_redirect(&url, &next) {
                return Err(fail("invalid_update_redirect"));
            }
            url = next;
        }
        Err(fail("invalid_update_redirect"))
    }
    pub async fn panel<T: DeserializeOwned>(
        &self,
        origin: &str,
        target: &str,
        method: Method,
        body: Vec<u8>,
        credential: Option<(&Installation, &ProofSession, &dyn Vault)>,
        proof_with_bearer: bool,
    ) -> Result<T> {
        Ok(self
            .panel_envelope(origin, target, method, body, credential, proof_with_bearer)
            .await?
            .data)
    }
    pub async fn panel_envelope<T: DeserializeOwned>(
        &self,
        origin: &str,
        target: &str,
        method: Method,
        body: Vec<u8>,
        credential: Option<(&Installation, &ProofSession, &dyn Vault)>,
        proof_with_bearer: bool,
    ) -> Result<PanelEnvelope<T>> {
        self.require_panel_origin(origin)?;
        if canonical_origin(origin)? != origin
            || !target.starts_with("/api/v1/orbit/")
            || target.contains('#')
            || target.contains("..")
        {
            return Err(fail("invalid_panel_target"));
        }
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Accept".into(), "application/json".into()),
        ];
        if let Some((install, session, vault)) = credential {
            if install.origin != origin {
                return Err(fail("installation_identity_mismatch"));
            }
            let id = uuid();
            let timestamp = chrono::Utc::now().timestamp();
            let input = ProofInput {
                method: method.as_str(),
                request_target: target,
                binding_id: &session.binding_id,
                timestamp,
                nonce: &session.nonce,
                request_id: &id,
                body: &body,
                bearer: if proof_with_bearer {
                    Some(&session.access_token)
                } else {
                    None
                },
            };
            let signature = sign_proof(install, &input, vault)?;
            headers.extend([
                ("X-Orbit-Installation-Id".into(), install.id.clone()),
                ("X-Orbit-Proof-Binding".into(), session.binding_id.clone()),
                ("X-Orbit-Proof-Timestamp".into(), timestamp.to_string()),
                ("X-Orbit-Proof-Nonce".into(), session.nonce.clone()),
                ("X-Orbit-Proof-Request-Id".into(), id),
                ("X-Orbit-Proof-Signature".into(), signature),
            ]);
            if proof_with_bearer {
                headers.push((
                    "Authorization".into(),
                    format!("Bearer {}", session.access_token),
                ));
            }
        }
        let response = self
            .request(
                method,
                &format!("{origin}{target}"),
                body,
                headers,
                1024 * 1024,
                Duration::from_secs(20),
            )
            .await?;
        if !(200..300).contains(&response.status) {
            return Err(fail(match response.status {
                401 => "session_expired",
                403 => "permission_denied",
                409 => "connection_conflict",
                429 => "rate_limited",
                _ => "panel_unavailable",
            }));
        }
        let envelope: PanelEnvelope<T> =
            serde_json::from_slice(&response.bytes).map_err(|_| fail("invalid_panel_response"))?;
        Ok(envelope)
    }
}
async fn bounded_response_body(response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|len| len > limit) {
        return Err(fail("response_too_large"));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| fail("result_unconfirmed"))?;
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > limit {
            return Err(fail("response_too_large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

pub struct RemoteResponse {
    pub status: u16,
    pub request_id: Option<String>,
    pub bytes: Vec<u8>,
}

fn update_network_error(error: reqwest::Error) -> crate::error::PublicError {
    fail(if error.is_timeout() {
        "update_request_timeout"
    } else {
        "update_request_failed"
    })
}
async fn update_response_body(
    response: reqwest::Response,
    limit: u64,
    progress: Option<&(dyn Fn(u64, Option<u64>) + Send + Sync)>,
) -> Result<Vec<u8>> {
    let total = response.content_length();
    if total.is_some_and(|len| len > limit) {
        return Err(fail("response_too_large"));
    }
    let mut bytes = Vec::new();
    let mut last_progress = std::time::Instant::now();
    if let Some(progress) = progress {
        progress(0, total.filter(|total| *total > 0));
    }
    let mut stream = response.bytes_stream();
    loop {
        // Slow downloads can continue; a stalled connection fails with an updater error.
        let chunk = tokio::time::timeout(Duration::from_secs(30), stream.next())
            .await
            .map_err(|_| fail("update_request_timeout"))?;
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(update_network_error)?;
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > limit {
            return Err(fail("response_too_large"));
        }
        bytes.extend_from_slice(&chunk);
        if last_progress.elapsed() >= Duration::from_millis(100)
            || total == Some(bytes.len() as u64)
        {
            if let Some(progress) = progress {
                progress(bytes.len() as u64, total);
            }
            last_progress = std::time::Instant::now();
        }
    }
    if let Some(progress) = progress {
        progress(bytes.len() as u64, total.filter(|total| *total > 0));
    }
    Ok(bytes)
}
#[derive(Deserialize)]
pub struct PanelEnvelope<T> {
    pub data: T,
    #[serde(default)]
    pub meta: serde_json::Value,
}
/// All fields are in native memory/vault only, never a public IPC DTO or SQLite record.
#[derive(Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct ProofSession {
    pub binding_id: String,
    pub nonce: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub session_expires_at: String,
    pub scopes: Vec<String>,
    pub credential_delivery_ticket: Option<String>,
}
#[derive(Deserialize)]
pub struct TokenResponse {
    pub token_type: String,
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: String,
    pub session_id: String,
    pub installation_id: String,
    pub account_id: String,
    pub scopes: Vec<String>,
    pub proof_nonce: String,
    pub session_expires_at: String,
    pub credential_delivery_ticket: Option<String>,
}
#[derive(Deserialize)]
pub struct CredentialDelivery {
    pub provider_id: String,
    pub api_key_id: String,
    pub key_revision: String,
    pub plain_text_token: String,
    pub expires_at: Option<String>,
}
impl TokenResponse {
    pub fn into_session(self, installation: &Installation) -> Result<(String, ProofSession)> {
        if self.token_type != "Bearer"
            || self.installation_id != installation.id
            || self.expires_in == 0
            || self.expires_in > 900
            || self.access_token.is_empty()
            || self.refresh_token.is_empty()
            || self.account_id.is_empty()
            || self.account_id.len() > 128
            || self.account_id.chars().any(char::is_control)
            || [&self.access_token, &self.refresh_token, &self.proof_nonce]
                .iter()
                .any(|s| {
                    s.len() > 4096
                        || s.bytes().any(|b| {
                            !b.is_ascii() || b.is_ascii_whitespace() || b.is_ascii_control()
                        })
                })
            || self.proof_nonce.is_empty()
            || self.session_id.len() != 26
            || !self.session_id.bytes().all(|b| b.is_ascii_alphanumeric())
            || self.scopes.len() > 32
            || self
                .scopes
                .iter()
                .any(|s| s.len() > 128 || s.bytes().any(|b| !b.is_ascii() || b.is_ascii_control()))
        {
            return Err(fail("invalid_panel_response"));
        }
        let expires_at = chrono::DateTime::parse_from_rfc3339(&self.session_expires_at)
            .map_err(|_| fail("invalid_panel_response"))?
            .timestamp();
        if expires_at <= chrono::Utc::now().timestamp() {
            return Err(fail("session_expired"));
        }
        Ok((
            self.account_id,
            ProofSession {
                binding_id: self.session_id,
                nonce: self.proof_nonce,
                access_token: self.access_token,
                refresh_token: self.refresh_token,
                expires_at: chrono::Utc::now().timestamp() + self.expires_in as i64,
                session_expires_at: self.session_expires_at,
                scopes: self.scopes,
                credential_delivery_ticket: self.credential_delivery_ticket,
            },
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    struct CountingVault(AtomicUsize);
    impl Vault for CountingVault {
        fn get(&self, _: &str) -> Result<zeroize::Zeroizing<Vec<u8>>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(fail("unexpected_vault_access"))
        }
        fn put(&self, _: &str, _: &[u8]) -> Result<()> {
            panic!("production origin check must not mutate credentials")
        }
        fn delete(&self, _: &str) -> Result<()> {
            panic!("production origin check must not delete credentials")
        }
    }

    #[tokio::test]
    async fn production_panel_origin_rejects_foreign_before_credentials_or_transport() {
        let http = Http::new().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let sent = calls.clone();
        http.scripted(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(fail("unexpected_transport")) })
        }));
        let vault = CountingVault(AtomicUsize::new(0));
        let session = ProofSession {
            binding_id: "synthetic-binding".into(),
            nonce: "synthetic-nonce".into(),
            access_token: "synthetic-access".into(),
            refresh_token: "synthetic-refresh".into(),
            expires_at: 1,
            session_expires_at: "2099-01-01T00:00:00Z".into(),
            scopes: vec![],
            credential_delivery_ticket: None,
        };
        for origin in [
            "https://panel.example.test",
            "https://orivelle.powerbyte.io.evil.test",
            "https://orivelle.powerbyte.io:8443",
            "https://orivelle.powerbyte.io:443",
            "https://user@orivelle.powerbyte.io",
            "https://orivelle.powerbyte.io/path",
            "https://orivelle.powerbyte.io/?query=synthetic",
            "https://orivelle.powerbyte.io/#fragment",
            "http://orivelle.powerbyte.io",
        ] {
            let installation = Installation {
                id: "11111111-1111-4111-8111-111111111111".into(),
                origin: origin.into(),
                public_key: "synthetic-public-key".into(),
                signing_reference: "synthetic-signing".into(),
                backup_reference: "synthetic-backup".into(),
            };
            let error = http
                .panel::<serde_json::Value>(
                    origin,
                    "/api/v1/orbit/bootstrap",
                    Method::GET,
                    vec![],
                    Some((&installation, &session, &vault)),
                    true,
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, "panel_origin_not_allowed", "{origin}");
            assert_eq!(vault.0.load(Ordering::SeqCst), 0);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn production_panel_origin_rejects_unsigned_foreign_start_before_transport() {
        let http = Http::new().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let sent = calls.clone();
        http.scripted(Arc::new(move |_| {
            sent.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Ok(RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: br#"{"data":{"unexpected":true}}"#.to_vec(),
                })
            })
        }));
        assert_eq!(
            http.panel::<serde_json::Value>(
                "https://panel.example.test",
                "/api/v1/orbit/authorization-requests",
                Method::POST,
                b"{}".to_vec(),
                None,
                false,
            )
            .await
            .unwrap_err()
            .code,
            "panel_origin_not_allowed"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn production_panel_origin_allows_official_panel_and_other_inference_hosts() {
        let http = Http::new().unwrap();
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sent = calls.clone();
        http.scripted(Arc::new(move |request| {
            sent.lock().unwrap().push(request.url);
            Box::pin(async {
                Ok(RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: br#"{"data":{"ok":true}}"#.to_vec(),
                })
            })
        }));
        let value: serde_json::Value = http
            .panel(
                "https://orivelle.powerbyte.io",
                "/api/v1/orbit/bootstrap",
                Method::GET,
                vec![],
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(value["ok"], true);
        http.request(
            Method::POST,
            "https://edge.example.test/v1/images/generations",
            b"{}".to_vec(),
            vec![("Authorization".into(), "Bearer synthetic".into())],
            1024,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                "https://orivelle.powerbyte.io/api/v1/orbit/bootstrap",
                "https://edge.example.test/v1/images/generations"
            ]
        );
    }
    #[tokio::test]
    async fn update_get_follows_only_allowed_redirects_and_keeps_the_body_bound() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for (path, response) in [
                (
                    "/latest",
                    "HTTP/1.1 302 Found\r\nLocation: /asset\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                ),
                (
                    "/asset",
                    "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nprobe",
                ),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 4096];
                let count = stream.read(&mut bytes).await.unwrap();
                let headers = String::from_utf8_lossy(&bytes[..count]);
                assert!(headers.starts_with(&format!("GET {path} ")));
                assert!(!headers.to_ascii_lowercase().contains("authorization:"));
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let url = Url::parse(&format!("http://{addr}/latest")).unwrap();
        let response = Http::fixture()
            .get_update(
                url,
                "application/octet-stream",
                4,
                Duration::from_secs(1),
                |from, to| {
                    from.path() == "/latest"
                        && to.path() == "/asset"
                        && from.origin() == to.origin()
                },
                None,
            )
            .await;
        assert!(matches!(response, Err(error) if error.code == "response_too_large"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn update_progress_arrives_before_the_body_finishes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let halfway = Arc::new(tokio::sync::Notify::new());
        let release = halfway.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(120)).await;
            stream.write_all(b"abc").await.unwrap();
            release.notified().await;
            stream.write_all(b"def").await.unwrap();
        });
        let events = std::sync::Mutex::new(Vec::new());
        let progress = |received, total| {
            events.lock().unwrap().push((received, total));
            if received == 3 {
                halfway.notify_one();
            }
        };
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            Http::fixture().get_update(
                Url::parse(&format!("http://{addr}/asset")).unwrap(),
                "application/octet-stream",
                10,
                Duration::from_secs(1),
                |_, _| false,
                Some(&progress),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(response.bytes, b"abcdef");
        let events = events.into_inner().unwrap();
        assert_eq!(events.first(), Some(&(0, Some(6))));
        assert!(events.contains(&(3, Some(6))));
        assert_eq!(events.last(), Some(&(6, Some(6))));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancelling_an_update_drops_the_http_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let started = Arc::new(tokio::sync::Notify::new());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            stream.read(&mut request).await.unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc")
                .await
                .unwrap();
            let closed = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut request))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(closed, 0);
        });
        let downloads = crate::update_download::Downloads::default();
        let id = uuid();
        let download = downloads.begin(&id).unwrap();
        let progress = |_, _| started.notify_one();
        let http = Http::fixture();
        let (result, _) = tokio::join!(
            download.run(
                http.get_update(
                    Url::parse(&format!("http://{addr}/asset")).unwrap(),
                    "application/octet-stream",
                    10,
                    Duration::from_secs(1),
                    |_, _| false,
                    Some(&progress),
                ),
                Duration::from_secs(2)
            ),
            async {
                started.notified().await;
                downloads.cancel(&id).unwrap();
            }
        );
        assert!(matches!(result, Err(error) if error.code == "update_download_cancelled"));
        drop(download);
        assert!(downloads.begin(&uuid()).is_ok());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn update_stream_handles_unknown_size_limits_and_disconnects() {
        for (length, limit, expected) in [
            ("", 10, None),
            ("", 2, Some("response_too_large")),
            ("Content-Length: 6\r\n", 10, Some("update_request_failed")),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                stream.read(&mut request).await.unwrap();
                stream
                    .write_all(
                        format!("HTTP/1.1 200 OK\r\n{length}Connection: close\r\n\r\nabc")
                            .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
            let events = std::sync::Mutex::new(Vec::new());
            let progress = |received, total| events.lock().unwrap().push((received, total));
            let result = Http::fixture()
                .get_update(
                    Url::parse(&format!("http://{addr}/asset")).unwrap(),
                    "application/octet-stream",
                    limit,
                    Duration::from_secs(1),
                    |_, _| false,
                    Some(&progress),
                )
                .await;
            if let Some(code) = expected {
                assert!(matches!(result, Err(error) if error.code == code));
            } else {
                assert_eq!(result.unwrap().bytes, b"abc");
                assert_eq!(events.into_inner().unwrap().last(), Some(&(3, None)));
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn post_redirect_never_resends_credentials_or_generation() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            observed.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf).await.unwrap();
            stream.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{addr}/again\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            tokio::time::timeout(Duration::from_millis(150), listener.accept())
                .await
                .is_ok()
        });
        let response = Http::fixture()
            .request(
                Method::POST,
                &format!("http://{addr}/generate"),
                b"{}".to_vec(),
                vec![("Authorization".into(), "Bearer synthetic".into())],
                1024,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(response.status, 307);
        assert!(!server.await.unwrap());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
}
