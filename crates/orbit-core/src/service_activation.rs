use super::*;
use crate::{client_process::Process, dto::ClientActivationState as State, store::Snapshot};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Activation {
    route_revision: String,
    saved_at_micros: i64,
    binding_id: String,
    connection_id: String,
    node_id: String,
    requires_mapping: bool,
    state: State,
}

fn route(client: &ClientView) -> Option<&str> {
    match client.client {
        ClientKind::Codex => client.codex_route_revision.as_deref(),
        ClientKind::Claude => client.claude_route_revision.as_deref(),
    }
}

fn process_state(saved: i64, processes: Option<&[Process]>) -> State {
    let now = chrono::Utc::now().timestamp_micros();
    let Some(processes) = processes else {
        return State::Unverified;
    };
    if saved <= 0
        || saved > now
        || processes
            .iter()
            .any(|p| p.pid == 0 || p.started_at_micros <= 0 || p.started_at_micros > now)
    {
        return State::Unverified;
    }
    if processes.iter().any(|p| p.started_at_micros <= saved) {
        return State::WaitingRestart;
    }
    if processes.iter().any(|p| p.desktop) {
        State::Connected
    } else {
        State::WaitingStart
    }
}

fn scope_valid(
    activation: &Activation,
    client: ClientKind,
    active: Option<&ConnectionRecord>,
    snapshot: &Snapshot,
) -> bool {
    let Some(active) = active else {
        return false;
    };
    let Ok(Some(binding)) = snapshot.get::<HelperBinding>("binding", &activation.binding_id) else {
        return false;
    };
    let now = chrono::Utc::now().timestamp();
    active.view.id == activation.connection_id
        && active.view.node_id.as_deref() == Some(activation.node_id.as_str())
        && matches!(
            active.session_status.as_str(),
            "authenticated" | "refreshing"
        )
        && binding.client == client
        && binding.enabled
        && binding.account_id == active.view.account_id
        && binding.origin == active.view.panel_origin
        && binding.installation_id == active.installation_id
        && binding.provider_id == active.view.provider_id
        && binding.expires_at.is_none_or(|at| at > now)
        && snapshot
            .meta
            .get(&authorization_epoch_reference(
                &binding.origin,
                &binding.installation_id,
                &binding.account_id,
            ))
            .and_then(|epoch| epoch.parse::<u64>().ok())
            == Some(binding.authorization_epoch)
        && snapshot
            .meta
            .get(&crate::identity::credential_disabled_reference(
                &binding.secret_reference,
            ))
            .map(String::as_str)
            != Some("1")
        && snapshot
            .meta
            .get(&format!(
                "installation_forgetting/{}",
                binding.installation_id
            ))
            .map(String::as_str)
            != Some("1")
}

impl Service {
    pub(super) fn validate_client_activation_store(store: &Store) -> Result<()> {
        let _ = store.list::<Activation>("client-activation", None)?;
        Ok(())
    }

    fn activation_runtime_available(&self, saved: &Activation) -> bool {
        !saved.requires_mapping || self.claude_mappings.is_running(&saved.binding_id)
    }
    /// Runs in a native background worker, including while every window is hidden.
    /// Observes process identity and launch time only; never launches or stops apps.
    pub fn reconcile_client_activations(&self) -> Result<()> {
        if self.refresh_client_activations()? {
            self.changed();
        }
        Ok(())
    }

