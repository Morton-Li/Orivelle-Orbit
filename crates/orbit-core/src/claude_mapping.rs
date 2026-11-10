//! Narrow local model mapping for the qualified Claude Desktop contract.
//! All credentials and routing permits stay inside native code.

use crate::{
    config::{ClaudeContext, Journal},
    dto::{ClaudeRoleModels, ClientKind},
    error::{Result, fail},
    identity::{HelperBinding, authorization_epoch_reference, sha256, validate_uuid},
    network::Http,
    store::Store,
    vault::Vault,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, TcpListener},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
use url::Url;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAlias {
    pub name: String,
    #[serde(rename = "labelOverride")]
    pub label_override: String,
}

const SONNET: &str = "claude-sonnet-5";
const OPUS: &str = "claude-opus-5";
const HAIKU: &str = "claude-haiku-4-5";
const DATED_HAIKU: &str = "claude-haiku-4-5-20251001";

pub fn requires_mapping(model_id: &str) -> bool {
    !(model_id.starts_with("claude-") || model_id.starts_with("anthropic/claude-"))
}

fn aliases(model_id: &str, roles: Option<&ClaudeRoleModels>) -> Vec<ModelAlias> {
    // Version 1 persisted labels are part of frozen encrypted contexts. Keep
    // this version's rule when adding future locales or plan versions.
    [
        (
            SONNET,
            "主模型",
            roles.map_or(model_id, |roles| &roles.primary),
        ),
        (
            OPUS,
            "备用主模型",
            roles.map_or(model_id, |roles| &roles.fallback),
        ),
        (
            HAIKU,
            "辅助模型",
            roles.map_or(model_id, |roles| &roles.auxiliary),
        ),
    ]
    .into_iter()
    .map(|(name, role, model_id)| ModelAlias {
        name: name.into(),
        label_override: format!("{model_id} · {role}"),
    })
    .collect()
}

/// Frozen, nonsecret configuration. No arbitrary loopback URL can construct a
/// mapping: the destination is derived from its canonical binding and port.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingPlan {
    pub version: u32,
    pub binding_id: String,
    pub profile_id: String,
    pub port: u16,
    pub upstream_model_id: String,
    pub aliases: Vec<ModelAlias>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<ClaudeRoleModels>,
}

impl MappingPlan {
    pub fn validate(&self) -> Result<()> {
        for id in [&self.binding_id, &self.profile_id] {
            validate_uuid(id)?;
            if uuid::Uuid::parse_str(id)
                .map_err(|_| fail("invalid_configuration"))?
                .to_string()
                != *id
            {
                return Err(fail("invalid_configuration"));
            }
        }
        let valid_model = |model: &str| {
            !model.is_empty() && model.len() <= 128 && model.bytes().all(|b| b.is_ascii_graphic())
        };
        if self.version != if self.roles.is_some() { 2 } else { 1 }
            || self.port == 0
            || !valid_model(&self.upstream_model_id)
            || self.roles.as_ref().is_some_and(|roles| {
                roles.primary != self.upstream_model_id || !roles.ids().into_iter().all(valid_model)
            })
            || self.aliases != aliases(&self.upstream_model_id, self.roles.as_ref())
            || chrono::DateTime::parse_from_rfc3339(&self.created_at).is_err()
        {
            return Err(fail("invalid_configuration"));
        }
        Ok(())
    }
    pub(crate) fn upstream_model(&self, alias: &str) -> Option<&str> {
        let index = match alias {
            SONNET => 0,
            OPUS => 1,
            HAIKU | DATED_HAIKU => 2,
            _ => return None,
        };
        Some(
            self.roles
                .as_ref()
                .map_or(self.upstream_model_id.as_str(), |roles| roles.ids()[index]),
        )
    }
    pub fn base_path(&self) -> String {
        format!("/orivelle-orbit/claude/{}", self.binding_id)
    }
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}{}", self.port, self.base_path())
    }
    pub(crate) fn token_reference(&self) -> String {
        format!("claude-map/{}", self.binding_id)
    }
    pub(crate) fn revision(&self) -> Result<String> {
        self.validate()?;
        Ok(sha256(
            &serde_json::to_vec(self).map_err(|_| fail("serialization_failed"))?,
        ))
    }
    pub(crate) fn route(&self) -> LocalRoute {
        LocalRoute {
            binding_id: self.binding_id.clone(),
            port: self.port,
            base_path: self.base_path(),
            aliases: self.aliases.clone(),
            created_at: self.created_at.clone(),
        }
    }
}

