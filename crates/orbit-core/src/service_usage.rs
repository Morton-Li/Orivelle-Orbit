use super::*;
use crate::usage::{UsagePage, UsageQuery, UsageSummary};

impl Service {
    fn usage_scope(
        &self,
        request: &UsageQuery,
        expected: Option<&ConnectionRecord>,
    ) -> Result<ConnectionRecord> {
        let _gate = self
            .account_gate
            .lock()
            .map_err(|_| fail("state_unavailable"))?;
        let current = self.record(&request.connection_id)?;
        if current.view.revision != request.expected_revision
            || current.session_status != "authenticated"
            || self.store.meta("active_connection")?.as_deref()
                != Some(request.connection_id.as_str())
            || expected.is_some_and(|old| {
                current.view.account_id != old.view.account_id
                    || current.view.panel_origin != old.view.panel_origin
                    || current.installation_id != old.installation_id
                    || current.authorization_epoch != old.authorization_epoch
                    || current.session_reference != old.session_reference
            })
        {
            return Err(fail("account_scope_mismatch"));
        }
        Ok(current)
    }
    pub async fn read_usage_summary(&self, request: &UsageQuery) -> Result<UsageSummary> {
        let _operation = self.begin_mutation()?;
        let target = request.target(true)?;
        let original = self.usage_scope(request, None)?;
        let (record, installation, session) = self.fresh_session(&request.connection_id).await?;
        self.usage_scope(request, Some(&original))?;
        let response = self
            .http
            .panel(
                &record.view.panel_origin,
                &target,
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await;
        self.usage_scope(request, Some(&record))?;
        response
    }
    pub async fn list_usage_records(&self, request: &UsageQuery) -> Result<UsagePage> {
        let _operation = self.begin_mutation()?;
        let target = request.target(false)?;
        let original = self.usage_scope(request, None)?;
        let (record, installation, session) = self.fresh_session(&request.connection_id).await?;
        self.usage_scope(request, Some(&original))?;
        let response = self
            .http
            .panel_document(
                &record.view.panel_origin,
                &target,
                Method::GET,
                vec![],
                Some((&installation, &session, self.vault.as_ref())),
                true,
            )
            .await;
        self.usage_scope(request, Some(&record))?;
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::RemoteResponse;
    fn request(record: &ConnectionRecord) -> UsageQuery {
        UsageQuery {
            connection_id: record.view.id.clone(),
            expected_revision: record.view.revision,
            date_from: "2026-11-01".into(),
            date_to: "2026-11-13".into(),
            timezone: "Asia/Shanghai".into(),
            outcome: None,
            cursor: None,
        }
    }
    fn page() -> Value {
        json!({"data":[],"prev_cursor":null,"next_cursor":"signed+/cursor=","cursor_reset":false,
            "range":{"date_from":"2026-11-01","date_to":"2026-11-13","timezone":"Asia/Shanghai","defaulted":false,"max_days":90}})
    }
    #[tokio::test]
    async fn readonly_usage_keeps_cursor_metadata_and_rejects_late_actor_changes() {
        let (_dir, service, _vault, mut record, _) = super::super::tests::setup();
        record.view.permission_state = "readonly".into();
        record.view.node_id = None;
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        service.http.scripted(Arc::new(|r| {
            Box::pin(async move {
                assert!(r.url.contains("/usage/records?"));
                assert!(r.body.is_empty());
                Ok(RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: serde_json::to_vec(&page()).unwrap(),
                })
            })
        }));
        let query = request(&record);
        let result = service.list_usage_records(&query).await.unwrap();
        assert_eq!(result.next_cursor.as_deref(), Some("signed+/cursor="));
        let store = service.store.clone();
        let changed = record.clone();
        service.http.scripted(Arc::new(move |_| {
            let store = store.clone();
            let mut value = changed.clone();
            Box::pin(async move {
                value.view.account_id = "other-actor".into();
                store
                    .put("connection", &value.view.id, &value.view.account_id, &value)
                    .unwrap();
                Ok(RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: serde_json::to_vec(&page()).unwrap(),
                })
            })
        }));
        assert_eq!(
            service.list_usage_records(&query).await.err().unwrap().code,
            "account_scope_mismatch"
        );
    }
}
