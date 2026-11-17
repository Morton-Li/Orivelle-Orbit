use super::*;

#[test]
fn local_image_preparation_does_not_require_a_key_node_or_inference_permission() {
    let (dir, service, _vault, mut record, _install) = setup();
    let image = clipboard_fixture(&service, &record);
    record.view.node_id = None;
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
    let path = dir.path().join("reference.png");
    std::fs::write(&path, export_png(16, 16, 0)).unwrap();
    let input = service.import_inputs(&record.view.id, &[path]).unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(input[0].mime, "image/png");
    let reused = service
        .use_history_as_input(&image.id, &record.view.id, record.view.revision)
        .unwrap();
    assert_eq!(reused.mime, "image/png");
    assert_eq!(
        service
            .create_image_task(
                &record.view.id,
                record.view.revision,
                "synthetic-image",
                "Synthetic edit".into(),
                ImageParameters::default(),
                vec![reused.id],
            )
            .unwrap_err()
            .code,
        "connection_conflict"
    );
    assert_eq!(
        service.store.list::<TaskView>("task", None).unwrap().len(),
        1
    );
}

#[test]
fn history_titles_persist_without_changing_request_identity_and_survive_save_retries() {
    let (_dir, service, _vault, record, _install) = setup();
    let image = clipboard_fixture(&service, &record);
    let request_id = service.scoped_task(&image.task_id).unwrap().task.request_id;
    let mut legacy = serde_json::to_value(&image).unwrap();
    legacy.as_object_mut().unwrap().remove("title");
    assert!(
        serde_json::from_value::<HistoryImage>(legacy)
            .unwrap()
            .title
            .is_none()
    );

    let renamed = service
        .rename_history_image(&image.id, "  海报草稿  ")
        .unwrap();
    assert_eq!(renamed.title.as_deref(), Some("海报草稿"));
    let reopened = Store::open_readonly(&service.store.root).unwrap();
    assert_eq!(
        reopened
            .get::<HistoryImage>("image", &image.id)
            .unwrap()
            .unwrap()
            .title,
        renamed.title
    );
    let snapshot = service.scoped_task(&image.task_id).unwrap();
    assert_eq!(snapshot.task.request_id, request_id);
    assert_eq!(
        images::finish_local_save(&service.store, &snapshot).unwrap()[0]
            .title
            .as_deref(),
        Some("海报草稿")
    );
    for invalid in ["长".repeat(121), "line\nbreak".into(), "a\0b".into()] {
        assert_eq!(
            service
                .rename_history_image(&image.id, &invalid)
                .unwrap_err()
                .code,
            "invalid_image_title"
        );
    }
    assert_eq!(
        service.scoped_image(&image.id).unwrap().0.title,
        renamed.title
    );
    assert!(
        service
            .rename_history_image(&image.id, " ")
            .unwrap()
            .title
            .is_none()
    );
    assert_eq!(
        service.scoped_task(&image.task_id).unwrap().task.request_id,
        request_id
    );
}

#[test]
fn local_image_preparation_and_renaming_enforce_session_revision_and_ownership() {
    let (dir, service, _vault, record, _install) = setup();
    let image = clipboard_fixture(&service, &record);
    assert_eq!(
        service
            .use_history_as_input(&image.id, &record.view.id, record.view.revision + 1)
            .unwrap_err()
            .code,
        "connection_conflict"
    );
    let path = dir.path().join("reference.png");
    std::fs::write(&path, export_png(16, 16, 0)).unwrap();
    let input = service
        .import_inputs(&record.view.id, std::slice::from_ref(&path))
        .unwrap()
        .remove(0);
    let mut other = record.clone();
    other.view.id = uuid();
    other.view.account_id = uuid();
    service
        .store
        .put("connection", &other.view.id, &other.view.account_id, &other)
        .unwrap();
    service
        .store
        .set_meta("active_connection", &other.view.id)
        .unwrap();
    assert_eq!(
        service
            .import_inputs(&record.view.id, std::slice::from_ref(&path))
            .unwrap_err()
            .code,
        "connection_conflict"
    );
    assert_eq!(
        service
            .use_history_as_input(&image.id, &other.view.id, other.view.revision)
            .unwrap_err()
            .code,
        "account_scope_mismatch"
    );
    assert_eq!(
        service
            .rename_history_image(&image.id, "Other account")
            .unwrap_err()
            .code,
        "account_scope_mismatch"
    );
    assert_eq!(
        service
            .refresh_image_inputs(&other.view.id, other.view.revision, &[input.id])
            .unwrap_err()
            .code,
        "input_scope_mismatch"
    );
    let mut expired = record.clone();
    expired.session_expires_at = Some(1);
    service
        .store
        .put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            &expired,
        )
        .unwrap();
    service
        .store
        .set_meta("active_connection", &record.view.id)
        .unwrap();
    assert_eq!(
        service
            .import_inputs(&record.view.id, &[path])
            .unwrap_err()
            .code,
        "connection_conflict"
    );
    assert_eq!(
        service
            .rename_history_image(&image.id, "Expired")
            .unwrap_err()
            .code,
        "session_expired"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepared_images_refresh_their_scope_and_preview_after_key_selection_before_submission() {
    let (dir, service, _vault, mut record, install) = setup();
    let path = dir.path().join("reference.png");
    std::fs::write(&path, export_png(16, 16, 0)).unwrap();
    let input = service
        .import_inputs(&record.view.id, &[path])
        .unwrap()
        .remove(0);
    let old_url = input.preview_url.clone().unwrap();
    let old_path = url::Url::parse(&old_url).unwrap().path().to_owned();
    assert!(service.read_preview_asset(&old_path).is_ok());
    service
        .accept_credential(
            &mut record,
            &install,
            serde_json::from_value(golden()["credential_reply"]["data"].clone()).unwrap(),
        )
        .unwrap();
    record.view.revision += 1;
    service
        .store
        .put(
            "connection",
            &record.view.id,
            &record.view.account_id,
            &record,
        )
        .unwrap();
    assert!(service.read_preview_asset(&old_path).is_err());
    let refreshed = service
        .refresh_image_inputs(
            &record.view.id,
            record.view.revision,
            std::slice::from_ref(&input.id),
        )
        .unwrap();
    let new_url = refreshed[0].preview_url.as_ref().unwrap();
    assert_ne!(new_url, &old_url);
    assert!(
        service
            .read_preview_asset(url::Url::parse(new_url).unwrap().path())
            .is_ok()
    );
    let model_id = service
        .account_cache(&record)
        .unwrap()
        .models
        .into_iter()
        .find(|model| {
            model
                .capabilities
                .iter()
                .any(|capability| capability == "openai.images.edits")
        })
        .unwrap()
        .id;
    // This single-threaded test examines admission before its queued worker runs.
    let task = service
        .create_image_task(
            &record.view.id,
            record.view.revision,
            &model_id,
            "Synthetic edit after choosing a key".into(),
            ImageParameters::default(),
            vec![input.id.clone()],
        )
        .unwrap();
    let snapshot = service.scoped_task(&task.id).unwrap();
    assert_eq!(snapshot.inputs[0].view.id, input.id);
    assert_eq!(snapshot.inputs[0].connection_revision, record.view.revision);
    assert_eq!(snapshot.task.kind, "edit");
}