pub(crate) fn allowed_alias(route: &LocalRoute, requested: &str) -> bool {
    route.aliases.iter().any(|alias| alias.name == requested)
        || requested == DATED_HAIKU && route.aliases.iter().any(|alias| alias.name == HAIKU)
}

/// Public-to-the-transport metadata, never a remote destination or a token.
#[derive(Clone)]
pub(crate) struct LocalRoute {
    pub binding_id: String,
    pub port: u16,
    pub base_path: String,
    pub aliases: Vec<ModelAlias>,
    pub created_at: String,
}

/// Owned scope and mutation lease. No mutex guard may be put in this capsule.
pub(crate) struct MappingAccess {
    pub binding_id: String,
    pub route_revision: String,
    pub _lease: Box<dyn Send + Sync>,
}

/// One immutable, native-only send authority. It is deliberately not clonable,
/// serializable or printable. The lease survives until the response finishes.
pub(crate) struct ForwardPermit {
    pub access: MappingAccess,
    pub messages_url: Url,
    pub upstream_model_id: String,
    pub requested_alias: String,
    pub api_key: Zeroizing<String>,
}

#[derive(Clone, Default)]
pub(crate) struct ProtocolHeaders {
    pub anthropic_version: Option<String>,
    pub anthropic_beta: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum ForwardOutcome {
    Complete { status: u16 },
    Unconfirmed,
    NotSent,
}

pub(crate) trait MappingAuthority: Send + Sync + 'static {
    fn authenticate(&self, bearer: &[u8]) -> Result<MappingAccess>;
    fn authorize_messages(&self, access: MappingAccess, alias: String) -> Result<ForwardPermit>;
    fn before_send(&self, permit: ForwardPermit) -> Result<ForwardPermit>;
    fn observe(&self, permit: ForwardPermit, outcome: ForwardOutcome) -> Result<()>;
}

pub(crate) trait MappingServer: Send + Sync {
    fn stop(&self);
    fn is_running(&self) -> bool;
    fn inflight(&self) -> usize;
}

/// The transport module owns the concrete HTTP server. Domain lifecycle code
/// calls this signature only after reserving the exact loopback route.
pub(crate) type StartServer =
    fn(TcpListener, LocalRoute, Arc<dyn MappingAuthority>, Http) -> Result<Box<dyn MappingServer>>;

enum Slot {
    Reserved {
        plan: MappingPlan,
        listener: TcpListener,
        expires: Instant,
        confirmed: bool,
    },
    Running {
        plan: MappingPlan,
        server: Box<dyn MappingServer>,
    },
}

#[derive(Default)]
pub(crate) struct MappingManager {
    slots: Arc<Mutex<HashMap<String, Slot>>>,
}

pub(crate) struct ReservationTicket {
    slots: Weak<Mutex<HashMap<String, Slot>>>,
    binding_id: String,
}
impl Drop for ReservationTicket {
    fn drop(&mut self) {
        if let Some(slots) = self.slots.upgrade()
            && let Ok(mut slots) = slots.lock()
            && matches!(slots.get(&self.binding_id), Some(Slot::Reserved { .. }))
        {
            slots.remove(&self.binding_id);
        }
    }
}

