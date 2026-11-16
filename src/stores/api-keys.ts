import { computed, ref, watch } from "vue";
import { defineStore } from "pinia";
import { call, publicFailure } from "../shared/bridge";
import type {
  PublicError,
  ClientKind,
  ClaudeModelMapping,
} from "../shared/contracts";
import type {
  ApiKeyPage,
  ApiKeyQuery,
  ApiKeyView,
  ApiKeyChanges,
  KeyAction,
  KeyReference,
} from "../shared/api-keys";
import { useOrbitStore } from "./orbit";

export const useApiKeysStore = defineStore("api-keys", () => {
  const orbit = useOrbitStore();
  const page = ref<ApiKeyPage | null>(null);
  const loading = ref(false);
  const busy = ref(false);
  const error = ref<PublicError | null>(null);
  const search = ref("");
  const status = ref<ApiKeyQuery["status"]>(null);
  const number = ref(1);
  const updateMapping = ref<{
    client: ClientKind;
    mapping: ClaudeModelMapping | null;
  } | null>(null);
  let generation = 0;
  let requested = false;
  const identity = computed(() => {
    const connection = orbit.activeConnection;
    return connection
      ? [connection.panel_origin, connection.account_id, connection.id].join(
          "/",
        )
      : "";
  });
  function scope() {
    const connection = orbit.activeConnection;
    if (!connection || orbit.state.session.status !== "authenticated")
      return null;
    return {
      connection_id: connection.id,
      expected_revision: connection.revision,
    };
  }
  async function load() {
    requested = true;
    const queryScope = scope();
    const captured = orbit.scope;
    const request = ++generation;
    error.value = null;
    if (!queryScope || !orbit.native) {
      page.value = null;
      loading.value = false;
      return;
    }
    loading.value = true;
    try {
      const result = await call("list_api_keys", {
        ...queryScope,
        search: search.value.trim(),
        status: status.value,
        page: number.value,
      });
      if (request === generation && captured === orbit.scope) {
        page.value = result;
        if (result.page > result.last_page && result.last_page > 0) {
          number.value = result.last_page;
          await load();
        }
      }
    } catch (reason) {
      if (request === generation && captured === orbit.scope)
        error.value = publicFailure(reason);
    } finally {
      if (request === generation) loading.value = false;
    }
  }
  function reference(key: ApiKeyView): KeyReference | null {
    const current = scope();
    return current
      ? { ...current, api_key_id: key.id, key_revision: key.key_revision }
      : null;
  }
  async function choose(key: ApiKeyView) {
    const request = reference(key);
    if (!request || busy.value) return false;
    busy.value = true;
    const actor = identity.value;
    try {
      await call("select_api_key", request);
      if (actor !== identity.value) return false;
      await orbit.refresh();
      await load();
      return true;
    } catch (reason) {
      if (actor === identity.value) orbit.report(reason);
      return false;
    } finally {
      busy.value = false;
    }
  }
  async function mutate(
    action: KeyAction,
    key: ApiKeyView | null,
    changes: ApiKeyChanges | null = null,
  ) {
    const current = scope();
    if (!current || busy.value) return;
    const actor = identity.value;
    busy.value = true;
    try {
      const result = await call("mutate_api_key", {
        ...current,
        action,
        api_key_id: key?.id ?? null,
        management_revision: key?.management_revision ?? null,
        changes,
      });
      if (actor !== identity.value) return;
      await orbit.refresh();
      await load();
      return result;
    } catch (reason) {
      if (actor === identity.value) {
        orbit.report(reason);
        await orbit.refresh();
        await load();
      }
      return;
    } finally {
      busy.value = false;
    }
  }
  async function recover() {
    const current = scope();
    if (!current || busy.value) return;
    const actor = identity.value;
    busy.value = true;
    try {
      const result = await call("recover_api_key_operation", current);
      if (actor !== identity.value) return;
      await orbit.refresh();
      await load();
      return result;
    } catch (reason) {
      if (actor === identity.value) {
        orbit.report(reason);
        await load();
      }
    } finally {
      busy.value = false;
    }
  }
  async function details(key: ApiKeyView) {
    const request = reference(key);
    const actor = identity.value;
    if (!request) return;
    try {
      const result = await call("get_api_key", request);
      if (actor === identity.value) return result;
    } catch (reason) {
      if (actor === identity.value) orbit.report(reason);
    }
    return;
  }
  async function copy(key: ApiKeyView) {
    const request = reference(key);
    const actor = identity.value;
    if (!request) return false;
    try {
      await call("copy_api_key", request);
      return actor === identity.value;
    } catch (reason) {
      if (actor === identity.value) orbit.report(reason);
      return false;
    }
  }
  watch(
    identity,
    () => {
      ++generation;
      page.value = null;
      error.value = null;
      search.value = "";
      status.value = null;
      number.value = 1;
      updateMapping.value = null;
      loading.value = false;
    },
    { flush: "sync" },
  );
  watch(
    () => orbit.scope,
    () => {
      if (requested) void load();
    },
  );
  return {
    page,
    loading,
    busy,
    error,
    search,
    status,
    number,
    identity,
    updateMapping,
    load,
    choose,
    mutate,
    recover,
    details,
    copy,
  };
});