    pub(super) fn refresh_client_activations(&self) -> Result<bool> {
        let _operation = self.begin_mutation()?;
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let snapshot = self.store.snapshot()?;
        let active = snapshot
            .meta
            .get("active_connection")
            .map(|id| snapshot.get::<ConnectionRecord>("connection", id))
            .transpose()?
            .flatten();
        let clients = self
            .clients
            .lock()
            .map_err(|_| fail("state_unavailable"))?
            .clone();
        let mut changed = false;
        for cached in clients {
            let (client, _) = self.observe_client_policy(cached);
            let previous =
                snapshot.get::<Activation>("client-activation", client.client.as_str())?;
            if client.configuration_state != "saved" {
                if previous.is_some() && client.configuration_state == "not_configured" {
                    self.store
                        .delete("client-activation", client.client.as_str())?;
                    changed = true;
                }
                continue;
            }
            let Some(revision) = route(&client) else {
                continue;
            };
            let mut current = match previous
                .as_ref()
                .filter(|a| a.route_revision == revision && !a.binding_id.is_empty())
            {
                Some(saved) => saved.clone(),
                None => self
                    .activation_baseline(&client, active.as_ref())
                    .unwrap_or_else(|_| Activation {
                        route_revision: revision.into(),
                        saved_at_micros: 0,
                        binding_id: String::new(),
                        connection_id: String::new(),
                        node_id: String::new(),
                        requires_mapping: false,
                        state: State::Unverified,
                    }),
            };
            current.state = if current.binding_id.is_empty() {
                State::Unverified
            } else if !scope_valid(&current, client.client, active.as_ref(), &snapshot)
                || !self.activation_runtime_available(&current)
            {
                State::Unavailable
            } else if client.configuration_access != ConfigurationAccess::Allowed
                || !client.installed
            {
                State::Unverified
            } else {
                process_state(
                    current.saved_at_micros,
                    self.paths.client_processes(&client).as_deref(),
                )
            };
            if previous.as_ref() != Some(&current) {
                self.store.put(
                    "client-activation",
                    client.client.as_str(),
                    "local",
                    &current,
                )?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Recover old installations from the actual selected helper and file write
    /// times. This path reads no Vault item and sends no network/model request.
    fn activation_baseline(
        &self,
        client: &ClientView,
        active: Option<&ConnectionRecord>,
    ) -> Result<Activation> {
        let active = active.ok_or_else(|| fail("account_scope_mismatch"))?;
        let (binding_id, files, requires_mapping) = match client.client {
            ClientKind::Codex => {
                let bytes = read_file(&self.paths.codex_config)?;
                let slot = crate::codex_config::applied_slot(bytes.as_deref())?;
                let doc = crate::codex_config::document(bytes.as_deref())?;
                let provider = doc
                    .get("model_providers")
                    .and_then(|p| p.get(&slot))
                    .ok_or_else(|| fail("configuration_conflict"))?;
                let auth = provider
                    .get("auth")
                    .ok_or_else(|| fail("configuration_conflict"))?;
                let args = auth
                    .get("args")
                    .and_then(toml_edit::Item::as_array)
                    .ok_or_else(|| fail("configuration_conflict"))?;
                if auth.get("command").and_then(toml_edit::Item::as_str)
                    != self.paths.helper.to_str()
                    || args.len() != 2
                    || args.get(0).and_then(toml_edit::Value::as_str) != Some("codex")
                    || provider.get("base_url").and_then(toml_edit::Item::as_str)
                        != active.view.edge_base_url.as_deref()
                {
                    return Err(fail("configuration_conflict"));
                }
                (
                    args.get(1)
                        .and_then(toml_edit::Value::as_str)
                        .ok_or_else(|| fail("binding_disabled"))?
                        .to_owned(),
                    vec![self.paths.codex_config.clone(), self.paths.codex_catalog()],
                    false,
                )
            }
            ClientKind::Claude => {
                let (_, selected) = self.claude_source()?;
                let (id, bytes) = selected.ok_or_else(|| fail("configuration_conflict"))?;
                let profile: Value =
                    serde_json::from_slice(&bytes).map_err(|_| fail("configuration_conflict"))?;
                let args = profile["inferenceCredentialHelperArgs"]
                    .as_array()
                    .ok_or_else(|| fail("configuration_conflict"))?;
                if profile["inferenceCredentialHelper"].as_str() != self.paths.helper.to_str()
                    || args.len() != 2
                    || !matches!(args[0].as_str(), Some("claude" | "claude-map"))
                {
                    return Err(fail("configuration_conflict"));
                }
                let binding = args[1]
                    .as_str()
                    .ok_or_else(|| fail("binding_disabled"))?
                    .to_owned();
                if args[0].as_str() == Some("claude-map") {
                    let mapping = self
                        .store
                        .get::<crate::claude_mapping::MappingRecord>("claude-mapping", &binding)?
                        .ok_or_else(|| fail("binding_disabled"))?;
                    if mapping.context.connection_id != active.view.id
                        || Some(&mapping.context.node_id) != active.view.node_id.as_ref()
                    {
                        return Err(fail("account_scope_mismatch"));
                    }
                } else if self
                    .account_cache(active)?
                    .claude_endpoints
                    .get(active.view.node_id.as_deref().unwrap_or(""))
                    .map(String::as_str)
                    != profile["inferenceGatewayBaseUrl"].as_str()
                {
                    return Err(fail("account_scope_mismatch"));
                }
                (
                    binding,
                    vec![
                        self.paths.claude_library.join(format!("{id}.json")),
                        self.paths.claude_library.join("_meta.json"),
                        self.paths
                            .claude_library
                            .parent()
                            .ok_or_else(|| fail("invalid_path"))?
                            .join("claude_desktop_config.json"),
                    ],
                    args[0].as_str() == Some("claude-map"),
                )
            }
        };
        validate_uuid(&binding_id)?;
        let saved = files
            .iter()
            .map(|path| {
                std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|at| i64::try_from(at.as_micros()).ok())
                    .ok_or_else(|| fail("local_metadata_invalid"))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .ok_or_else(|| fail("local_metadata_invalid"))?;
        Ok(Activation {
            route_revision: route(client)
                .ok_or_else(|| fail("configuration_conflict"))?
                .into(),
            saved_at_micros: saved,
            binding_id,
            connection_id: active.view.id.clone(),
            node_id: active
                .view
                .node_id
                .clone()
                .ok_or_else(|| fail("node_unavailable"))?,
            state: State::Unverified,
            requires_mapping,
        })
    }

    pub(super) fn project_activation(
        &self,
        client: &mut ClientView,
        active: Option<&ConnectionRecord>,
        snapshot: &Snapshot,
    ) -> Result<()> {
        client.activation_state = None;
        if client.configuration_state != "saved" {
            return Ok(());
        }
        let state = match snapshot.get::<Activation>("client-activation", client.client.as_str())? {
            Some(saved)
                if Some(saved.route_revision.as_str()) == route(client)
                    && !saved.binding_id.is_empty() =>
            {
                if !scope_valid(&saved, client.client, active, snapshot)
                    || !self.activation_runtime_available(&saved)
                {
                    State::Unavailable
                } else if client.configuration_access != ConfigurationAccess::Allowed {
                    State::Unverified
                } else {
                    saved.state
                }
            }
            Some(saved)
                if saved.binding_id.is_empty()
                    && Some(saved.route_revision.as_str()) == route(client) =>
            {
                State::Unverified
            }
            _ => State::WaitingRestart,
        };
        client.activation_state = Some(state);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_requires_every_old_process_to_exit_and_a_new_desktop() {
        let saved = chrono::Utc::now().timestamp_micros() - 10_000_000;
        let old = Process {
            pid: 10,
            started_at_micros: saved - 1,
            desktop: true,
        };
        let fresh = Process {
            pid: 10,
            started_at_micros: saved + 1,
            desktop: true,
        };
        let backend = Process {
            desktop: false,
            pid: 20,
            ..old.clone()
        };
        assert_eq!(process_state(saved, None), State::Unverified);
        assert_eq!(process_state(saved, Some(&[])), State::WaitingStart);
        assert_eq!(process_state(saved, Some(&[old])), State::WaitingRestart);
        assert_eq!(
            process_state(saved, Some(&[fresh.clone(), backend.clone()])),
            State::WaitingRestart
        );
        assert_eq!(
            process_state(
                saved,
                Some(&[Process {
                    started_at_micros: saved + 1,
                    ..backend
                }])
            ),
            State::WaitingStart
        );
        // A reused PID is a new instance only when its creation time is newer.
        assert_eq!(process_state(saved, Some(&[fresh])), State::Connected);
    }

    #[test]
    fn client_activation_tracks_both_clients_and_survives_orbit_restart_without_network_evidence() {
        for kind in [ClientKind::Codex, ClientKind::Claude] {
            let f = crate::service::tests::policy_fixture(kind);
            let old = Process {
                pid: 11,
                started_at_micros: chrono::Utc::now().timestamp_micros() - 60_000_000,
                desktop: true,
            };
            let observed = Arc::new(Mutex::new(Some(vec![old.clone()])));
            let source = observed.clone();
            let mut paths = f.service.paths.clone();
            paths.fixture_client_processes =
                Some(Arc::new(move |_| source.lock().unwrap().clone()));
            let service =
                Service::new(f.service.store.clone(), f.vault.clone(), paths.clone()).unwrap();
            let preview = service
                .preview_client_config(&f.record.view.id, f.record.view.revision, kind, &f.model)
                .unwrap();
            service
                .apply_client_config(&preview.id, true, false)
                .unwrap();
            let state = |s: &Service| {
                s.bootstrap()
                    .unwrap()
                    .clients
                    .into_iter()
                    .find(|c| c.client == kind)
                    .unwrap()
            };
            assert_eq!(
                state(&service).activation_state,
                Some(State::WaitingRestart),
                "{}",
                kind.as_str()
            );
            let saved: Activation = service
                .store
                .get("client-activation", kind.as_str())
                .unwrap()
                .unwrap();
            let fresh = Process {
                started_at_micros: saved.saved_at_micros + 1,
                ..old.clone()
            };
            let calls = f.vault.calls.load(Ordering::SeqCst);
            let revision = service.store.snapshot().unwrap().revision;
            service.reconcile_client_activations().unwrap();
            assert_eq!(
                service
                    .store
                    .get::<Activation>("client-activation", kind.as_str())
                    .unwrap()
                    .unwrap(),
                saved,
                "{} baseline changed",
                kind.as_str()
            );
            assert_eq!(
                service.store.snapshot().unwrap().revision,
                revision,
                "unchanged polls do not write or emit revisions"
            );
            *observed.lock().unwrap() = Some(vec![
                fresh.clone(),
                Process {
                    desktop: false,
                    pid: 12,
                    ..old
                },
            ]);
            service.reconcile_client_activations().unwrap();
            assert_eq!(
                state(&service).activation_state,
                Some(State::WaitingRestart)
            );
            *observed.lock().unwrap() = None;
            service.reconcile_client_activations().unwrap();
            assert_eq!(state(&service).activation_state, Some(State::Unverified));
            *observed.lock().unwrap() = Some(vec![]);
            service.reconcile_client_activations().unwrap();
            assert_eq!(state(&service).activation_state, Some(State::WaitingStart));
            *observed.lock().unwrap() = Some(vec![Process {
                desktop: false,
                ..fresh.clone()
            }]);
            service.reconcile_client_activations().unwrap();
            assert_eq!(state(&service).activation_state, Some(State::WaitingStart));
            *observed.lock().unwrap() = Some(vec![fresh.clone()]);
            service.reconcile_client_activations().unwrap();
            assert_eq!(state(&service).activation_state, Some(State::Connected));
            assert_eq!(
                f.vault.calls.load(Ordering::SeqCst),
                calls,
                "process polling must not read credentials"
            );
            assert!(state(&service).loaded_evidence.is_none());
            assert!(state(&service).connection_evidence.is_none());
            assert_eq!(state(&service).connection_state, "not_tested");
            // The receipt persists, and an old-version installation with no receipt
            // can migrate from the actual helper and file publication timestamps.
            for legacy in [false, true] {
                if legacy {
                    service
                        .store
                        .delete("client-activation", kind.as_str())
                        .unwrap();
                }
                let rebuilt =
                    Service::new(service.store.clone(), f.vault.clone(), paths.clone()).unwrap();
                assert_eq!(state(&rebuilt).activation_state, Some(State::Connected));
            }
            if kind == ClientKind::Codex {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&service.paths.codex_config)
                    .unwrap()
                    .write_all(b"\n# Personal preference retained after connection\n")
                    .unwrap();
                service.reconcile_client_activations().unwrap();
                assert_eq!(state(&service).activation_state, Some(State::Connected));
                assert_eq!(
                    service
                        .store
                        .get::<Activation>("client-activation", kind.as_str())
                        .unwrap()
                        .unwrap()
                        .saved_at_micros,
                    saved.saved_at_micros
                );
            }
            // Explicit revocation removes the connected badge even before the
            // next process poll.
            let mut binding: HelperBinding = service
                .store
                .get("binding", &saved.binding_id)
                .unwrap()
                .unwrap();
            binding.enabled = false;
            service
                .store
                .put("binding", &binding.id, &binding.account_id, &binding)
                .unwrap();
            assert_eq!(state(&service).activation_state, Some(State::Unavailable));
            binding.enabled = true;
            service
                .store
                .put("binding", &binding.id, &binding.account_id, &binding)
                .unwrap();
            service.reconcile_client_activations().unwrap();
            assert_eq!(state(&service).activation_state, Some(State::Connected));
            let next = service
                .preview_client_config(&f.record.view.id, f.record.view.revision, kind, &f.model)
                .unwrap();
            service.apply_client_config(&next.id, true, false).unwrap();
            assert_eq!(
                state(&service).activation_state,
                Some(State::WaitingRestart),
                "a new write invalidates an earlier launch"
            );
            // Removing the switch restores the original files and removes the
            // lifecycle hint rather than preserving a green connected badge.
            let journals: Vec<Journal> = service.store.list("journal", None).unwrap();
            let source = journals
                .iter()
                .filter(|j| j.client == kind && j.phase == "committed")
                .max_by_key(|j| &j.created_at)
                .unwrap();
            crate::service::tests::mark_switch_snapshot(&service, &source.id);
            let restore = service.preview_restore(&source.id).unwrap();
            service
                .apply_client_config(&restore.id, true, true)
                .unwrap();
            assert_eq!(state(&service).configuration_state, "not_configured");
            assert_eq!(state(&service).activation_state, None);
        }
    }
}
