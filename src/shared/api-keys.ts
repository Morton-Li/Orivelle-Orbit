import type { ClientKind, ClaudeModelMapping } from "./contracts";

export type ApiKeyStatus =
  | "active"
  | "spending_exhausted"
  | "risk_suspended"
  | "expired"
  | "revoked"
  | "retired";
export type ServiceTier =
  "default" | "follow_client" | "priority" | "ultrafast";
export type ApiKeyView = {
  id: string;
  name: string;
  prefix: string;
  status: ApiKeyStatus;
  state: string;
  key_revision: string;
  management_revision: string;
  service_tier: ServiceTier | null;
  force_fast: boolean;
  service_tier_revision: number;
  spending_limit_mode: "cumulative" | "rolling";
  spending_limit: string | null;
  spending_used: string;
  spending_active_used: string;
  spending_remaining: string | null;
  spending_window_minutes: number | null;
  spending_window_used: string;
  spending_window_started_at: string | null;
  spending_window_expires_at: string | null;
  spending_window_resets_at: string | null;
  spending_window_status: string | null;
  expires_at: string | null;
  last_used_at: string | null;
  created_at: string | null;
  revoked_at: string | null;
  legacy_issued: boolean;
  selectable: boolean;
};
export type KeyScope = { connection_id: string; expected_revision: number };
export type ApiKeyQuery = KeyScope & {
  search: string;
  status: ApiKeyStatus | null;
  page: number;
};
export type KeyReference = KeyScope & {
  api_key_id: string;
  key_revision: string;
};
export type ApiKeyChanges = {
  name: string;
  expires_at: string | null;
  service_tier: ServiceTier;
  spending_limit: string | null;
  spending_limit_mode: "cumulative" | "rolling";
  spending_window_minutes: number | null;
};
export type KeyAction =
  "create" | "update" | "rotate" | "reset" | "revoke" | "delete";
export type ApiKeyMutation = KeyScope & {
  action: KeyAction;
  api_key_id: string | null;
  management_revision: string | null;
  changes: ApiKeyChanges | null;
};
export type KeyMutationResult = {
  api_key: ApiKeyView | null;
  completed: boolean;
  operation_id: string;
};
export type KeyBindingView = {
  client: ClientKind;
  api_key_id: string | null;
  name: string;
  key_revision: string;
  current: boolean;
  claude_mapping: ClaudeModelMapping | null;
};
export type ApiKeyPage = {
  items: ApiKeyView[];
  page: number;
  last_page: number;
  total: number;
  currency: string;
  can_write: boolean;
  can_reveal: boolean;
  selected: ApiKeyView | null;
  selected_ready: boolean;
  bindings: KeyBindingView[];
  legacy_key: ApiKeyView | null;
  legacy_cleanup_completed: boolean;
  cleanup_status: string;
  pending_operation: boolean;
};
/** Explicit reveal only. Never place this reply in an application store. */
export type RevealedApiKey = { api_key_id: string; plain_text_token: string };
