use crate::{
    dto::ClientKind,
    error::{Result, fail},
    vault::Vault,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer, SigningKey};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use uuid::Uuid;

pub fn canonical_origin(raw: &str) -> Result<String> {
    let url = Url::parse(raw).map_err(|_| fail("invalid_panel_origin"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(fail("invalid_panel_origin"));
    }
    Ok(url.origin().ascii_serialization())
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn uuid() -> String {
    Uuid::new_v4().to_string()
}
pub fn validate_uuid(value: &str) -> Result<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| fail("invalid_identifier"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installation {
    pub id: String,
    pub origin: String,
    pub public_key: String,
    pub signing_reference: String,
    pub backup_reference: String,
}
#[derive(Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct SigningSecret {
    origin: String,
    installation_id: String,
    seed: String,
}
impl Installation {
    pub fn create(origin: &str, vault: &dyn Vault) -> Result<Self> {
        let origin = canonical_origin(origin)?;
        let id = uuid();
        let key = SigningKey::generate(&mut OsRng);
        let signing_reference = format!("installation/{id}/ed25519");
        let backup_reference = format!("installation/{id}/backup");
        let mut backup = zeroize::Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(&mut *backup);
        let seed = zeroize::Zeroizing::new(key.to_bytes());
        let secret = SigningSecret {
            origin: origin.clone(),
            installation_id: id.clone(),
            seed: URL_SAFE_NO_PAD.encode(&*seed),
        };
        let bytes = zeroize::Zeroizing::new(
            serde_json::to_vec(&secret).map_err(|_| fail("serialization_failed"))?,
        );
        vault.put(&signing_reference, &bytes)?;
        if let Err(e) = vault.put(&backup_reference, &*backup) {
            let _ = vault.delete(&signing_reference);
            return Err(e);
        }
        Ok(Self {
            id,
            origin,
            public_key: URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes()),
            signing_reference,
            backup_reference,
        })
    }
}
pub struct ProofInput<'a> {
    pub method: &'a str,
    pub request_target: &'a str,
    pub binding_id: &'a str,
    pub timestamp: i64,
    pub nonce: &'a str,
    pub request_id: &'a str,
    pub body: &'a [u8],
    pub bearer: Option<&'a str>,
}
pub fn canonical_proof(installation: &Installation, input: &ProofInput<'_>) -> Result<String> {
    validate_uuid(&installation.id)?;
    validate_uuid(input.request_id)?;
    if !input.request_target.starts_with('/')
        || input.request_target.contains('#')
        || input.method != input.method.to_ascii_uppercase()
    {
        return Err(fail("invalid_proof"));
    }
    let body_hash = sha256(input.body);
    let token_hash = input
        .bearer
        .map(|v| sha256(v.as_bytes()))
        .unwrap_or_default();
    let time = input.timestamp.to_string();
    let fields = [
        "orbit-installation-proof-v1",
        input.method,
        input.request_target,
        &installation.origin,
        &installation.id,
        input.binding_id,
        &time,
        input.nonce,
        input.request_id,
        &body_hash,
        &token_hash,
    ];
    if fields.iter().any(|v| v.contains('\n') || v.contains('\r')) {
        return Err(fail("invalid_proof"));
    }
    Ok(fields.join("\n") + "\n")
}
pub fn sign_proof(
    installation: &Installation,
    input: &ProofInput<'_>,
    vault: &dyn Vault,
) -> Result<String> {
    let secret = vault.get(&installation.signing_reference)?;
    if canonical_origin(&installation.origin)? != installation.origin
        || installation.signing_reference != format!("installation/{}/ed25519", installation.id)
    {
        return Err(fail("installation_identity_mismatch"));
    }
    let envelope: SigningSecret =
        serde_json::from_slice(&secret).map_err(|_| fail("credential_missing"))?;
    if envelope.origin != installation.origin || envelope.installation_id != installation.id {
        return Err(fail("installation_identity_mismatch"));
    }
    let decoded = zeroize::Zeroizing::new(
        URL_SAFE_NO_PAD
            .decode(&envelope.seed)
            .map_err(|_| fail("credential_missing"))?,
    );
    let bytes = zeroize::Zeroizing::new(
        <[u8; 32]>::try_from(decoded.as_slice()).map_err(|_| fail("credential_missing"))?,
    );
    let key = SigningKey::from_bytes(&bytes);
    if URL_SAFE_NO_PAD.encode(key.verifying_key().to_bytes()) != installation.public_key {
        return Err(fail("installation_identity_mismatch"));
    }
    Ok(URL_SAFE_NO_PAD.encode(
        key.sign(canonical_proof(installation, input)?.as_bytes())
            .to_bytes(),
    ))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelperBinding {
    pub id: String,
    pub client: ClientKind,
    pub origin: String,
    pub account_id: String,
    pub installation_id: String,
    pub provider_id: String,
    pub credential_revision: String,
    pub secret_reference: String,
    pub enabled: bool,
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub authorization_epoch: u64,
}
#[derive(Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
pub struct InferenceSecret {
    pub origin: String,
    pub account_id: String,
    pub installation_id: String,
    pub provider_id: String,
    pub credential_revision: String,
    pub token: String,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub authorization_epoch: u64,
}
/// No active GUI connection is consulted; the entire grant is immutable.
pub fn helper_token(
    binding: &HelperBinding,
    client: ClientKind,
    binding_id: &str,
    vault: &dyn Vault,
    now: i64,
) -> Result<zeroize::Zeroizing<String>> {
    validate_uuid(binding_id)?;
    if binding.id != binding_id
        || binding.client != client
        || !binding.enabled
        || binding.expires_at.is_some_and(|at| at <= now)
        || canonical_origin(&binding.origin)? != binding.origin
        || binding.account_id.is_empty()
        || binding.provider_id.is_empty()
        || binding.credential_revision.is_empty()
    {
        return Err(fail("binding_disabled"));
    }
    let secret = vault.get(&binding.secret_reference)?;
    let credential: InferenceSecret =
        serde_json::from_slice(&secret).map_err(|_| fail("credential_missing"))?;
    if credential.origin != binding.origin
        || credential.account_id != binding.account_id
        || credential.installation_id != binding.installation_id
        || credential.provider_id != binding.provider_id
        || credential.credential_revision != binding.credential_revision
        || credential.authorization_epoch != binding.authorization_epoch
        || credential.expires_at.is_some_and(|at| at <= now)
    {
        return Err(fail("binding_disabled"));
    }
    let value = &credential.token;
    if !value.starts_with("orv_sk_") || value.len() > 4096 || value.chars().any(char::is_whitespace)
    {
        return Err(fail("credential_missing"));
    }
    Ok(zeroize::Zeroizing::new(value.clone()))
}
pub fn authorization_epoch_reference(
    origin: &str,
    installation_id: &str,
    account_id: &str,
) -> String {
    format!(
        "authorization_epoch/{}",
        sha256(format!("{origin}\n{installation_id}\n{account_id}").as_bytes())
    )
}
pub fn bound_helper_token(
    binding: &HelperBinding,
    client: ClientKind,
    id: &str,
    store: &crate::store::Store,
    vault: &dyn Vault,
    now: i64,
) -> Result<zeroize::Zeroizing<String>> {
    crate::update_recovery::ensure_ready(&store.root)?;
    if store
        .meta(&format!(
            "installation_forgetting/{}",
            binding.installation_id
        ))?
        .as_deref()
        == Some("1")
    {
        return Err(fail("binding_disabled"));
    }
    let epoch = store
        .meta(&authorization_epoch_reference(
            &binding.origin,
            &binding.installation_id,
            &binding.account_id,
        ))?
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| fail("binding_disabled"))?;
    if epoch != binding.authorization_epoch {
        return Err(fail("binding_disabled"));
    }
    let token = helper_token(binding, client, id, vault, now)?;
    crate::update_recovery::ensure_ready(&store.root)?;
    let latest = store
        .meta(&authorization_epoch_reference(
            &binding.origin,
            &binding.installation_id,
            &binding.account_id,
        ))?
        .and_then(|s| s.parse::<u64>().ok());
    if latest != Some(epoch)
        || store
            .meta(&format!(
                "installation_forgetting/{}",
                binding.installation_id
            ))?
            .as_deref()
            == Some("1")
    {
        return Err(fail("binding_disabled"));
    }
    Ok(token)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::FakeVault;
    #[test]
    fn proof_binds_actual_bytes_origin_and_token() {
        let vault = FakeVault::default();
        let install = Installation::create("https://PANEL.example:443/", &vault).unwrap();
        let id = uuid();
        let mut input = ProofInput {
            method: "POST",
            request_target: "/api/v1/orbit/token",
            binding_id: "binding",
            timestamp: 1,
            nonce: "nonce",
            request_id: &id,
            body: b"{\"x\":1}",
            bearer: None,
        };
        let first = sign_proof(&install, &input, &vault).unwrap();
        input.body = b"{ \"x\":1}";
        assert_ne!(first, sign_proof(&install, &input, &vault).unwrap());
        input.bearer = Some("account-token");
        assert_ne!(first, sign_proof(&install, &input, &vault).unwrap());
        assert_eq!(
            canonical_proof(&install, &input).unwrap().lines().count(),
            11
        );
    }
    #[test]
    fn helper_never_uses_an_active_account_or_another_client() {
        let vault = FakeVault::default();
        let id = uuid();
        let installation_id = uuid();
        vault
            .put(
                "key-a",
                &serde_json::to_vec(&InferenceSecret {
                    origin: "https://panel.example".into(),
                    account_id: "a".into(),
                    installation_id: installation_id.clone(),
                    provider_id: "openai".into(),
                    credential_revision: "r1".into(),
                    token: "orv_sk_synthetic".into(),
                    expires_at: None,
                    authorization_epoch: 1,
                })
                .unwrap(),
            )
            .unwrap();
        let mut binding = HelperBinding {
            id: id.clone(),
            client: ClientKind::Codex,
            origin: "https://panel.example".into(),
            account_id: "a".into(),
            installation_id,
            provider_id: "openai".into(),
            credential_revision: "r1".into(),
            secret_reference: "key-a".into(),
            enabled: true,
            expires_at: None,
            authorization_epoch: 1,
        };
        assert_eq!(
            &*helper_token(&binding, ClientKind::Codex, &id, &vault, 1).unwrap(),
            "orv_sk_synthetic"
        );
        assert!(helper_token(&binding, ClientKind::Claude, &id, &vault, 1).is_err());
        binding.enabled = false;
        assert!(helper_token(&binding, ClientKind::Codex, &id, &vault, 1).is_err());
    }
}
