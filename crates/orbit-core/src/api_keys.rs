//! Actor-owned API Key metadata and closed management requests.
use crate::{
    dto::ClientKind,
    error::{Result, fail},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiKeyLabel {
    pub id: String,
    pub name: String,
    pub prefix: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiKeyView {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub status: String,
    pub state: String,
    pub key_revision: String,
    pub management_revision: String,
    pub service_tier: Option<String>,
    pub force_fast: bool,
    pub service_tier_revision: u64,
    pub spending_limit_mode: String,
    pub spending_limit: Option<String>,
    pub spending_used: String,
    pub spending_active_used: String,
    pub spending_remaining: Option<String>,
    pub spending_window_minutes: Option<u64>,
    pub spending_window_used: String,
    pub spending_window_started_at: Option<String>,
    pub spending_window_expires_at: Option<String>,
    pub spending_window_resets_at: Option<String>,
    pub spending_window_status: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: Option<String>,
    pub revoked_at: Option<String>,
    pub legacy_issued: bool,
    pub selectable: bool,
}
impl ApiKeyView {
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.id)?;
        validate_revision(&self.key_revision)?;
        validate_revision(&self.management_revision)?;
        if self.name.is_empty()
            || self.name.len() > 400
            || self.name.chars().any(char::is_control)
            || self.prefix.len() > 128
            || self.prefix.chars().any(char::is_control)
            || ![
                "active",
                "spending_exhausted",
                "risk_suspended",
                "expired",
                "revoked",
                "retired",
            ]
            .contains(&self.status.as_str())
            || ![
                "active",
                "risk_suspended",
                "expired",
                "revoked",
                "retired",
                "unavailable",
            ]
            .contains(&self.state.as_str())
        {
            return Err(fail("invalid_panel_response"));
        }
        Ok(())
    }
}
pub fn validate_id(id: &str) -> Result<()> {
    if id.len() != 26 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(fail("invalid_api_key_request"));
    }
    Ok(())
}
pub fn validate_revision(revision: &str) -> Result<()> {
    if revision.len() != 64
        || !revision
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(fail("invalid_api_key_request"));
    }
    Ok(())
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyScope {
    pub connection_id: String,
    pub expected_revision: u64,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyQuery {
    pub connection_id: String,
    pub expected_revision: u64,
    #[serde(default)]
    pub search: String,
    pub status: Option<String>,
    #[serde(default = "first_page")]
    pub page: u32,
}
fn first_page() -> u32 {
    1
}
impl ApiKeyQuery {
    pub fn target(&self) -> Result<String> {
        if self.search.chars().count() > 100
            || self.search.chars().any(char::is_control)
            || self.page == 0
            || self.page > 100000
            || self.status.as_ref().is_some_and(|s| {
                ![
                    "active",
                    "spending_exhausted",
                    "risk_suspended",
                    "expired",
                    "revoked",
                    "retired",
                ]
                .contains(&s.as_str())
            })
        {
            return Err(fail("invalid_api_key_request"));
        }
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("page", &self.page.to_string());
        if !self.search.is_empty() {
            query.append_pair("search", &self.search);
        }
        if let Some(status) = &self.status {
            query.append_pair("status", status);
        }
        Ok(format!("/api/v1/orbit/api-keys?{}", query.finish()))
    }
    pub fn scope(&self) -> KeyScope {
        KeyScope {
            connection_id: self.connection_id.clone(),
            expected_revision: self.expected_revision,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyReference {
    pub connection_id: String,
    pub expected_revision: u64,
    pub api_key_id: String,
    pub key_revision: String,
}
impl KeyReference {
    pub fn scope(&self) -> KeyScope {
        KeyScope {
            connection_id: self.connection_id.clone(),
            expected_revision: self.expected_revision,
        }
    }
    pub fn validate(&self) -> Result<()> {
        validate_id(&self.api_key_id)?;
        validate_revision(&self.key_revision)
    }
}
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Create,
    Update,
    Rotate,
    Reset,
    Revoke,
    Delete,
}
impl KeyAction {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Rotate => "rotate",
            Self::Reset => "reset",
            Self::Revoke => "revoke",
            Self::Delete => "delete",
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyChanges {
    pub name: String,
    pub expires_at: Option<String>,
    pub service_tier: String,
    pub spending_limit: Option<String>,
    pub spending_limit_mode: String,
    pub spending_window_minutes: Option<u64>,
}
impl ApiKeyChanges {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 100
            || self.name.chars().any(char::is_control)
            || !["follow_client", "default", "priority", "ultrafast"]
                .contains(&self.service_tier.as_str())
            || !["cumulative", "rolling"].contains(&self.spending_limit_mode.as_str())
            || self.spending_limit.as_ref().is_some_and(|s| {
                s.is_empty() || s.len() > 32 || s.bytes().any(|c| !c.is_ascii_digit() && c != b'.')
            })
            || self
                .expires_at
                .as_ref()
                .is_some_and(|s| s.len() > 64 || s.chars().any(char::is_control))
            || (self.spending_limit_mode == "rolling"
                && ![Some(300), Some(1440), Some(10080)].contains(&self.spending_window_minutes))
            || (self.spending_limit_mode == "cumulative" && self.spending_window_minutes.is_some())
        {
            return Err(fail("invalid_api_key_request"));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyMutation {
    pub connection_id: String,
    pub expected_revision: u64,
    pub action: KeyAction,
    pub api_key_id: Option<String>,
    pub management_revision: Option<String>,
    pub changes: Option<ApiKeyChanges>,
}
impl ApiKeyMutation {
    pub fn scope(&self) -> KeyScope {
        KeyScope {
            connection_id: self.connection_id.clone(),
            expected_revision: self.expected_revision,
        }
    }
    pub fn validate(&self) -> Result<()> {
        if self.action == KeyAction::Create {
            if self.api_key_id.is_some() || self.management_revision.is_some() {
                return Err(fail("invalid_api_key_request"));
            }
        } else {
            validate_id(
                self.api_key_id
                    .as_deref()
                    .ok_or_else(|| fail("invalid_api_key_request"))?,
            )?;
            validate_revision(
                self.management_revision
                    .as_deref()
                    .ok_or_else(|| fail("invalid_api_key_request"))?,
            )?;
        }
        if matches!(self.action, KeyAction::Create | KeyAction::Update) {
            self.changes
                .as_ref()
                .ok_or_else(|| fail("invalid_api_key_request"))?
                .validate()?;
        } else if self.changes.is_some() {
            return Err(fail("invalid_api_key_request"));
        }
        Ok(())
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct KeyBindingView {
    pub client: ClientKind,
    pub api_key_id: Option<String>,
    pub name: String,
    pub key_revision: String,
    pub current: bool,
    pub claude_mapping: Option<crate::dto::ClaudeModelMapping>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct ApiKeyPage {
    pub items: Vec<ApiKeyView>,
    pub page: u32,
    pub last_page: u32,
    pub total: u64,
    pub currency: String,
    pub can_write: bool,
    pub can_reveal: bool,
    pub legacy_key: Option<ApiKeyView>,
    pub legacy_cleanup_completed: bool,
    #[serde(default)]
    pub selected: Option<ApiKeyView>,
    #[serde(default)]
    pub selected_ready: bool,
    #[serde(default)]
    pub bindings: Vec<KeyBindingView>,
    #[serde(default)]
    pub pending_operation: bool,
    #[serde(default)]
    pub cleanup_status: String,
}
#[derive(Deserialize, Serialize)]
pub struct KeyMutationResult {
    pub api_key: Option<ApiKeyView>,
    pub operation_id: String,
    pub completed: bool,
}
/// The ONLY IPC secret reply: explicit, ephemeral reveal. Never cached or broadcast.
#[derive(Serialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
pub struct RevealedApiKey {
    pub api_key_id: String,
    pub plain_text_token: String,
}