impl MappingManager {
    pub(crate) fn reserve(
        &self,
        binding_id: &str,
        profile_id: &str,
        model_id: &str,
        roles: Option<&ClaudeRoleModels>,
    ) -> Result<MappingPlan> {
        self.reserve_for(
            binding_id,
            profile_id,
            model_id,
            roles,
            Duration::from_secs(300),
        )
    }
    fn reserve_for(
        &self,
        binding_id: &str,
        profile_id: &str,
        model_id: &str,
        roles: Option<&ClaudeRoleModels>,
        ttl: Duration,
    ) -> Result<MappingPlan> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| fail("claude_mapping_unavailable"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| fail("claude_mapping_unavailable"))?;
        let plan = MappingPlan {
            version: if roles.is_some() { 2 } else { 1 },
            binding_id: binding_id.into(),
            profile_id: profile_id.into(),
            port: listener
                .local_addr()
                .map_err(|_| fail("claude_mapping_unavailable"))?
                .port(),
            upstream_model_id: model_id.into(),
            aliases: aliases(model_id, roles),
            created_at: chrono::Utc::now().to_rfc3339(),
            roles: roles.cloned(),
        };
        plan.validate()?;
        let mut slots = self.slots.lock().map_err(|_| fail("state_unavailable"))?;
        slots.retain(
            |_, slot| !matches!(slot, Slot::Reserved{expires,confirmed:false,..} if *expires <= Instant::now()),
        );
        if slots.len() >= 8 || slots.contains_key(binding_id) {
            return Err(fail("claude_mapping_unavailable"));
        }
        slots.insert(
            binding_id.into(),
            Slot::Reserved {
                plan: plan.clone(),
                listener,
                expires: Instant::now() + ttl,
                confirmed: false,
            },
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let weak = Arc::downgrade(&self.slots);
            runtime.spawn(async move {tokio::time::sleep(ttl).await;
                if let Some(slots)=weak.upgrade() && let Ok(mut slots)=slots.lock(){
                    slots.retain(|_,slot|!matches!(slot,Slot::Reserved{expires,confirmed:false,..} if *expires<=Instant::now()));
                }
            });
        }
        Ok(plan)
    }
    pub(crate) fn ticket(&self, plan: &MappingPlan) -> ReservationTicket {
        ReservationTicket {
            slots: Arc::downgrade(&self.slots),
            binding_id: plan.binding_id.clone(),
        }
    }
    pub(crate) fn claim(&self, plan: &MappingPlan) -> Result<()> {
        let mut slots = self.slots.lock().map_err(|_| fail("state_unavailable"))?;
        match slots.get_mut(&plan.binding_id) {
            Some(Slot::Reserved {
                plan: current,
                confirmed,
                expires,
                ..
            }) if current == plan && (*confirmed || *expires > Instant::now()) => {
                *confirmed = true;
                Ok(())
            }
            _ => Err(fail("preview_expired")),
        }
    }
    pub(crate) fn activate(
        &self,
        plan: &MappingPlan,
        authority: Arc<dyn MappingAuthority>,
        http: Http,
        start: StartServer,
    ) -> Result<()> {
        plan.validate()?;
        let listener = {
            let mut slots = self.slots.lock().map_err(|_| fail("state_unavailable"))?;
            if let Some(Slot::Running {
                plan: current,
                server,
            }) = slots.get(&plan.binding_id)
                && current == plan
                && server.is_running()
            {
                return Ok(());
            }
            match slots.remove(&plan.binding_id) {
                Some(Slot::Reserved {
                    plan: current,
                    listener,
                    ..
                }) if current == *plan => listener,
                Some(Slot::Running { server, .. }) => {
                    server.stop();
                    return Err(fail("claude_mapping_unavailable"));
                }
                Some(_) => return Err(fail("claude_mapping_unavailable")),
                None => TcpListener::bind((Ipv4Addr::LOCALHOST, plan.port))
                    .map_err(|_| fail("claude_mapping_unavailable"))?,
            }
        };
        listener
            .set_nonblocking(true)
            .map_err(|_| fail("claude_mapping_unavailable"))?;
        let server = start(listener, plan.route(), authority, http)?;
        self.slots
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .insert(
                plan.binding_id.clone(),
                Slot::Running {
                    plan: plan.clone(),
                    server,
                },
            );
        Ok(())
    }
    pub(crate) fn stop(&self, binding_id: &str) {
        if let Ok(mut slots) = self.slots.lock()
            && let Some(Slot::Running { server, .. }) = slots.remove(binding_id)
        {
            server.stop();
        }
    }
    pub(crate) fn stop_running_except(&self, keep: Option<&str>) {
        if let Ok(mut slots) = self.slots.lock() {
            slots.retain(|id, slot| {
                if matches!(slot, Slot::Running { .. }) && Some(id.as_str()) != keep {
                    if let Slot::Running { server, .. } = slot {
                        server.stop();
                    }
                    false
                } else {
                    true
                }
            });
        }
    }
    pub(crate) fn inflight(&self) -> usize {
        self.slots
            .lock()
            .map(|slots| {
                slots
                    .values()
                    .map(|slot| match slot {
                        Slot::Running { server, .. } => server.inflight(),
                        _ => 0,
                    })
                    .sum()
            })
            .unwrap_or(1)
    }
}

