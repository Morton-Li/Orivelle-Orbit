/** Public IPC contract. Credentials, raw HTTP, SQL and arbitrary file access are intentionally absent. */
export type ClientKind = "codex" | "claude";
export type ClaudeRoleModels = {
  primary: string;
  fallback: string;
  auxiliary: string;
};
export type Theme = "system" | "light" | "dark";
export type Locale = "zh-CN" | "en";
export const CAPABILITIES = {
  responses: "openai.responses",
  messages: "anthropic.messages",
  imageGeneration: "openai.images.generations",
  imageEdit: "openai.images.edits",
} as const;
export type PublicError = { code: string; message: string; retryable: boolean };
export type Money = {
  amount: string | null;
  currency: string | null;
  status: "known" | "unknown" | "pending" | "conflict" | "unlimited";
};
export type ResultState =
  "historical" | "confirmed" | "incomplete" | "conflict" | "unconfirmed";
export type ChargeState =
  | "incomplete"
  | "reconciliation_pending"
  | "charged"
  | "recorded"
  | "pending"
  | "unbilled"
  | "unavailable";
export type Charge = {
  amount: string | null;
  currency: string | null;
  request_id: string | null;
};
export type PricingMeter =
  | "input"
  | "cached_input"
  | "cache_write"
  | "output"
  | "reasoning"
  | "request"
  | "image"
  | "text_input"
  | "cached_text_input"
  | "image_input"
  | "cached_image_input"
  | "image_output";
export type PriceView = {
  currency: string | null;
  source:
    | "sales"
    | "official"
    | "configured_reference"
    | "operator_estimate"
    | "unknown";
  status:
    "configured" | "partial" | "unpriced_free" | "unavailable" | "unknown";
  rates: {
    meter: PricingMeter;
    unit_size: number;
    unit_price: string | null;
  }[];
};
export type ImageParameterConstraints = {
  max_results: number;
  max_inputs: number;
  maximum_request_bytes: number;
  prompt_maximum_bytes: number;
  qualities: ("auto" | "low" | "medium" | "high")[];
  sizes: string[];
  backgrounds: ("auto" | "opaque" | "transparent")[];
  verified: boolean;
};
export type AccountView = {
  id: string;
  label: string;
  email: string | null;
  updated_at: string | null;
  member: boolean;
  wallet: Money | null;
  budget: {
    available: Money;
    limit: Money | null;
    mode: string;
    period: string | null;
  } | null;
};
export type SessionView = {
  status:
    | "logged_out"
    | "authorizing"
    | "authenticated"
    | "refreshing"
    | "expired"
    | "revoked"
    | "vault_unavailable";
  panel_origin: string | null;
  account: AccountView | null;
  inference_credential_status:
    "unavailable" | "ready" | "permission_denied" | "revoked";
};
export type ConnectionView = {
  id: string;
  label: string;
  panel_origin: string;
  account_id: string;
  node_id: string | null;
  node_label: string | null;
  edge_base_url: string | null;
  provider_id: string;
  revision: number;
  credential_revision: string | null;
  permission_state: "allowed" | "readonly" | "unavailable";
};
export type NodeView = {
  id: string;
  label: string;
  base_url: string;
  available: boolean;
  maximum_request_bytes: number;
  capabilities: string[];
  revision: string;
};
export type ClientView = {
  client: ClientKind;
  installed: boolean;
  display_name: string;
  version: string | null;
  bundle_id: string | null;
  app_path: string | null;
  config_paths: string[];
  platform_supported: boolean;
  managed: "yes" | "no" | "unknown";
  configuration_access: "allowed" | "blocked" | "unverified";
  configuration_access_revision: string | null;
  configuration_state: "not_configured" | "saved" | "conflict" | "unsupported";
  loaded_state: "unconfirmed" | "restart_required" | "confirmed";
  connection_state: "not_tested" | "passed" | "failed";
  configuration_revision: string | null;
  loaded_evidence: ClientEvidence | null;
  connection_evidence: ClientEvidence | null;
  manual_steps: string[];
};
export type ClientEvidence = {
  kind:
    | "client_runtime_observation"
    | "fake_gateway_request"
    | "authorized_gateway_record";
  observed_at: string;
  configuration_revision: string;
  client_version: string;
  account_id: string;
  connection_id: string;
  connection_revision: number;
  node_id: string;
  model_id: string;
};
export type CodexCatalogBinding =
  | { state: "unavailable"; verified: false }
  | {
      state: "declared";
      verified: false;
      profile_id: string;
      cli_version: string;
      source_sha256: string;
      template_slug: string;
    };
