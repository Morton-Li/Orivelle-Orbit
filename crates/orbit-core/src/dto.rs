pub use crate::error::PublicError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Codex,
    Claude,
}
impl ClientKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Money {
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charge {
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub request_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetView {
    pub available: Money,
    pub limit: Option<Money>,
    pub mode: String,
    pub period: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountView {
    pub id: String,
    pub label: String,
    pub email: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
    pub member: bool,
    pub wallet: Option<Money>,
    pub budget: Option<BudgetView>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionView {
    pub status: String,
    pub panel_origin: Option<String>,
    pub account: Option<AccountView>,
    pub inference_credential_status: String,
}
impl Default for SessionView {
    fn default() -> Self {
        Self {
            status: "logged_out".into(),
            panel_origin: None,
            account: None,
            inference_credential_status: "unavailable".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionView {
    pub id: String,
    pub label: String,
    pub panel_origin: String,
    pub account_id: String,
    pub node_id: Option<String>,
    pub node_label: Option<String>,
    pub edge_base_url: Option<String>,
    pub provider_id: String,
    pub revision: u64,
    pub credential_revision: Option<String>,
    pub permission_state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeView {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub available: bool,
    pub maximum_request_bytes: u64,
    pub capabilities: Vec<String>,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientEvidence {
    pub kind: String,
    pub observed_at: String,
    pub configuration_revision: String,
    pub client_version: String,
    pub account_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub node_id: String,
    pub model_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientView {
    pub client: ClientKind,
    pub installed: bool,
    pub display_name: String,
    pub version: Option<String>,
    pub bundle_id: Option<String>,
    pub app_path: Option<String>,
    pub config_paths: Vec<String>,
    pub platform_supported: bool,
    pub managed: String,
    #[serde(default)]
    pub configuration_access: ConfigurationAccess,
    #[serde(default)]
    pub configuration_access_revision: Option<String>,
    pub configuration_state: String,
    pub loaded_state: String,
    pub connection_state: String,
    pub configuration_revision: Option<String>,
    pub loaded_evidence: Option<ClientEvidence>,
    pub connection_evidence: Option<ClientEvidence>,
    pub manual_steps: Vec<String>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigurationAccess {
    Allowed,
    Blocked,
    #[default]
    Unverified,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceRate {
    pub meter: String,
    pub unit_size: u64,
    pub unit_price: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceView {
    pub currency: Option<String>,
    pub source: String,
    pub status: String,
    pub rates: Vec<PriceRate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageParameterConstraints {
    pub max_results: u8,
    pub max_inputs: u8,
    pub maximum_request_bytes: u64,
    pub prompt_maximum_bytes: u64,
    pub qualities: Vec<String>,
    pub sizes: Vec<String>,
    pub backgrounds: Vec<String>,
    pub verified: bool,
    #[serde(default)]
    pub contract: Option<String>,
    #[serde(default)]
    pub output_formats: Vec<String>,
    #[serde(default)]
    pub output_compression: bool,
    #[serde(default)]
    pub complexities: Vec<String>,
    #[serde(default)]
    pub custom_size: Option<ImageSizeConstraints>,
    #[serde(default)]
    pub request_timeout_seconds: Option<u64>,
}
impl ImageParameterConstraints {
    /// New product tasks expose only controls established on the Codex route.
    /// Keep the serializable parameter types intact for historical snapshots.
    pub fn available_controls(&self) -> Self {
        let mut controls = self.clone();
        controls.max_results = controls.max_results.min(1);
        controls.max_inputs = controls.max_inputs.min(5);
        controls.qualities.clear();
        controls.sizes.clear();
        controls
            .backgrounds
            .retain(|value| matches!(value.as_str(), "opaque" | "transparent"));
        controls.output_formats.clear();
        controls.output_compression = false;
        controls.complexities.clear();
        controls.custom_size = None;
        controls
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSizeConstraints {
    pub multiple: u32,
    pub min_edge: u32,
    pub max_edge: u32,
    pub min_pixels: u64,
    pub max_pixels: u64,
    pub max_aspect_ratio: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase", deny_unknown_fields)]
pub enum CodexCatalogBindingView {
    Declared {
        verified: bool,
        profile_id: String,
        cli_version: String,
        source_sha256: String,
        template_slug: String,
    },
    Unavailable {
        verified: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReasoningPolicyView {
    pub state: String,
    pub capability_status: String,
    pub supported_efforts: Vec<String>,
    pub allowed_efforts: Option<Vec<String>>,
    pub default_effort: Option<String>,
    pub revision: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelView {
    pub id: String,
    pub label: String,
    pub provider_id: String,
    pub revision: String,
    pub category: String,
    pub capabilities: Vec<String>,
    pub execution_profile: Option<String>,
    pub image_billing_mode: Option<String>,
    pub reasoning_efforts: Vec<String>,
    #[serde(default)]
    pub reasoning: Option<ReasoningPolicyView>,
    #[serde(default)]
    pub codex_catalog_binding: Option<CodexCatalogBindingView>,
    pub support_status: String,
    pub image_parameters: Option<ImageParameterConstraints>,
    pub sales_price: Option<PriceView>,
    pub official_price: Option<PriceView>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInput {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub preview_url: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub n: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_compression: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complexity: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImageResultParameters {
    pub quality: Option<String>,
    pub size: Option<String>,
    pub background: Option<String>,
    pub output_format: Option<String>,
    pub observed_model: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskView {
    pub id: String,
    pub account_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub credential_revision: String,
    pub node_id: String,
    pub kind: String,
    pub requested_model: String,
    pub prompt: String,
    pub parameters: ImageParameters,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_parameters: Option<ImageResultParameters>,
    pub state: String,
    #[serde(default)]
    pub local_save_available: bool,
    pub started_at: Option<String>,
    pub request_id: Option<String>,
    pub result_state: String,
    pub outcome: Option<String>,
    pub charge_state: String,
    pub charges: Vec<Charge>,
    pub image_ids: Vec<String>,
    pub error: Option<PublicError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryImage {
    pub id: String,
    pub task_id: String,
    pub account_id: String,
    pub created_at: String,
    pub requested_model: String,
    pub observed_model: Option<String>,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub preview_url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreOwner {
    pub account_id: String,
    pub account_label: String,
    pub panel_origin: String,
    pub installation_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupGroup {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BackupKind>,
    pub client: ClientKind,
    #[serde(flatten)]
    pub owner: RestoreOwner,
    pub created_at: String,
    pub expires_at: String,
    pub file_count: u32,
    pub state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    pub theme: String,
    pub locale: String,
    pub close_behavior: String,
    pub image_disk_limit_bytes: u64,
    pub preview_cache_limit_bytes: u64,
    pub backup_retention_days: u32,
    pub backup_max_groups: u32,
    pub check_updates_on_start: bool,
}
impl Default for SettingsView {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            locale: "zh-CN".into(),
            close_behavior: "keep_running".into(),
            image_disk_limit_bytes: 2 * 1024 * 1024 * 1024,
            preview_cache_limit_bytes: 128 * 1024 * 1024,
            backup_retention_days: 30,
            backup_max_groups: 5,
            check_updates_on_start: false,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapView {
    pub schema_version: u32,
    pub state_revision: u64,
    pub source: String,
    pub platform: Option<String>,
    pub pending_authorization: Option<LoginStart>,
    pub maintenance_active: bool,
    pub session: SessionView,
    pub connections: Vec<ConnectionView>,
    pub nodes: Vec<NodeView>,
    pub active_connection_id: Option<String>,
    pub clients: Vec<ClientView>,
    pub settings: SettingsView,
    pub tasks: Vec<TaskView>,
    pub image_history: Vec<HistoryImage>,
    pub backups: Vec<BackupGroup>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewContext {
    pub account_id: String,
    pub account_label: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub credential_revision: String,
    pub node_id: String,
    pub node_label: String,
    pub model_revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreSource {
    pub backup_group_id: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<BackupKind>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupKind {
    Switch,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeRoleModels {
    pub primary: String,
    pub fallback: String,
    pub auxiliary: String,
}
impl ClaudeRoleModels {
    pub(crate) fn ids(&self) -> [&str; 3] {
        [&self.primary, &self.fallback, &self.auxiliary]
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaudeRestoreCoordination {
    #[serde(rename = "claude_main_preferences_v1")]
    MainPreferencesV1,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPreview {
    pub id: String,
    pub client: ClientKind,
    pub operation: String,
    pub target_version: Option<String>,
    pub paths: Vec<String>,
    pub model_id: Option<String>,
    pub context: Option<PreviewContext>,
    pub restore_owner: Option<RestoreOwner>,
    #[serde(default)]
    pub restore_source: Option<RestoreSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore_coordination: Option<ClaudeRestoreCoordination>,
    pub impact: Vec<String>,
    pub redacted_diff: String,
    pub expires_at: String,
    pub can_apply: bool,
    pub can_export_import: bool,
    pub blocking_reason: Option<String>,
    #[serde(default)]
    pub codex_catalog: Option<CodexCatalogPreview>,
    #[serde(default)]
    pub codex_runtime: Option<CodexRuntimeView>,
    #[serde(default)]
    pub codex_configuration_target: Option<CodexConfigurationTargetView>,
    #[serde(default)]
    pub claude_configuration_target: Option<ClaudeConfigurationTargetView>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexConfigurationTargetView {
    pub cli_version: String,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfigurationTargetView {
    pub app_version: String,
    pub revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexCatalogPreview {
    pub profile_id: String,
    pub cli_version: String,
    pub source_sha256: String,
    pub sha256: String,
    pub scope_revision: String,
    pub path: String,
    pub bytes: u64,
    pub aliases: Vec<String>,
    pub default_alias: String,
    pub default_reasoning_effort: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexRuntimeView {
    pub state: String,
    pub cli_version: Option<String>,
    pub revision: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigResult {
    pub client: ClientKind,
    pub state: String,
    pub loaded_state: String,
    pub backup_group_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginStart {
    pub request_id: String,
    pub expires_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorizationEvent {
    pub request_id: String,
    pub state: String,
    pub error: Option<PublicError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsPreview {
    pub id: String,
    pub text: String,
    pub contains_images: bool,
    pub uploaded: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateView {
    pub status: String,
    pub current_version: String,
    pub version: Option<String>,
    pub reason: Option<String>,
}