impl Drop for MappingManager {
    fn drop(&mut self) {
        if let Ok(slots) = self.slots.lock() {
            for slot in slots.values() {
                if let Slot::Running { server, .. } = slot {
                    server.stop();
                }
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MappingRecord {
    pub journal_id: String,
    pub context: ClaudeContext,
    pub committed: bool,
    #[serde(default)]
    pub retained_until: String,
}

impl MappingRecord {
    pub(crate) fn plan(&self) -> Result<&MappingPlan> {
        let plan = self
            .context
            .mapping
            .as_ref()
            .ok_or_else(|| fail("binding_disabled"))?;
        plan.validate()?;
        let mut plain = self.context.clone();
        plain.scope_revision.clear();
        if self.context.binding_id != plan.binding_id
            || self.context.profile_id != plan.profile_id
            || self.context.selected_model_id != plan.upstream_model_id
            || self.context.scope_revision
                != sha256(&serde_json::to_vec(&plain).map_err(|_| fail("serialization_failed"))?)
        {
            return Err(fail("binding_disabled"));
        }
        Ok(plan)
    }
    pub(crate) fn ready(&self, store: &Store) -> Result<()> {
        self.plan()?;
        match store.get::<Journal>("journal", &self.journal_id)? {
            Some(journal)
                if journal.client == ClientKind::Claude
                    && journal.phase == "committed"
                    && journal.account_id == self.context.account_id
                    && journal.installation.id == self.context.installation_id
                    && journal.installation.origin == self.context.panel_origin =>
            {
                Ok(())
            }
            None if self.committed => Ok(()), // active mapping outlives pruned encrypted backups
            _ => Err(fail("binding_disabled")),
        }
    }
}

#[derive(Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(deny_unknown_fields)]
struct LocalSecret {
    token: String,
    binding_sha256: String,
    context_revision: String,
    route_revision: String,
    journal_id: String,
}

fn binding_digest(binding: &HelperBinding) -> Result<String> {
    let mut binding = binding.clone();
    binding.enabled = true;
    Ok(sha256(
        &serde_json::to_vec(&binding).map_err(|_| fail("serialization_failed"))?,
    ))
}

fn check_binding(
    binding: &HelperBinding,
    record: &MappingRecord,
    store: &Store,
    now: i64,
) -> Result<()> {
    let plan = record.plan()?;
    crate::update_recovery::ensure_ready(&store.root)?;
    record.ready(store)?;
    if binding.client != ClientKind::Claude
        || binding.id != plan.binding_id
        || !binding.enabled
        || binding.origin != record.context.panel_origin
        || binding.account_id != record.context.account_id
        || binding.installation_id != record.context.installation_id
        || binding.provider_id != record.context.provider_id
        || binding.credential_revision != record.context.credential_revision
        || binding.secret_reference != record.context.credential_reference
        || binding.authorization_epoch != record.context.authorization_epoch
        || binding.expires_at != record.context.credential_expires_at
        || binding.expires_at.is_some_and(|at| at <= now)
        || store
            .meta(&format!(
                "installation_forgetting/{}",
                binding.installation_id
            ))?
            .as_deref()
            == Some("1")
        || store
            .meta(&authorization_epoch_reference(
                &binding.origin,
                &binding.installation_id,
                &binding.account_id,
            ))?
            .and_then(|v| v.parse::<u64>().ok())
            != Some(binding.authorization_epoch)
        || store
            .installation(&binding.origin)?
            .is_none_or(|i| i.id != binding.installation_id)
    {
        return Err(fail("binding_disabled"));
    }
    Ok(())
}

pub(crate) fn mapping_binding(record: &MappingRecord, store: &Store) -> Result<HelperBinding> {
    let binding = store
        .get::<HelperBinding>("binding", &record.context.binding_id)?
        .ok_or_else(|| fail("binding_disabled"))?;
    check_binding(&binding, record, store, chrono::Utc::now().timestamp())?;
    Ok(binding)
}

pub(crate) fn ensure_local_secret(
    record: &MappingRecord,
    binding: &HelperBinding,
    vault: &dyn Vault,
) -> Result<()> {
    let plan = record.plan()?;
    let reference = plan.token_reference();
    let secret = match vault.get_optional(&reference)? {
        Some(bytes) => {
            serde_json::from_slice::<LocalSecret>(&bytes).map_err(|_| fail("credential_missing"))?
        }
        None => {
            let mut random = Zeroizing::new([0u8; 32]);
            OsRng.fill_bytes(&mut *random);
            let secret = LocalSecret {
                token: format!("orbt_map_{}", URL_SAFE_NO_PAD.encode(*random)),
                binding_sha256: binding_digest(binding)?,
                context_revision: record.context.scope_revision.clone(),
                route_revision: plan.revision()?,
                journal_id: record.journal_id.clone(),
            };
            let bytes = Zeroizing::new(
                serde_json::to_vec(&secret).map_err(|_| fail("serialization_failed"))?,
            );
            #[cfg(target_os = "macos")]
            vault.create_claude_mapping_secret(&record.context.binding_id, &bytes)?;
            #[cfg(not(target_os = "macos"))]
            vault.put(&reference, &bytes)?;
            secret
        }
    };
    validate_secret(&secret, record, binding)
}

pub(crate) enum LocalSecretPresence {
    Absent,
    Authenticated,
}

pub(crate) fn verify_context_secret(
    record: &MappingRecord,
    vault: &dyn Vault,
) -> Result<LocalSecretPresence> {
    let context = &record.context;
    let binding = HelperBinding {
        id: context.binding_id.clone(),
        client: ClientKind::Claude,
        origin: context.panel_origin.clone(),
        account_id: context.account_id.clone(),
        installation_id: context.installation_id.clone(),
        provider_id: context.provider_id.clone(),
        credential_revision: context.credential_revision.clone(),
        secret_reference: context.credential_reference.clone(),
        enabled: true,
        expires_at: context.credential_expires_at,
        authorization_epoch: context.authorization_epoch,
    };
    let Some(bytes) = vault.get_optional(&record.plan()?.token_reference())? else {
        return Ok(LocalSecretPresence::Absent);
    };
    let secret: LocalSecret =
        serde_json::from_slice(&bytes).map_err(|_| fail("credential_missing"))?;
    validate_secret(&secret, record, &binding)?;
    Ok(LocalSecretPresence::Authenticated)
}

fn validate_secret(
    secret: &LocalSecret,
    record: &MappingRecord,
    binding: &HelperBinding,
) -> Result<()> {
    if secret.binding_sha256 != binding_digest(binding)?
        || secret.context_revision != record.context.scope_revision
        || secret.route_revision != record.plan()?.revision()?
        || secret.journal_id != record.journal_id
        || secret.token.len() != 52
        || !secret.token.starts_with("orbt_map_")
        || secret.token.bytes().any(|b| !b.is_ascii_graphic())
    {
        return Err(fail("credential_missing"));
    }
    Ok(())
}

/// Only the installed native helper calls this API. No IPC exposes it.
pub fn mapped_helper_token(
    binding_id: &str,
    store: &Store,
    vault: &dyn Vault,
    now: i64,
) -> Result<Zeroizing<String>> {
    validate_uuid(binding_id)?;
    let record = store
        .get::<MappingRecord>("claude-mapping", binding_id)?
        .ok_or_else(|| fail("binding_disabled"))?;
    let binding = store
        .get::<HelperBinding>("binding", binding_id)?
        .ok_or_else(|| fail("binding_disabled"))?;
    check_binding(&binding, &record, store, now)?;
    let bytes = vault.get(&record.plan()?.token_reference())?;
    let secret: LocalSecret =
        serde_json::from_slice(&bytes).map_err(|_| fail("credential_missing"))?;
    validate_secret(&secret, &record, &binding)?;
    let latest = store
        .get::<HelperBinding>("binding", binding_id)?
        .ok_or_else(|| fail("binding_disabled"))?;
    check_binding(
        &latest,
        &record,
        store,
        chrono::Utc::now().timestamp().max(now),
    )?;
    if binding_digest(&latest)? != binding_digest(&binding)? {
        return Err(fail("binding_disabled"));
    }
    Ok(Zeroizing::new(secret.token.clone()))
}

/// An executable helper cannot use its legacy mode to extract the real key of
/// a mapped binding. Native configuration/forwarding use bound_helper_token
/// directly after their own scope qualification.
pub fn unmapped_helper_token(
    binding: &HelperBinding,
    client: ClientKind,
    binding_id: &str,
    store: &Store,
    vault: &dyn Vault,
    now: i64,
) -> Result<Zeroizing<String>> {
    if client == ClientKind::Claude
        && store
            .get::<MappingRecord>("claude-mapping", binding_id)?
            .is_some()
    {
        return Err(fail("binding_disabled"));
    }
    crate::identity::bound_helper_token(binding, client, binding_id, store, vault, now)
}

pub(crate) fn authenticate_local(
    record: &MappingRecord,
    bearer: &[u8],
    store: &Store,
    vault: &dyn Vault,
) -> Result<()> {
    let expected = mapped_helper_token(
        &record.context.binding_id,
        store,
        vault,
        chrono::Utc::now().timestamp(),
    )?;
    if bearer.len() != expected.len() {
        return Err(fail("binding_disabled"));
    }
    let different = bearer
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b));
    if different != 0 {
        return Err(fail("binding_disabled"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mapping_reservations_expire_without_another_preview_and_drop_on_discard() {
        let manager = MappingManager::default();
        let plan = manager
            .reserve_for(
                &crate::identity::uuid(),
                &crate::identity::uuid(),
                "gpt-6-luna",
                None,
                Duration::from_millis(20),
            )
            .unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, plan.port)).is_ok());
        let confirmed = manager
            .reserve_for(
                &crate::identity::uuid(),
                &crate::identity::uuid(),
                "gpt-6-luna",
                None,
                Duration::from_millis(20),
            )
            .unwrap();
        let ticket = manager.ticket(&confirmed);
        manager.claim(&confirmed).unwrap();
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            TcpListener::bind((Ipv4Addr::LOCALHOST, confirmed.port)).is_err(),
            "accepted confirmation preserves its reservation across slow qualification"
        );
        drop(ticket);
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, confirmed.port)).is_ok());
        let plan = manager
            .reserve(
                &crate::identity::uuid(),
                &crate::identity::uuid(),
                "gpt-6-luna",
                None,
            )
            .unwrap();
        let ticket = manager.ticket(&plan);
        drop(ticket);
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, plan.port)).is_ok());
        for _ in 0..8 {
            manager
                .reserve(
                    &crate::identity::uuid(),
                    &crate::identity::uuid(),
                    "gpt-6-luna",
                    None,
                )
                .unwrap();
        }
        assert!(
            manager
                .reserve(
                    &crate::identity::uuid(),
                    &crate::identity::uuid(),
                    "gpt-6-luna",
                    None,
                )
                .is_err()
        );
    }

    #[test]
    fn mapping_plan_reserves_exact_local_endpoint_and_bounded_role_whitelist() {
        let manager = MappingManager::default();
        let binding = crate::identity::uuid();
        let profile = crate::identity::uuid();
        let plan = manager
            .reserve(&binding, &profile, "gpt-6-luna", None)
            .unwrap();
        assert_eq!(
            plan.base_url(),
            format!(
                "http://127.0.0.1:{}/orivelle-orbit/claude/{binding}",
                plan.port
            )
        );
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, plan.port)).is_err());
        let mut route = plan.route();
        for name in [SONNET, OPUS, HAIKU, DATED_HAIKU] {
            assert!(allowed_alias(&route, name));
        }
        for name in [
            "gpt-6-luna",
            "claude-sonnet-unknown",
            "claude-haiku-4-5-20990101",
            "claude-fable-5",
        ] {
            assert!(!allowed_alias(&route, name));
        }
        route.aliases.retain(|alias| alias.name != HAIKU);
        assert!(!allowed_alias(&route, DATED_HAIKU));
        let mut tampered = plan.clone();
        tampered.aliases[0].label_override = "Claude Sonnet".into();
        assert!(tampered.validate().is_err());
        drop(manager);
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, plan.port)).is_ok());
    }
}