export type ModelReasoning = {
  state: "unrestricted" | "restricted";
  capability_status: "supported" | "unknown" | "not_applicable";
  supported_efforts: string[];
  allowed_efforts: string[] | null;
  default_effort: string | null;
  revision: number | null;
};
export type ModelView = {
  id: string;
  label: string;
  provider_id: string;
  revision: string;
  category: "text" | "image";
  capabilities: string[];
  execution_profile: string | null;
  image_billing_mode: "image_count" | "image_tokens_v1" | null;
  reasoning_efforts: string[];
  reasoning: ModelReasoning | null;
  codex_catalog_binding: CodexCatalogBinding | null;
  support_status: "verified" | "unverified" | "unsupported";
  image_parameters: ImageParameterConstraints | null;
  sales_price: PriceView | null;
  official_price: PriceView | null;
};
export type ImageInput = {
  id: string;
  name: string;
  mime: "image/png" | "image/jpeg" | "image/webp";
  bytes: number;
  width: number;
  height: number;
  preview_url: string | null;
};
export type ImageParameters = {
  n?: number;
  quality?: "auto" | "low" | "medium" | "high";
  size?: string;
  background?: "auto" | "opaque" | "transparent";
};
export type ImageInputsEvent = {
  account_id: string;
  connection_id: string;
  connection_revision: number;
  inputs: ImageInput[];
};
export type ScopedErrorEvent = {
  error: PublicError;
  account_id: string | null;
  connection_id: string | null;
  connection_revision: number | null;
};
export type TaskView = {
  id: string;
  account_id: string;
  connection_id: string;
  connection_revision: number;
  credential_revision: string;
  node_id: string;
  kind: "generation" | "edit";
  requested_model: string;
  prompt: string;
  parameters: ImageParameters;
  state:
    | "draft"
    | "queued"
    | "sending"
    | "waiting"
    | "saving"
    | "completed"
    | "save_failed"
    | "cancelled"
    | "failed"
    | "unconfirmed";
  started_at: string | null;
  request_id: string | null;
  result_state: ResultState;
  outcome: string | null;
  charge_state: ChargeState;
  charges: Charge[];
  image_ids: string[];
  local_save_available: boolean;
  error: PublicError | null;
};
export type HistoryImage = {
  id: string;
  task_id: string;
  account_id: string;
  created_at: string;
  requested_model: string;
  observed_model: string | null;
  mime: "image/png" | "image/jpeg" | "image/webp";
  width: number;
  height: number;
  bytes: number;
  preview_url: string | null;
};
export type RestoreOwner = {
  account_id: string;
  account_label: string;
  panel_origin: string;
  installation_id: string;
};
export type BackupGroup = RestoreOwner & {
  id: string;
  kind?: "switch" | "manual";
  client: ClientKind;
  created_at: string;
  expires_at: string;
  file_count: number;
  state: "available" | "conflict" | "unavailable";
};
export type SettingsView = {
  theme: Theme;
  locale: Locale;
  close_behavior: "platform_default" | "keep_running" | "quit";
  image_disk_limit_bytes: number;
  preview_cache_limit_bytes: number;
  backup_retention_days: number;
  backup_max_groups: number;
  check_updates_on_start: boolean;
};
export type BootstrapView = {
  schema_version: 1;
  state_revision: number;
  source: "local" | "remote" | "synthetic";
  platform: "macos" | "windows" | null;
  pending_authorization: LoginStart | null;
  maintenance_active: boolean;
  session: SessionView;
  connections: ConnectionView[];
  nodes: NodeView[];
  active_connection_id: string | null;
  clients: ClientView[];
  settings: SettingsView;
  tasks: TaskView[];
  image_history: HistoryImage[];
  backups: BackupGroup[];
};
type ConfigPreviewBase = {
  id: string;
  client: ClientKind;
  target_version: string | null;
  paths: string[];
  impact: string[];
  redacted_diff: string;
  expires_at: string;
  can_apply: boolean;
  can_export_import: boolean;
  blocking_reason: string | null;
  claude_configuration_target: {
    app_version: string;
    revision: string;
  } | null;
  codex_configuration_target: {
    cli_version: string;
    // Local configuration qualification, independent of runtime/load evidence.
    revision: string;
  } | null;
  codex_catalog: {
    profile_id: string;
    cli_version: string;
    source_sha256: string;
    sha256: string;
    scope_revision: string;
    path: string;
    bytes: number;
    aliases: string[];
    default_alias: string;
    default_reasoning_effort: string;
  } | null;
  codex_runtime: {
    state: "unverified" | "verified";
    cli_version: string | null;
    revision: string | null;
  } | null;
};
export type ClaudeRestoreCoordination = "claude_main_preferences_v1";
export type ConfigPreview = ConfigPreviewBase &
  (
    | {
        operation: "apply";
        model_id: string;
        context: {
          account_id: string;
          account_label: string;
          connection_id: string;
          connection_revision: number;
          credential_revision: string;
          node_id: string;
          node_label: string;
          model_revision: string;
        };
        restore_owner: null;
        restore_source: null;
      }
    | {
        operation: "restore";
        model_id: null;
        context: null;
        restore_owner: RestoreOwner;
        restore_source: {
          backup_group_id: string;
          expires_at: string;
          kind?: BackupGroup["kind"];
        };
        restore_coordination?: ClaudeRestoreCoordination | null;
      }
  );
