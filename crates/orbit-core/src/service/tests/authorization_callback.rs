use super::*;

#[tokio::test]
async fn browser_callback_reports_the_validated_outcome_without_returning_credentials() {
    for mode in [
        "connected",
        "denied",
        "wrong_state",
        "wrong_host",
        "exchange_failed",
    ] {
        let (_dir, service, _vault, mut record, install) = setup();
        record.session_status = "expired".into();
        service
            .store
            .put(
                "connection",
                &record.view.id,
                &record.view.account_id,
                &record,
            )
            .unwrap();
        let captured = Arc::new(Mutex::new(None::<Value>));
        let request = captured.clone();
        let exchanges = Arc::new(AtomicU64::new(0));
        let exchange_count = exchanges.clone();
        service.http.scripted(Arc::new(move |input| {
            let (request, exchange_count) = (request.clone(), exchange_count.clone());
            Box::pin(async move {
                let mut wire = golden();
                let reply = if input.url.ends_with("/authorization-requests") {
                    *request.lock().unwrap() = Some(serde_json::from_slice(&input.body).unwrap());
                    wire["start_reply"]["data"]["expires_at"] =
                        json!((chrono::Utc::now() + chrono::Duration::seconds(60)).to_rfc3339());
                    wire["start_reply"].clone()
                } else if input.url.ends_with("/token") {
                    exchange_count.fetch_add(1, Ordering::SeqCst);
                    if mode == "exchange_failed" {
                        return Err(fail("panel_unavailable"));
                    }
                    let mut reply = token_reply();
                    reply["data"]["credential_delivery_ticket"] = Value::Null;
                    reply
                } else {
                    assert!(input.url.ends_with("/bootstrap"));
                    wire["bootstrap_reply"].clone()
                };
                Ok(crate::network::RemoteResponse {
                    status: 200,
                    request_id: None,
                    bytes: serde_json::to_vec(&reply).unwrap(),
                })
            })
        }));
        let finished = Arc::new(tokio::sync::Notify::new());
        let signal = finished.clone();
        service
            .observe_authorization(Arc::new(move |_| signal.notify_one()))
            .unwrap();
        service.start_login(&install.origin).await.unwrap();
        let request = captured.lock().unwrap().clone().unwrap();
        let state = request["state"].as_str().unwrap();
        let mut callback = url::Url::parse(request["redirect_uri"].as_str().unwrap()).unwrap();
        callback
            .query_pairs_mut()
            .append_pair(
                "state",
                if mode == "wrong_state" {
                    "invalid-state"
                } else {
                    state
                },
            )
            .append_pair("iss", &install.origin)
            .append_pair(
                if mode == "denied" { "error" } else { "code" },
                if mode == "denied" {
                    "access_denied"
                } else {
                    "synthetic-callback-code"
                },
            );
        let mut socket = TcpStream::connect(("127.0.0.1", callback.port().unwrap()))
            .await
            .unwrap();
        let message = format!(
            "GET {}?{} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            callback.path(),
            callback.query().unwrap(),
            if mode == "wrong_host" {
                "invalid.example.test"
            } else {
                callback.authority()
            },
        );
        socket.write_all(message.as_bytes()).await.unwrap();
        let mut response = String::new();
        tokio::time::timeout(Duration::from_secs(5), socket.read_to_string(&mut response))
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), finished.notified())
            .await
            .unwrap();
        let (headers, html) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.contains("Content-Type: text/html; charset=utf-8"));
        assert!(headers.contains("Cache-Control: no-store, private"));
        assert!(headers.contains("Referrer-Policy: no-referrer"));
        assert!(headers.contains("Content-Security-Policy: default-src 'none'"));
        assert!(headers.contains(&format!("Content-Length: {}", html.len())));
        assert!(html.contains("返回 Orbit 继续"));
        for secret in [
            state,
            "synthetic-callback-code",
            "orv_orbit_at_",
            "orv_orbit_rt_",
            "orv_sk_",
        ] {
            assert!(!html.contains(secret));
        }
        let event: AuthorizationEvent = service
            .store
            .get("authorization-terminal", "local")
            .unwrap()
            .unwrap();
        if mode == "connected" {
            assert!(headers.starts_with("HTTP/1.1 200 OK"));
            assert!(html.contains("<h1 id=\"result-title\">账户已连接</h1>"));
            assert_eq!(event.state, "connected");
            assert_eq!(
                service.record(&record.view.id).unwrap().session_status,
                "authenticated"
            );
        } else {
            assert!(!html.contains("账户已连接"));
            assert_eq!(
                service.record(&record.view.id).unwrap().session_status,
                "expired"
            );
            if mode == "denied" {
                assert!(html.contains("已取消授权"));
                assert_eq!(event.state, "denied");
            } else {
                assert!(html.contains("连接未完成"));
                assert!(headers.starts_with(if mode == "exchange_failed" {
                    "HTTP/1.1 502"
                } else {
                    "HTTP/1.1 400"
                }));
                assert_eq!(event.state, "failed");
            }
        }
        assert_eq!(
            exchanges.load(Ordering::SeqCst),
            u64::from(matches!(mode, "connected" | "exchange_failed"))
        );
    }
}
