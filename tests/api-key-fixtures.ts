import type { ApiKeyPage, ApiKeyView } from "../src/shared/api-keys";

export function apiKey(
  letter = "A",
  overrides: Partial<ApiKeyView> = {},
): ApiKeyView {
  return {
    id: letter.repeat(26),
    name: letter === "A" ? "日常开发" : "图片创作",
    prefix: "orv_sk_" + letter.toLowerCase().repeat(8),
    status: "active",
    state: "active",
    key_revision: letter.toLowerCase().repeat(64),
    management_revision: "f".repeat(64),
    service_tier: "follow_client",
    force_fast: false,
    service_tier_revision: 1,
    spending_limit_mode: "cumulative",
    spending_limit: "50.00000000",
    spending_used: "8.25000000",
    spending_active_used: "8.25000000",
    spending_remaining: "41.75000000",
    spending_window_minutes: null,
    spending_window_used: "0.00000000",
    spending_window_started_at: null,
    spending_window_expires_at: null,
    spending_window_resets_at: null,
    spending_window_status: null,
    expires_at: null,
    last_used_at: "2026-11-15T08:00:00Z",
    created_at: "2026-11-01T08:00:00Z",
    revoked_at: null,
    legacy_issued: false,
    selectable: true,
    ...overrides,
  };
}
export function apiKeyPage(overrides: Partial<ApiKeyPage> = {}): ApiKeyPage {
  const first = apiKey();
  return {
    items: [
      first,
      apiKey("B", { spending_limit: null }),
      apiKey("C", {
        name: "旧版自动密钥",
        legacy_issued: true,
        selectable: false,
      }),
      apiKey("D", {
        name: "临时测试",
        state: "expired",
        status: "expired",
        selectable: false,
        expires_at: "2026-11-01T08:00:00Z",
      }),
    ],
    selected: first,
    selected_ready: true,
    bindings: [
      {
        client: "codex",
        api_key_id: first.id,
        name: first.name,
        key_revision: first.key_revision,
        current: true,
        claude_mapping: null,
      },
      {
        client: "claude",
        api_key_id: first.id,
        name: first.name,
        key_revision: first.key_revision,
        current: true,
        claude_mapping: null,
      },
    ],
    page: 1,
    last_page: 1,
    total: 4,
    currency: "USD",
    can_write: true,
    can_reveal: true,
    legacy_key: null,
    legacy_cleanup_completed: false,
    cleanup_status: "pending",
    pending_operation: false,
    ...overrides,
  };
}
