use orbit_core::{
    clipboard::{ClipboardImage, ImageClipboardSink},
    dto::*,
    error::{Result, fail},
    service::Service,
    updates,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

pub struct Native {
    pub service: Arc<Service>,
    pub _instance: orbit_core::runtime_lock::RuntimeLock,
}
struct NativeImageClipboard;
impl ImageClipboardSink for NativeImageClipboard {
    fn set_image(&mut self, image: ClipboardImage) -> Result<()> {
        let width = image.width();
        let height = image.height();
        let mut clipboard = arboard::Clipboard::new().map_err(|_| fail("clipboard_unavailable"))?;
        clipboard
            .set_image(arboard::ImageData {
                width,
                height,
                bytes: std::borrow::Cow::Owned(image.into_bytes()),
            })
            .map_err(|_| fail("clipboard_unavailable"))
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    connection_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshAccount {
    connection_id: String,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelLogin {
    request_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    connection_id: String,
    node_id: String,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeProbe {
    connection_id: String,
    expected_revision: u64,
    node_ids: Vec<String>,
    probe_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelNodeProbes {
    probe_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    connection_id: String,
    expected_revision: u64,
    client: ClientKind,
    claude_mapping: Option<ClaudeModelMapping>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    preview_id: String,
    confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchSnapshotRequest {
    snapshot_id: String,
    #[serde(default, deserialize_with = "restore_coordination_selection")]
    coordination: Option<ClaudeRestoreCoordination>,
}
fn restore_coordination_selection<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<ClaudeRestoreCoordination>, D::Error> {
    match String::deserialize(deserializer)?.as_str() {
        "claude_main_preferences_v1" => Ok(Some(ClaudeRestoreCoordination::MainPreferencesV1)),
        _ => Err(serde::de::Error::custom("unsupported restore coordination")),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateTask {
    connection_id: String,
    expected_revision: u64,
    model_id: String,
    prompt: String,
    parameters: ImageParameters,
    input_ids: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    task_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    image_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryInput {
    image_id: String,
    connection_id: String,
    expected_revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteImages {
    image_ids: Vec<String>,
    confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    patch: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    preview_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    version: String,
    confirmed: bool,
}
#[derive(Serialize)]
pub struct Saved {
    saved: bool,
}
#[derive(Serialize)]
pub struct Copied {
    copied: bool,
}
#[derive(Serialize)]
pub struct Deleted {
    deleted: u32,
}
#[derive(Clone, Serialize)]
pub struct InputsEvent {
    pub account_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub inputs: Vec<ImageInput>,
}
#[derive(Clone, Serialize)]
pub struct ScopedErrorEvent {
    pub error: PublicError,
    pub account_id: Option<String>,
    pub connection_id: Option<String>,
    pub connection_revision: Option<u64>,
}

#[tauri::command]
pub fn bootstrap(native: State<'_, Native>, request: Empty) -> Result<BootstrapView> {
    let _ = request;
    native.service.bootstrap()
}
#[tauri::command]
pub async fn refresh_account(
    native: State<'_, Native>,
    request: RefreshAccount,
) -> Result<BootstrapView> {
    native
        .service
        .refresh_account(&request.connection_id, request.expected_revision)
        .await
}
#[tauri::command]
pub async fn start_login(
    app: AppHandle,
    native: State<'_, Native>,
    request: Empty,
) -> Result<LoginStart> {
    let _ = request;
    let (view, url) = native
        .service
        .start_login(orbit_core::network::PRODUCTION_PANEL_ORIGIN)
        .await?;
    if app.opener().open_url(url, None::<&str>).is_err() {
        let error = fail("browser_unavailable");
        native.service.fail_login(&view.request_id, error.clone())?;
        return Err(error);
    }
    Ok(view)
}
#[tauri::command]
pub fn cancel_login(native: State<'_, Native>, request: CancelLogin) -> Result<BootstrapView> {
    native.service.cancel_login(&request.request_id)
}
#[tauri::command]
pub async fn logout(native: State<'_, Native>, request: Connection) -> Result<BootstrapView> {
    native.service.logout(&request.connection_id).await
}
#[tauri::command]
pub fn select_node(native: State<'_, Native>, request: Node) -> Result<BootstrapView> {
    native.service.select_node(
        &request.connection_id,
        &request.node_id,
        request.expected_revision,
    )
}
#[tauri::command]
pub async fn list_models(native: State<'_, Native>, request: Connection) -> Result<Vec<ModelView>> {
    native.service.list_models(&request.connection_id).await
}
#[tauri::command]
pub async fn probe_nodes(
    native: State<'_, Native>,
    request: NodeProbe,
) -> Result<Vec<NodeLatencyView>> {
    native
        .service
        .probe_nodes(
            &request.connection_id,
            request.expected_revision,
            &request.node_ids,
            &request.probe_id,
        )
        .await
}
#[tauri::command]
pub fn cancel_node_probes(native: State<'_, Native>, request: CancelNodeProbes) -> Result<()> {
    native.service.cancel_node_probes(&request.probe_id)
}
#[tauri::command]
pub async fn preview_client_config(
    native: State<'_, Native>,
    request: Preview,
) -> Result<ConfigPreview> {
    native
        .service
        .preview_switch_config(
            &request.connection_id,
            request.expected_revision,
            request.client,
            request.claude_mapping.as_ref(),
        )
        .await
}
#[tauri::command]
pub async fn apply_client_config(
    native: State<'_, Native>,
    request: Confirmation,
) -> Result<ConfigResult> {
    native
        .service
        .apply_client_config_refreshed(&request.preview_id, request.confirmed)
        .await
}
#[tauri::command]
pub async fn export_claude_import(
    app: AppHandle,
    native: State<'_, Native>,
    request: Confirmation,
) -> Result<Saved> {
    if !request.confirmed {
        return Err(fail("confirmation_required"));
    }
    let service = native.service.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name("orbit-claude-import-diagnostic.json")
            .add_filter("Claude official V2 body", &["json"])
            .blocking_save_file()
    })
    .await
    .map_err(|_| fail("file_dialog_unavailable"))?;
    let Some(file) = picked else {
        return Ok(Saved { saved: false });
    };
    let path = file.into_path().map_err(|_| fail("invalid_path"))?;
    tauri::async_runtime::spawn_blocking(move || {
        service.export_claude_import(&request.preview_id, request.confirmed, &path)
    })
    .await
    .map_err(|_| fail("file_dialog_unavailable"))??;
    Ok(Saved { saved: true })
}
#[tauri::command]
pub async fn preview_restore(
    native: State<'_, Native>,
    request: SwitchSnapshotRequest,
) -> Result<ConfigPreview> {
    let service = native.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.preview_restore_with_coordination(&request.snapshot_id, request.coordination)
    })
    .await
    .map_err(|_| fail("state_unavailable"))?
}
#[tauri::command]
pub async fn restore_client_config(
    native: State<'_, Native>,
    request: Confirmation,
) -> Result<ConfigResult> {
    let service = native.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.apply_client_config(&request.preview_id, request.confirmed, true)
    })
    .await
    .map_err(|_| fail("state_unavailable"))?
}
#[tauri::command]
pub async fn select_image_inputs(
    app: AppHandle,
    native: State<'_, Native>,
    request: Connection,
) -> Result<Vec<ImageInput>> {
    let service = native.service.clone();
    let frozen = service.connection_view(&request.connection_id)?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("PNG / JPEG / WebP", &["png", "jpg", "jpeg", "webp"])
            .blocking_pick_files()
    })
    .await
    .map_err(|_| fail("file_dialog_unavailable"))?;
    let Some(files) = picked else {
        return Ok(vec![]);
    };
    let paths = files
        .into_iter()
        .map(|file| file.into_path().map_err(|_| fail("invalid_path")))
        .collect::<Result<Vec<_>>>()?;
    let current = service.connection_view(&request.connection_id)?;
    if current.revision != frozen.revision || current.account_id != frozen.account_id {
        return Err(fail("connection_conflict"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        service.import_inputs(&request.connection_id, &paths)
    })
    .await
    .map_err(|_| fail("image_worker_failed"))?
}
#[tauri::command]
pub async fn use_history_as_input(
    native: State<'_, Native>,
    request: HistoryInput,
) -> Result<ImageInput> {
    let service = native.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.use_history_as_input(
            &request.image_id,
            &request.connection_id,
            request.expected_revision,
        )
    })
    .await
    .map_err(|_| fail("image_worker_failed"))?
}
#[tauri::command]
pub async fn create_image_task(native: State<'_, Native>, request: CreateTask) -> Result<TaskView> {
    let service = native.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.create_image_task(
            &request.connection_id,
            request.expected_revision,
            &request.model_id,
            request.prompt,
            request.parameters,
            request.input_ids,
        )
    })
    .await
    .map_err(|_| fail("image_worker_failed"))?
}
#[tauri::command]
pub fn cancel_image_task(native: State<'_, Native>, request: Task) -> Result<TaskView> {
    native.service.cancel_image_task(&request.task_id)
}
#[tauri::command]
pub async fn retry_local_save(native: State<'_, Native>, request: Task) -> Result<TaskView> {
    let service = native.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.retry_local_save(&request.task_id))
        .await
        .map_err(|_| fail("image_worker_failed"))?
}
#[tauri::command]
pub async fn save_image_as(
    app: AppHandle,
    native: State<'_, Native>,
    request: Image,
) -> Result<Saved> {
    let service = native.service.clone();
    let plan = service.begin_image_export(&request.image_id)?;
    let image = plan.image();
    let name = format!(
        "orbit-{}.{}",
        &image.id[..8],
        match image.mime.as_str() {
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => "png",
        }
    );
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_file_name(name).blocking_save_file()
    })
    .await
    .map_err(|_| fail("file_dialog_unavailable"))?;
    let Some(file) = picked else {
        return Ok(Saved { saved: false });
    };
    let path = file.into_path().map_err(|_| fail("invalid_path"))?;
    if service.paths.isolated && !path.starts_with(&service.store.root) {
        return Err(fail("integration_authorization_required"));
    }
    tauri::async_runtime::spawn_blocking(move || plan.read()?.save(&path))
        .await
        .map_err(|_| fail("image_worker_failed"))??;
    Ok(Saved { saved: true })
}
#[tauri::command]
pub async fn copy_image(native: State<'_, Native>, request: Image) -> Result<Copied> {
    let service = native.service.clone();
    if service.paths.isolated {
        return Err(fail("integration_authorization_required"));
    }
    let plan = service.begin_image_export(&request.image_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        plan.decode()?.commit(&mut NativeImageClipboard)?;
        Ok(Copied { copied: true })
    })
    .await
    .map_err(|_| fail("image_worker_failed"))?
}
#[tauri::command]
pub fn delete_history(native: State<'_, Native>, request: DeleteImages) -> Result<Deleted> {
    Ok(Deleted {
        deleted: native
            .service
            .delete_history(&request.image_ids, request.confirmed)?,
    })
}
#[tauri::command]
pub fn reuse_task_parameters(native: State<'_, Native>, request: Task) -> Result<Value> {
    native.service.reuse_task_parameters(&request.task_id)
}
#[tauri::command]
pub fn update_settings(native: State<'_, Native>, request: Settings) -> Result<SettingsView> {
    native.service.update_settings(request.patch)
}
#[tauri::command]
pub fn prepare_diagnostics(
    native: State<'_, Native>,
    request: Empty,
) -> Result<DiagnosticsPreview> {
    let _ = request;
    native.service.prepare_diagnostics()
}
#[tauri::command]
pub async fn export_diagnostics(
    app: AppHandle,
    native: State<'_, Native>,
    request: Diagnostic,
) -> Result<Saved> {
    let service = native.service.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name("orbit-diagnostics.txt")
            .blocking_save_file()
    })
    .await
    .map_err(|_| fail("file_dialog_unavailable"))?;
    let Some(file) = picked else {
        return Ok(Saved { saved: false });
    };
    let path = file.into_path().map_err(|_| fail("invalid_path"))?;
    if service.paths.isolated && !path.starts_with(&service.store.root) {
        return Err(fail("integration_authorization_required"));
    }
    service.export_diagnostics(&request.preview_id, &path)?;
    Ok(Saved { saved: true })
}
#[tauri::command]
pub async fn check_update(native: State<'_, Native>, request: Empty) -> Result<UpdateView> {
    let _operation = native.service.begin_mutation()?;
    let _ = request;
    updates::check(&native.service.http, &native.service.store).await
}
#[tauri::command]
pub async fn download_update(native: State<'_, Native>, request: Update) -> Result<UpdateView> {
    let _operation = native.service.begin_mutation()?;
    updates::download(
        &native.service.http,
        &native.service.store,
        &request.version,
        request.confirmed,
    )
    .await
}
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    native: State<'_, Native>,
    request: Update,
) -> Result<UpdateView> {
    if !request.confirmed {
        return Err(fail("confirmation_required"));
    }
    if updates::compiled_trust().is_none() {
        return Ok(updates::disabled());
    }
    if native.service.paths.isolated {
        return Err(fail("integration_authorization_required"));
    }
    #[cfg(target_os = "windows")]
    {
        let _ = app;
        Err(fail("windows_update_recovery_unverified"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _maintenance = native.service.begin_maintenance()?;
        let _lock = updates::exclusive_helper_lock(&native.service.store.root)?;
        let artifact = updates::verified_local_candidate(&native.service.store, &request.version)?;
        install_verified_candidate(&app, &native.service, artifact).await
    }
}
#[cfg(not(target_os = "windows"))]
async fn install_verified_candidate(
    app: &AppHandle,
    service: &Service,
    artifact: updates::VerifiedArtifact,
) -> Result<UpdateView> {
    #[cfg(target_os = "macos")]
    {
        let exe = std::env::current_exe().map_err(|_| fail("update_install_failed"))?;
        let current = exe
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .ok_or_else(|| fail("update_bundle_invalid"))?;
        let parent = current.parent().ok_or_else(|| fail("invalid_path"))?;
        let staging = parent.join(format!(".orbit-update-{}", orbit_core::identity::uuid()));
        let bundle = updates::extract_macos_artifact(&artifact, &staging)?;
        let prepared = orbit_core::update_recovery::prepare_macos(
            &service.store,
            current,
            &bundle,
            &artifact,
        )?;
        prepared.swap()?;
        app.restart();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, service, artifact);
        Err(fail("platform_unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_commands_keep_async_results_and_closed_request_schema() {
        async fn preview_signature(
            native: State<'_, Native>,
            request: SwitchSnapshotRequest,
        ) -> Result<ConfigPreview> {
            preview_restore(native, request).await
        }
        async fn restore_signature(
            native: State<'_, Native>,
            request: Confirmation,
        ) -> Result<ConfigResult> {
            restore_client_config(native, request).await
        }
        let _ = (preview_signature, restore_signature);
        assert!(
            serde_json::from_value::<SwitchSnapshotRequest>(
                serde_json::json!({"backup_group_id":"retired-backup"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<SwitchSnapshotRequest>(
                serde_json::json!({"snapshot_id":"synthetic-backup"})
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<Confirmation>(
                serde_json::json!({"preview_id":"synthetic-preview","confirmed":true})
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<SwitchSnapshotRequest>(
                serde_json::json!({"snapshot_id":"synthetic-backup","path":"/unexpected"})
            )
            .is_err()
        );
        assert!(serde_json::from_value::<Confirmation>(serde_json::json!({"preview_id":"synthetic-preview","confirmed":true,"path":"/unexpected"})).is_err());
        let coordinated = serde_json::from_value::<SwitchSnapshotRequest>(serde_json::json!({
            "snapshot_id":"synthetic-backup", "coordination":"claude_main_preferences_v1"
        }))
        .unwrap();
        assert_eq!(
            coordinated.coordination,
            Some(ClaudeRestoreCoordination::MainPreferencesV1)
        );
        for mode in [
            serde_json::Value::Null,
            serde_json::json!("automatic"),
            serde_json::json!(true),
            serde_json::json!({"path":"/unexpected"}),
            serde_json::json!({"claude_main_preferences_v1":null}),
        ] {
            assert!(
                serde_json::from_value::<SwitchSnapshotRequest>(
                    serde_json::json!({"snapshot_id":"synthetic-backup","coordination":mode})
                )
                .is_err()
            );
        }
        assert!(serde_json::from_value::<Confirmation>(serde_json::json!({"preview_id":"synthetic-preview","confirmed":true,"coordination":"claude_main_preferences_v1"})).is_err());
    }

    #[test]
    fn production_panel_login_empty_request_rejects_caller_selected_origin() {
        async fn typed_command(app: AppHandle, native: State<'_, Native>, request: Empty) {
            let _ = start_login(app, native, request).await;
        }
        let _ = typed_command;
        assert!(serde_json::from_value::<Empty>(serde_json::json!({})).is_ok());
        for input in [
            serde_json::json!({"panel_origin":"https://panel.example.test"}),
            serde_json::json!({"panel_origin":orbit_core::network::PRODUCTION_PANEL_ORIGIN}),
            serde_json::json!({"origin":"https://panel.example.test"}),
        ] {
            assert!(serde_json::from_value::<Empty>(input).is_err());
        }
    }
}
