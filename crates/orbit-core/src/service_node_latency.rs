use super::*;
use futures_util::{StreamExt, stream};
use tokio::sync::watch;

pub(super) struct NodeProbes {
    active: Mutex<ProbeState>,
    // Shared across runs so replacing a cancelled run cannot exceed the limit.
    workers: Semaphore,
}

#[derive(Default)]
struct ProbeState {
    running: Option<(String, watch::Sender<bool>)>,
    // Cancellation IPC may arrive before the async probe handler is first polled.
    cancelled: std::collections::VecDeque<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::RemoteResponse;
    use std::sync::atomic::AtomicUsize;

    struct Fixture {
        _dir: tempfile::TempDir,
        service: Arc<Service>,
        record: ConnectionRecord,
        ids: Vec<String>,
    }
    fn fixture(count: usize) -> Fixture {
        let (dir, service, _, mut record, _) = super::super::tests::setup();
        record.session_expires_at = Some(chrono::Utc::now().timestamp() + 3600);
        record.view.permission_state = "readonly".into();
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        let mut cache = service.account_cache(&record).unwrap();
        let template = cache.nodes[0].clone();
        cache.nodes = (0..count)
            .map(|index| NodeView {
                id: format!("node-{index}"),
                available: true,
                base_url: "https://edge.example.test:8443/v1".into(),
                ..template.clone()
            })
            .collect();
        let ids = cache.nodes.iter().map(|node| node.id.clone()).collect();
        service
            .store
            .put(
                "account-cache",
                &record.view.id,
                &record.view.account_id,
                &cache,
            )
            .unwrap();
        Fixture {
            _dir: dir,
            service,
            record,
            ids,
        }
    }
    fn response(status: u16) -> Result<RemoteResponse> {
        Ok(RemoteResponse {
            status,
            request_id: None,
            bytes: vec![],
        })
    }

    #[tokio::test]
    async fn probes_only_health_without_credentials_or_configuration_mutations() {
        let f = fixture(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        f.service.http.scripted(Arc::new(move |request| {
            assert_eq!(request.method, Method::GET);
            assert_eq!(request.url, "https://edge.example.test:8443/healthz");
            assert!(request.body.is_empty());
            assert_eq!(
                request.headers,
                vec![("Cache-Control".into(), "no-cache, no-store".into())]
            );
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { response(204) })
        }));
        let before_cache =
            serde_json::to_value(f.service.account_cache(&f.record).unwrap()).unwrap();
        let before_record =
            serde_json::to_value(f.service.record(&f.record.view.id).unwrap()).unwrap();
        let revision = f.service.state_revision.load(Ordering::SeqCst);
        let result = f
            .service
            .probe_nodes(&f.record.view.id, f.record.view.revision, &f.ids, &uuid())
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(result[0].status, NodeLatencyStatus::Available);
        assert!(result[0].milliseconds.unwrap() >= 1);
        assert!(chrono::DateTime::parse_from_rfc3339(&result[0].measured_at).is_ok());
        assert_eq!(
            serde_json::to_value(f.service.account_cache(&f.record).unwrap()).unwrap(),
            before_cache
        );
        assert_eq!(
            serde_json::to_value(f.service.record(&f.record.view.id).unwrap()).unwrap(),
            before_record
        );
        assert_eq!(f.service.state_revision.load(Ordering::SeqCst), revision);
    }

