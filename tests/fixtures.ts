import { emptyState } from "../src/stores/orbit";
import type {
  BootstrapView,
  ClientView,
  ModelView,
  TaskView,
} from "../src/shared/contracts";
import { CAPABILITIES } from "../src/shared/contracts";

export function bootstrap(
  accountId = "synthetic-account-a",
  revision = 1,
): BootstrapView {
  const state = emptyState();
  const connectionId = `connection-${accountId}`;
  state.state_revision = revision;
  state.source = "synthetic";
  state.session = {
    status: "authenticated",
    panel_origin: "https://panel.invalid",
    inference_credential_status: "ready",
    account: {
      id: accountId,
      label: `Synthetic ${accountId}`,
      email: "fixture@example.invalid",
      member: false,
      wallet: {
        amount: "123456789.123456789",
        currency: "USD",
        status: "known",
      },
      budget: null,
    },
  };
  state.active_connection_id = connectionId;
  state.connections = [
    {
      id: connectionId,
      label: `Fixture ${accountId}`,
      panel_origin: "https://panel.invalid",
      account_id: accountId,
      node_id: "node-a",
      node_label: "Synthetic node",
      edge_base_url: "https://edge.invalid/v1",
      provider_id: "openai",
      revision: 1,
      credential_revision: "credential-revision-1",
      permission_state: "allowed",
    },
  ];
  state.nodes = [
    {
      id: "node-a",
      label: "Synthetic node",
      base_url: "https://edge.invalid/v1",
      available: true,
      maximum_request_bytes: 64 * 1024 ** 2,
      capabilities: Object.values(CAPABILITIES),
      revision: "node-revision-1",
    },
  ];
  state.clients = [client("codex"), client("claude")];
  return state;
}
export function client(kind: "codex" | "claude" = "codex"): ClientView {
  return {
    client: kind,
    installed: true,
    display_name: kind,
    version: "synthetic-version",
    bundle_id: "test.invalid",
    app_path: "/fixture/app",
    config_paths: ["/fixture/config"],
    platform_supported: true,
    managed: "no",
    configuration_state: "saved",
    loaded_state: "restart_required",
    connection_state: "not_tested",
    configuration_revision: "config-1",
    loaded_evidence: null,
    connection_evidence: null,
    manual_steps: [],
  };
}
export function imageModel(id = "synthetic-image"): ModelView {
  return {
    id,
    label: id,
    provider_id: "openai",
    revision: "model-revision-1",
    category: "image",
    capabilities: [CAPABILITIES.imageGeneration, CAPABILITIES.imageEdit],
    execution_profile: "openai_images",
    image_billing_mode: "image_tokens_v1",
    reasoning_efforts: [],
    support_status: "unverified",
    image_parameters: {
      max_results: 5,
      max_inputs: 5,
      maximum_request_bytes: 64 * 1024 ** 2,
      prompt_maximum_bytes: 32768,
      qualities: ["auto", "low", "medium", "high"],
      sizes: ["auto", "1024x1024"],
      backgrounds: ["auto", "opaque", "transparent"],
      verified: false,
    },
    sales_price: {
      currency: "USD",
      source: "sales",
      status: "configured",
      rates: [
        { meter: "text_input", unit_size: 1000000, unit_price: "8.00" },
        { meter: "cached_text_input", unit_size: 1000000, unit_price: "2.00" },
        { meter: "image_output", unit_size: 1000000, unit_price: "32.00" },
      ],
    },
    official_price: null,
  };
}
export function task(state: TaskView["state"] = "completed"): TaskView {
  return {
    id: "synthetic-task",
    account_id: "synthetic-account-a",
    connection_id: "connection-synthetic-account-a",
    connection_revision: 1,
    credential_revision: "credential-revision-1",
    node_id: "node-a",
    kind: "generation",
    requested_model: "synthetic-image",
    prompt: "Synthetic prompt",
    parameters: {},
    state,
    started_at: "2026-10-13T03:00:00Z",
    request_id: "synthetic-request",
    result_state: "unconfirmed",
    outcome: null,
    charge_state: "pending",
    charges: [],
    image_ids: [],
    local_save_available: state === "save_failed",
    error: null,
  };
}
