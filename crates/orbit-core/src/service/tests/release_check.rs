//! Opt-in synthetic acceptance of the native image consumer over real TLS.
//! The owned fixture supplies a disposable inference key; it must run an actual
//! local Edge, Panel and controlled upstream. Login and Vault remain synthetic.
use super::*;
use std::io::Write;

#[tokio::test]
#[ignore = "requires an owned synthetic local Panel/Edge/upstream fixture"]
async fn local_release_check_subscription_png_once() {
    run_case("subscription-png", false, false).await;
}

#[tokio::test]
#[ignore = "requires an owned synthetic local Panel/Edge/upstream fixture"]
async fn local_release_check_api_stream_once() {
    run_case("api-stream", true, false).await;
}

#[tokio::test]
#[ignore = "requires an owned synthetic local Panel/Edge/upstream fixture"]
async fn local_release_check_api_mask_edit_once() {
    run_case("api-mask-edit", false, true).await;
}

async fn run_case(case: &str, stream: bool, edit: bool) {
    assert_eq!(std::env::var("ORBIT_RELEASE_CHECK").as_deref(), Ok("1"));
    let root = PathBuf::from(std::env::var("ORBIT_RELEASE_CHECK_ROOT").unwrap());
    assert!(root.is_absolute());
    let root = root.canonicalize().unwrap();
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json")).unwrap()).unwrap();
    assert_eq!(fixture["synthetic"], true, "synthetic fixture required");
    let port = fixture["edge_port"]
        .as_u64()
        .and_then(|v| u16::try_from(v).ok())
        .filter(|v| *v != 0)
        .expect("explicit local Edge TLS port required");
    let slug = fixture["model_slug"].as_str().unwrap();
    let ca = std::fs::read(root.join("certs/edge.pem")).unwrap();
    let evidence = root.join("evidence");
    std::fs::create_dir_all(&evidence).unwrap();
    // Keep the reservation even on failure. An already recorded case is never
    // posted again by rerunning this test, including after an ambiguous result.
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(evidence.join(format!("orbit-{case}.json")))
        .expect("never replay a reserved image case");
    output
        .write_all(b"{\"state\":\"reserved\",\"synthetic\":true}\n")
        .unwrap();
    output.sync_all().unwrap();

    let (_dir, service, _vault, mut record, install) = setup();
    let mut native =
        Arc::try_unwrap(service).unwrap_or_else(|_| panic!("exclusive fixture service required"));
    native.http = Http::for_local_tls_acceptance(&ca, port).unwrap();
    let service = Arc::new_cyclic(|weak| {
        native.self_reference = weak.clone();
        native
    });
    record.view.edge_base_url = Some(format!("https://orbit-e2e.test:{port}/v1"));
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
    snapshot.task.kind = if edit { "edit" } else { "generation" }.into();
    snapshot.task.requested_model = slug.into();
    snapshot.task.requested_model_label = Some("Synthetic release-check image".into());
    snapshot.task.request_id = None;
    snapshot.task.prompt = format!("Synthetic release-check {case}; controlled upstream only.");
    snapshot.edge_base_url = record.view.edge_base_url.clone().unwrap();
    snapshot.task.parameters = ImageParameters {
        n: Some(if stream { 2 } else { 1 }),
        quality: Some("auto".into()),
        size: Some("auto".into()),
        background: Some("opaque".into()),
        output_format: Some("png".into()),
        stream: Some(stream),
        partial_images: Some(if stream { 1 } else { 0 }),
        ..Default::default()
    };
    if edit {
        for (name, value) in [("reference", 255), ("mask", 0)] {
            let source = service.store.root.join(format!("{name}.png"));
            std::fs::write(&source, export_png(16, 16, value)).unwrap();
            let input = images::import_input(&service.store, &source, &record.view).unwrap();
            let input = service.store.get("input", &input.id).unwrap().unwrap();
            if name == "mask" {
                snapshot.mask = Some(input);
            } else {
                snapshot.inputs.push(input);
            }
        }
    }
    let (model, catalog_source) = if root.join("catalog.json").exists() {
        let catalog: Value =
            serde_json::from_slice(&std::fs::read(root.join("catalog.json")).unwrap()).unwrap();
        let models = catalog["models"].as_array().unwrap();
        let raw = models.iter().find(|model| model["id"] == slug).unwrap();
        (
            map_model(raw, "synthetic-release-check").unwrap(),
            "local Panel projection",
        )
    } else {
        let api: Value = serde_json::from_str(include_str!(
            "../../../../../tests/contracts/images-api-catalog.json"
        ))
        .unwrap();
        let legacy: Value = serde_json::from_str(include_str!(
            "../../../../../tests/contracts/images-parameters.json"
        ))
        .unwrap();
        let model: ModelView = serde_json::from_value(json!({
            "id":slug,"label":"Synthetic image","provider_id":"openai",
            "revision":"synthetic-release-check","category":"image",
            "capabilities":["openai.images.generations","openai.images.edits"],
            "execution_profile":"codex-images-json-v1","image_billing_mode":"image_count",
            "reasoning_efforts":[],"support_status":"unverified",
            "image_parameters":legacy["codex_available_controls"],
            "image_api_parameters":api["models"]["gpt-image-2.6-flare"],
            "sales_price":null,"official_price":null
        }))
        .unwrap();
        (model, "frozen synthetic contract fixture")
    };
    let (limit, n, api) = crate::images_api::validate(
        &model,
        &snapshot.task.parameters,
        &snapshot.task.prompt,
        snapshot.inputs.len(),
        images::MAX_REQUEST_BYTES,
        snapshot.mask.is_some(),
        false,
    )
    .unwrap();
    assert_eq!(api, stream || edit);
    snapshot.maximum_request_bytes = limit;
    snapshot.maximum_results = n;
    snapshot.api_contract = api.then(|| images::API_CONTRACT.into());
    let request_hash = sha256(&images::request_bytes(&service.store, &snapshot).unwrap());
    service.persist_task(&snapshot).unwrap();
    let result = service.send_image_once(&mut snapshot).await;
    let mut report = json!({
        "synthetic":true,"case":case,"catalog_source":catalog_source,
        "actual_consumer":"Orbit Service::send_image_once over native reqwest TLS",
        "fixture_boundaries":["preseeded local session","in-memory FakeVault","controlled test upstream"],
        "expected_fixture_chain":["local Edge main","local Panel machine authorization and MySQL settlement","controlled upstream"],
        "outbound_client_media":"application/json",
        "expected_provider_edit_media":if edit {Some("application/json")}else{None},
        "request_sha256":request_hash,"request_id":snapshot.task.request_id,
        "state":snapshot.task.state,"parameters":snapshot.task.parameters,
        "api_contract":snapshot.api_contract,
        "error_code":result.as_ref().err().map(|e|e.code.clone()),"images":[]
    });
    if result.is_ok() {
        assert_eq!(snapshot.task.state, "completed");
        assert_eq!(snapshot.task.image_ids.len(), usize::from(n));
        let mut saved = vec![];
        for id in &snapshot.task.image_ids {
            let image: HistoryImage = service.store.get("image", id).unwrap().unwrap();
            let filename: String = service.store.get("image-file", id).unwrap().unwrap();
            let bytes = std::fs::read(images::owned_image_path(&service.store, &filename).unwrap())
                .unwrap();
            let pixels = images::decode_pixels(&bytes).unwrap();
            assert_eq!(pixels.dimensions(), (image.width, image.height));
            assert_eq!(image.mime, "image/png");
            saved.push(
                json!({"mime":image.mime,"width":image.width,"height":image.height,
                "bytes":bytes.len(),"sha256":sha256(&bytes)}),
            );
        }
        report["images"] = json!(saved);
        assert_eq!(
            service
                .send_image_once(&mut snapshot)
                .await
                .unwrap_err()
                .code,
            "task_state_conflict"
        );
        report["replay_prevented"] = true.into();
    }
    use std::io::{Seek, SeekFrom};
    output.seek(SeekFrom::Start(0)).unwrap();
    output.set_len(0).unwrap();
    output
        .write_all(&serde_json::to_vec_pretty(&report).unwrap())
        .unwrap();
    output.sync_all().unwrap();
    assert!(
        result.is_ok(),
        "inspect the reserved synthetic case evidence"
    );
}