export type ConfigResult = {
  client: ClientKind;
  state: "saved" | "restored" | "conflict";
  loaded_state: "restart_required" | "unconfirmed";
  backup_group_id: string | null;
};
export type LoginStart = { request_id: string; expires_at: string };
export type AuthorizationEvent = {
  request_id: string;
  state: "connected" | "denied" | "expired" | "failed" | "cancelled";
  error: PublicError | null;
};
export type DiagnosticsPreview = {
  id: string;
  text: string;
  contains_images: false;
  uploaded: false;
};
export type UpdateView = {
  status:
    | "disabled"
    | "up_to_date"
    | "available"
    | "downloading"
    | "verified"
    | "blocked"
    | "failed";
  current_version: string;
  version: string | null;
  reason: string | null;
};
export interface IpcCommands {
  bootstrap: { args: Record<string, never>; result: BootstrapView };
  start_login: { args: Record<string, never>; result: LoginStart };
  cancel_login: { args: { request_id: string }; result: BootstrapView };
  refresh_account: {
    args: { connection_id: string; expected_revision: number };
    result: BootstrapView;
  };
  logout: { args: { connection_id: string }; result: BootstrapView };
  select_node: {
    args: { connection_id: string; node_id: string; expected_revision: number };
    result: BootstrapView;
  };
  list_models: { args: { connection_id: string }; result: ModelView[] };
  preview_client_config: {
    args: {
      connection_id: string;
      expected_revision: number;
      client: ClientKind;
      claude_models?: ClaudeRoleModels;
    };
    result: ConfigPreview;
  };
  apply_client_config: {
    args: { preview_id: string; confirmed: true };
    result: ConfigResult;
  };
  export_claude_import: {
    args: { preview_id: string; confirmed: true };
    result: { saved: boolean };
  };
  preview_restore: {
    args: {
      backup_group_id: string;
      coordination?: ClaudeRestoreCoordination;
    };
    result: ConfigPreview;
  };
  backup_client_config: { args: { client: ClientKind }; result: BootstrapView };
  restore_client_config: {
    args: { preview_id: string; confirmed: true };
    result: ConfigResult;
  };
  select_image_inputs: {
    args: { connection_id: string };
    result: ImageInput[];
  };
  create_image_task: {
    args: {
      connection_id: string;
      expected_revision: number;
      model_id: string;
      prompt: string;
      parameters: ImageParameters;
      input_ids: string[];
    };
    result: TaskView;
  };
  cancel_image_task: { args: { task_id: string }; result: TaskView };
  retry_local_save: { args: { task_id: string }; result: TaskView };
  save_image_as: { args: { image_id: string }; result: { saved: boolean } };
  copy_image: { args: { image_id: string }; result: { copied: boolean } };
  delete_history: {
    args: { image_ids: string[]; confirmed: true };
    result: { deleted: number };
  };
  reuse_task_parameters: {
    args: { task_id: string };
    result: {
      prompt: string;
      model_id: string;
      parameters: ImageParameters;
      inputs: ImageInput[];
    };
  };
  use_history_as_input: {
    args: {
      image_id: string;
      connection_id: string;
      expected_revision: number;
    };
    result: ImageInput;
  };
  update_settings: {
    args: { patch: Partial<SettingsView> };
    result: SettingsView;
  };
  prepare_diagnostics: {
    args: Record<string, never>;
    result: DiagnosticsPreview;
  };
  export_diagnostics: {
    args: { preview_id: string };
    result: { saved: boolean };
  };
  check_update: { args: Record<string, never>; result: UpdateView };
  download_update: {
    args: { version: string; confirmed: true };
    result: UpdateView;
  };
  install_update: {
    args: { version: string; confirmed: true };
    result: UpdateView;
  };
}