    #[tokio::test]
    async fn bounds_concurrency_across_nodes() {
        let f = fixture(7);
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let (current, peak) = (active.clone(), maximum.clone());
        f.service.http.scripted(Arc::new(move |_| {
            let (current, peak) = (current.clone(), peak.clone());
            Box::pin(async move {
                let count = current.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
                current.fetch_sub(1, Ordering::SeqCst);
                response(204)
            })
        }));
        assert_eq!(
            f.service
                .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
                .await
                .unwrap()
                .len(),
            7
        );
        assert_eq!(maximum.load(Ordering::SeqCst), 3);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn distinguishes_deadline_from_http_failure() {
        let f = fixture(1);
        f.service
            .http
            .scripted(Arc::new(|_| Box::pin(async { response(307) })));
        let result = f
            .service
            .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
            .await
            .unwrap();
        assert_eq!(result[0].status, NodeLatencyStatus::Failed);
        assert!(result[0].milliseconds.is_none());
        f.service
            .http
            .scripted(Arc::new(|_| Box::pin(std::future::pending())));
        let result = f
            .service
            .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
            .await
            .unwrap();
        assert_eq!(result[0].status, NodeLatencyStatus::Timeout);
        assert!(result[0].milliseconds.is_none());
    }

    #[tokio::test]
    async fn cancellation_drops_io_and_releases_worker_permits() {
        let f = fixture(4);
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        f.service.http.scripted(Arc::new(move |_| {
            signal.notify_one();
            Box::pin(std::future::pending())
        }));
        let (service, id, nodes, probe) = (
            f.service.clone(),
            f.record.view.id.clone(),
            f.ids.clone(),
            uuid(),
        );
        let run_id = probe.clone();
        let task = tokio::spawn(async move { service.probe_nodes(&id, 1, &nodes, &run_id).await });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        f.service.cancel_node_probes(&probe).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err().code, "node_probe_cancelled");
        assert_eq!(f.service.node_probes.workers.available_permits(), 3);
        assert!(
            f.service
                .node_probes
                .active
                .lock()
                .unwrap()
                .running
                .is_none()
        );
    }

    #[tokio::test]
    async fn cancellation_before_handler_start_prevents_network() {
        let f = fixture(1);
        f.service
            .http
            .scripted(Arc::new(|_| panic!("cancelled probe reached network")));
        let probe = uuid();
        f.service.cancel_node_probes(&probe).unwrap();
        let error = f
            .service
            .probe_nodes(&f.record.view.id, 1, &f.ids, &probe)
            .await
            .unwrap_err();
        assert_eq!(error.code, "node_probe_cancelled");
    }

    #[tokio::test]
    async fn replacing_a_run_cancels_it_and_late_old_cancellation_leaves_the_new_run_alive() {
        let f = fixture(1);
        let started = Arc::new(tokio::sync::Notify::new());
        let signal = started.clone();
        f.service.http.scripted(Arc::new(move |_| {
            signal.notify_one();
            Box::pin(std::future::pending())
        }));
        let old_probe = uuid();
        let (service, id, ids, probe) = (
            f.service.clone(),
            f.record.view.id.clone(),
            f.ids.clone(),
            old_probe.clone(),
        );
        let old = tokio::spawn(async move { service.probe_nodes(&id, 1, &ids, &probe).await });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();
        let callback_service = Arc::downgrade(&f.service);
        f.service.http.scripted(Arc::new(move |_| {
            callback_service
                .upgrade()
                .unwrap()
                .cancel_node_probes(&old_probe)
                .unwrap();
            Box::pin(async { response(204) })
        }));
        let result = f
            .service
            .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
            .await
            .unwrap();
        assert_eq!(result[0].status, NodeLatencyStatus::Available);
        let error = tokio::time::timeout(Duration::from_secs(1), old)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "node_probe_cancelled");
        assert_eq!(f.service.node_probes.workers.available_permits(), 3);
    }

    #[tokio::test]
    async fn rejects_unknown_disabled_duplicate_and_unsafe_targets_before_network() {
        let f = fixture(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        f.service.http.scripted(Arc::new(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { response(204) })
        }));
        for ids in [
            vec![],
            vec!["unknown".into()],
            vec![f.ids[0].clone(), f.ids[0].clone()],
        ] {
            assert!(
                f.service
                    .probe_nodes(&f.record.view.id, 1, &ids, &uuid())
                    .await
                    .is_err()
            );
        }
        let mut cache = f.service.account_cache(&f.record).unwrap();
        cache.nodes[0].available = false;
        f.service
            .store
            .put(
                "account-cache",
                &f.record.view.id,
                &f.record.view.account_id,
                &cache,
            )
            .unwrap();
        assert!(
            f.service
                .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
                .await
                .is_err()
        );
        cache.nodes[0].available = true;
        for url in [
            "http://127.0.0.1/v1",
            "https://edge.example.test/v1?secret=1",
            "https://user@edge.example.test/v1",
            "https://edge.example.test/other",
        ] {
            cache.nodes[0].base_url = url.into();
            f.service
                .store
                .put(
                    "account-cache",
                    &f.record.view.id,
                    &f.record.view.account_id,
                    &cache,
                )
                .unwrap();
            assert!(
                f.service
                    .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
                    .await
                    .is_err()
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn rejects_late_results_after_authority_or_node_address_changes() {
        for authority_change in [true, false] {
            let f = fixture(1);
            let service = f.service.clone();
            let id = f.record.view.id.clone();
            f.service.http.scripted(Arc::new(move |_| {
                if authority_change {
                    let mut record = service.record(&id).unwrap();
                    record.authorization_epoch += 1;
                    service
                        .store
                        .put("connection", &id, &record.view.account_id, &record)
                        .unwrap();
                } else {
                    let record = service.record(&id).unwrap();
                    let mut cache = service.account_cache(&record).unwrap();
                    cache.nodes[0].base_url = "https://replacement.example.test/v1".into();
                    service
                        .store
                        .put("account-cache", &id, &record.view.account_id, &cache)
                        .unwrap();
                }
                Box::pin(async { response(204) })
            }));
            let error = f
                .service
                .probe_nodes(&f.record.view.id, 1, &f.ids, &uuid())
                .await
                .unwrap_err();
            assert_eq!(error.code, "connection_conflict");
        }
    }
}

impl Default for NodeProbes {
    fn default() -> Self {
        Self {
            active: Mutex::new(ProbeState::default()),
            workers: Semaphore::new(3),
        }
    }
}

struct ProbeRun<'a> {
    probes: &'a NodeProbes,
    id: String,
}

