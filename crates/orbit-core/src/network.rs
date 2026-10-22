use crate::{
    error::{Result, fail},
    identity::{Installation, ProofInput, canonical_origin, sign_proof, uuid},
    vault::Vault,
};
use futures_util::StreamExt;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::time::Duration;
use url::Url;

#[derive(Clone)]
pub struct Http {
    client: Client,
    allow_loopback: bool,
    #[cfg(test)]
    fixture: std::sync::Arc<std::sync::Mutex<Option<FixtureHandler>>>,
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
impl Http {
    pub fn new() -> Result<Self> {
        Self::build(false)
    }
    fn build(allow_loopback: bool) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(10))
            .user_agent("Orivelle-Orbit/0.1.0")
            .build()
            .map_err(|_| fail("network_unavailable"))?;
        Ok(Self {
            client,
            allow_loopback,
            #[cfg(test)]
            fixture: Default::default(),
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
            .user_agent("Orivelle-Orbit/0.1.0")
            .build()
            .map_err(|_| fail("local_tls_client_unavailable"))?;
        Ok(Self {
            client,
            allow_loopback: false,
            fixture: Default::default(),
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
        Ok(RemoteResponse {
            status,
            request_id,
            bytes,
        })
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
pub struct RemoteResponse {
    pub status: u16,
    pub request_id: Option<String>,
    pub bytes: Vec<u8>,
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
