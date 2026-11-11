use super::*;

// Explicitly invoked local acceptance only. This client trusts one project CA
// and reserved hostname; no local-origin override exists in production.
#[tokio::test]
#[ignore = "requires an owned Docker demo and an explicitly approved image budget"]
async fn local_docker_image_once() {
    assert_eq!(std::env::var("ORBIT_IMAGE_LIVE_DEMO").as_deref(), Ok("1"));
    let root = PathBuf::from(std::env::var("ORBIT_IMAGE_DEMO_ROOT").unwrap());
    assert!(root.is_absolute());
    let mode = std::env::var("ORBIT_IMAGE_DEMO_OPERATION").unwrap();
    assert!(matches!(mode.as_str(), "generation" | "edit"));
    let evidence = root.join("evidence");
    let output = evidence.join(format!("{mode}.json"));
    assert!(!output.exists(), "never replay a recorded image attempt");
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json")).unwrap()).unwrap();
    let ca = std::fs::read(root.join("certs/ca.pem")).unwrap();
    let (_dir, service, _vault, mut record, install) = setup();
    // Service keeps a weak self reference. Unwrap the sole strong owner and
    // rebuild that reference while installing the strict local TLS client.
    let mut native =
        Arc::try_unwrap(service).unwrap_or_else(|_| panic!("exclusive demo service required"));
    native.http = Http::for_local_tls_acceptance(&ca, 18782).unwrap();
    let service = Arc::new_cyclic(|weak| {
        native.self_reference = weak.clone();
        native
    });
    record.view.edge_base_url = Some("https://orbit-e2e.test:18782/v1".into());
    service
        .store
        .put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            &record,
        )
        .unwrap();
    let mut delivery = golden()["credential_reply"]["data"].clone();
    delivery["plain_text_token"] = fixture["api_token"].clone();
    delivery["api_key_id"] = fixture["api_key_id"].clone();
    service
        .accept_credential(
            &mut record,
            &install,
            serde_json::from_value(delivery).unwrap(),
        )
        .unwrap();
    let mut snapshot = task_snapshot(&record);
    snapshot.task.state = "queued".into();
    snapshot.task.kind = mode.clone();
    snapshot.task.requested_model = "reasoning-test-images".into();
    snapshot.task.request_id = None;
    snapshot.task.prompt = if mode == "generation" {
        "A minimal still life: one small orange ceramic cup on a pale blue tabletop, soft daylight, clean composition, no text.".into()
    } else {
        "Keep the same ceramic cup and composition, change the cup color to emerald green. No text."
            .into()
    };
    snapshot.task.parameters = ImageParameters {
        n: Some(1),
        size: Some(
            if mode == "generation" {
                "1024x1024"
            } else {
                "1536x1024"
            }
            .into(),
        ),
        quality: Some("low".into()),
        background: Some("opaque".into()),
        output_format: Some(if mode == "generation" { "png" } else { "jpeg" }.into()),
        output_compression: if mode == "edit" { Some(65) } else { None },
        ..Default::default()
    };
    if mode == "edit" {
        let previous: Value =
            serde_json::from_slice(&std::fs::read(evidence.join("generation.json")).unwrap())
                .unwrap();
        let filename = previous["artifact"]
            .as_str()
            .expect("generation must have saved a valid image");
        let source = evidence.join(filename);
        assert_eq!(source.parent(), Some(evidence.as_path()));
        let input = images::import_input(&service.store, &source, &record.view).unwrap();
        snapshot
            .inputs
            .push(service.store.get("input", &input.id).unwrap().unwrap());
    }
    service.persist_task(&snapshot).unwrap();
    // Reserve the attempt before the only network send, including failed sends.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
        .unwrap();
    let result = service.send_image_once(&mut snapshot).await;
    let mut report = json!({"operation":mode,"task":snapshot.task,"error_code":result.as_ref().err().map(|e|e.code.clone())});
    if result.is_ok() {
        let image: HistoryImage = service
            .store
            .get("image", &snapshot.task.image_ids[0])
            .unwrap()
            .unwrap();
        let filename: String = service.store.get("image-file", &image.id).unwrap().unwrap();
        let bytes =
            std::fs::read(images::owned_image_path(&service.store, &filename).unwrap()).unwrap();
        let suffix = match image.mime.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => panic!("validated format required"),
        };
        let artifact = format!("{mode}.{suffix}");
        std::fs::write(evidence.join(&artifact), &bytes).unwrap();
        report["artifact"] = artifact.into();
        report["decoded"] = json!({"width":image.width,"height":image.height,"mime":image.mime,"sha256":sha256(&bytes)});
        assert_eq!(
            service
                .send_image_once(&mut snapshot)
                .await
                .unwrap_err()
                .code,
            "task_state_conflict"
        );
    }
    std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(
        result.is_ok(),
        "image attempt failed; inspect safe evidence error_code"
    );
}