impl Drop for ProbeRun<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.probes.active.lock()
            && active
                .running
                .as_ref()
                .is_some_and(|(id, _)| id == &self.id)
        {
            active.running = None;
        }
    }
}

impl Service {
    // Probing is independent of inference permission, credentials and catalogue refresh.
    fn node_probe_context(
        &self,
        id: &str,
        revision: u64,
    ) -> Result<(ConnectionRecord, AccountCache)> {
        self.ensure_mutations_available()?;
        let record = self.record(id)?;
        let active = self
            .active_record()?
            .ok_or_else(|| fail("session_expired"))?;
        if active.view.id != id
            || record.view.revision != revision
            || record.session_status != "authenticated"
            || record
                .session_expires_at
                .is_none_or(|at| at <= chrono::Utc::now().timestamp())
        {
            return Err(fail("connection_conflict"));
        }
        self.ensure_installation_available(&record.installation_id)?;
        let cache = self.account_cache(&record)?;
        Ok((record, cache))
    }

    fn check_node_probe(&self, record: &ConnectionRecord, node: &NodeView) -> Result<()> {
        let (current, cache) = self.node_probe_context(&record.view.id, record.view.revision)?;
        if current.authorization_epoch != record.authorization_epoch
            || current.installation_id != record.installation_id
            || current.view.account_id != record.view.account_id
            || current.view.panel_origin != record.view.panel_origin
            || !cache.nodes.iter().any(|entry| {
                entry.id == node.id
                    && entry.revision == node.revision
                    && entry.base_url == node.base_url
                    && entry.available
            })
        {
            return Err(fail("connection_conflict"));
        }
        Ok(())
    }

    pub fn cancel_node_probes(&self, probe_id: &str) -> Result<()> {
        validate_uuid(probe_id)?;
        let mut active = self
            .node_probes
            .active
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        if let Some((id, cancel)) = active.running.as_ref()
            && id == probe_id
        {
            cancel.send_replace(true);
        }
        if !active.cancelled.iter().any(|id| id == probe_id) {
            if active.cancelled.len() >= 128 {
                active.cancelled.pop_front();
            }
            active.cancelled.push_back(probe_id.into());
        }
        Ok(())
    }

    pub async fn probe_nodes(
        &self,
        id: &str,
        revision: u64,
        node_ids: &[String],
        probe_id: &str,
    ) -> Result<Vec<NodeLatencyView>> {
        validate_uuid(probe_id)?;
        if node_ids.is_empty() || node_ids.len() > 128 {
            return Err(fail("invalid_node_probe"));
        }
        let (record, cache) = self.node_probe_context(id, revision)?;
        let mut unique = HashSet::new();
        let nodes = node_ids
            .iter()
            .map(|id| {
                if !unique.insert(id) {
                    return Err(fail("invalid_node_probe"));
                }
                cache
                    .nodes
                    .iter()
                    .find(|node| &node.id == id && node.available)
                    .cloned()
                    .ok_or_else(|| fail("node_unavailable"))
            })
            .collect::<Result<Vec<_>>>()?;
        let (cancel, mut cancelled) = watch::channel(false);
        {
            let mut active = self
                .node_probes
                .active
                .lock()
                .map_err(|_| fail("state_unavailable"))?;
            if active.cancelled.iter().any(|id| id == probe_id) {
                return Err(fail("node_probe_cancelled"));
            }
            if active
                .running
                .as_ref()
                .is_some_and(|(id, _)| id == probe_id)
            {
                return Err(fail("invalid_node_probe"));
            }
            if let Some((_, previous)) = active.running.replace((probe_id.into(), cancel)) {
                previous.send_replace(true);
            }
        }
        let _run = ProbeRun {
            probes: &self.node_probes,
            id: probe_id.into(),
        };
        let captured_record = &record;
        let measurements = stream::iter(nodes.clone().into_iter().map(|node| async move {
            let _worker = self
                .node_probes
                .workers
                .acquire()
                .await
                .map_err(|_| fail("state_unavailable"))?;
            self.measure_node_latency(captured_record, &node).await
        }))
        .buffer_unordered(3)
        .collect::<Vec<_>>();
        let results = tokio::select! {
            biased;
            _ = cancelled.changed() => return Err(fail("node_probe_cancelled")),
            results = measurements => results.into_iter().collect::<Result<Vec<_>>>()?,
        };
        // Reject the whole old run if account authority or a target changed while awaiting I/O.
        for node in &nodes {
            self.check_node_probe(&record, node)?;
        }
        Ok(results)
    }

    async fn measure_node_latency(
        &self,
        record: &ConnectionRecord,
        node: &NodeView,
    ) -> Result<NodeLatencyView> {
        let mut target = url::Url::parse(&node.base_url).map_err(|_| fail("invalid_node_probe"))?;
        if target.scheme() != "https"
            || !target.username().is_empty()
            || target.password().is_some()
            || target.query().is_some()
            || target.fragment().is_some()
            || target.path() != "/v1"
        {
            return Err(fail("invalid_node_probe"));
        }
        target.set_path("/healthz");
        let sample = async {
            let mut samples = Vec::with_capacity(3);
            // Discard the first request so connection establishment does not dominate the sample.
            for index in 0..4 {
                self.check_node_probe(record, node)?;
                let started = std::time::Instant::now();
                let response = tokio::time::timeout(
                    Duration::from_secs(3),
                    self.http.request(
                        Method::GET,
                        target.as_str(),
                        vec![],
                        vec![("Cache-Control".into(), "no-cache, no-store".into())],
                        1024,
                        Duration::from_secs(4),
                    ),
                )
                .await
                .map_err(|_| fail("node_probe_timeout"))??;
                if response.status != 204 {
                    return Err(fail("node_probe_failed"));
                }
                if index > 0 {
                    samples.push((started.elapsed().as_millis() as u64).max(1));
                }
            }
            samples.sort_unstable();
            Ok(samples[1])
        }
        .await;
        let (status, milliseconds) = match sample {
            Ok(value) => (NodeLatencyStatus::Available, Some(value)),
            Err(error)
                if error.code == "connection_conflict"
                    || error.code == "session_expired"
                    || error.code == "update_busy" =>
            {
                return Err(error);
            }
            Err(error) if error.code == "node_probe_timeout" => (NodeLatencyStatus::Timeout, None),
            Err(_) => (NodeLatencyStatus::Failed, None),
        };
        Ok(NodeLatencyView {
            node_id: node.id.clone(),
            node_revision: node.revision.clone(),
            status,
            milliseconds,
            measured_at: chrono::Utc::now().to_rfc3339(),
        })
    }
}
