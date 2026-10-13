use orbit_core::{
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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    panel_origin: String,
}
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
pub struct DeleteConnection {
    connection_id: String,
    expected_revision: u64,
    expected_credential_revision: Option<String>,
    revoke_inference_key: bool,
    confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeKey {
    connection_id: String,
    expected_credential_revision: String,
    confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    connection_id: String,
    expected_revision: u64,
    client: ClientKind,
    model_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    preview_id: String,
    confirmed: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    backup_group_id: String,
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
pub struct Installation {
    installation_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Forget {
    preview_id: String,
    acknowledged_backup_loss: bool,
    confirmed: bool,
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
    request: Origin,
) -> Result<LoginStart> {
    let (view, url) = native.service.start_login(&request.panel_origin).await?;
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
pub fn select_connection(native: State<'_, Native>, request: Connection) -> Result<BootstrapView> {
    native.service.select_connection(&request.connection_id)
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
pub async fn delete_connection(
    native: State<'_, Native>,
    request: DeleteConnection,
) -> Result<BootstrapView> {
    native
        .service
        .delete_connection(
            &request.connection_id,
            request.expected_revision,
            request.expected_credential_revision.as_deref(),
            request.revoke_inference_key,
            request.confirmed,
        )
        .await
}
#[tauri::command]
pub async fn revoke_inference_key(
    native: State<'_, Native>,
    request: RevokeKey,
) -> Result<BootstrapView> {
    native
        .service
        .revoke_inference_key(
            &request.connection_id,
            &request.expected_credential_revision,
            request.confirmed,
        )
        .await
}
#[tauri::command]
pub async fn list_models(native: State<'_, Native>, request: Connection) -> Result<Vec<ModelView>> {
    native.service.list_models(&request.connection_id).await
}
#[tauri::command]
pub async fn preview_client_config(
    native: State<'_, Native>,
    request: Preview,
) -> Result<ConfigPreview> {
    let _ = native.service.list_models(&request.connection_id).await?;
    native.service.preview_client_config(
        &request.connection_id,
        request.expected_revision,
        request.client,
        &request.model_id,
    )
}
#[tauri::command]
pub fn apply_client_config(
    native: State<'_, Native>,
    request: Confirmation,
) -> Result<ConfigResult> {
    native
        .service
        .apply_client_config(&request.preview_id, request.confirmed, false)
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
pub fn preview_restore(native: State<'_, Native>, request: Backup) -> Result<ConfigPreview> {
    native.service.preview_restore(&request.backup_group_id)
}
#[tauri::command]
pub fn restore_client_config(
    native: State<'_, Native>,
    request: Confirmation,
) -> Result<ConfigResult> {
    native
        .service
        .apply_client_config(&request.preview_id, request.confirmed, true)
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
pub fn create_image_task(native: State<'_, Native>, request: CreateTask) -> Result<TaskView> {
    native.service.create_image_task(
        &request.connection_id,
        request.expected_revision,
        &request.model_id,
        request.prompt,
        request.parameters,
        request.input_ids,
    )
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
    let (image, _) = service.image_for_native_export(&request.image_id)?;
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
    tauri::async_runtime::spawn_blocking(move || service.export_image(&request.image_id, &path))
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
    let (_, path) = service.image_for_native_export(&request.image_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _permit = service.image_memory_permit()?;
        let bytes = orbit_core::images::bounded_read(&path, orbit_core::images::MAX_IMAGE_BYTES)?;
        let _ = orbit_core::images::validate_image(&bytes)?;
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| fail("invalid_image"))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(orbit_core::images::MAX_DECODE_BYTES);
        limits.max_image_width = Some(orbit_core::images::MAX_DIMENSION);
        limits.max_image_height = Some(orbit_core::images::MAX_DIMENSION);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| fail("invalid_image"))?
            .into_rgba8();
        let mut clipboard = arboard::Clipboard::new().map_err(|_| fail("clipboard_unavailable"))?;
        clipboard
            .set_image(arboard::ImageData {
                width: image.width() as usize,
                height: image.height() as usize,
                bytes: std::borrow::Cow::Owned(image.into_raw()),
            })
            .map_err(|_| fail("clipboard_unavailable"))?;
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
pub fn preview_forget_installation(
    native: State<'_, Native>,
    request: Installation,
) -> Result<ForgetInstallationPreview> {
    native
        .service
        .preview_forget_installation(&request.installation_id)
}
#[tauri::command]
pub async fn forget_installation(
    native: State<'_, Native>,
    request: Forget,
) -> Result<BootstrapView> {
    native
        .service
        .forget_installation(
            &request.preview_id,
            request.acknowledged_backup_loss,
            request.confirmed,
        )
        .await
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
    let _maintenance = native.service.begin_maintenance()?;
    let _lock = updates::exclusive_helper_lock(&native.service.store.root)?;
    let artifact = updates::verified_local_candidate(&native.service.store, &request.version)?;
    let _ = updates::prepare_schema_backup(&native.service.store, &artifact.candidate().version)?;
    install_verified_candidate(&app, &native.service, artifact).await
}
async fn install_verified_candidate(
    app: &AppHandle,
    service: &Service,
    artifact: updates::VerifiedArtifact,
) -> Result<UpdateView> {
    let candidate = artifact.candidate().clone();
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
        updates::validate_macos_bundle(&bundle, &candidate.version)?;
        let staged = parent.join(format!(
            ".orbit-staged-{}.app",
            orbit_core::identity::uuid()
        ));
        std::fs::rename(&bundle, &staged).map_err(|_| fail("update_staging_failed"))?;
        updates::swap_macos_bundle(current, &staged)?;
        service
            .store
            .set_meta("update_previous_bundle", &staged.display().to_string())?;
        service
            .store
            .set_meta("update_installed_version", &candidate.version)?;
        app.restart();
    }
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_updater::UpdaterExt;
        let trust = updates::compiled_trust().ok_or_else(|| fail("update_disabled"))?;
        let updater = app
            .updater_builder()
            .pubkey(trust.public_key)
            .endpoints(vec![
                url::Url::parse(trust.endpoint).map_err(|_| fail("invalid_update_channel"))?,
            ])
            .map_err(|_| fail("invalid_update_channel"))?
            .build()
            .map_err(|_| fail("update_unavailable"))?;
        let update = updater
            .check()
            .await
            .map_err(|_| fail("update_unavailable"))?
            .ok_or_else(|| fail("update_not_available"))?;
        if update.version != candidate.version {
            return Err(fail("update_artifact_changed"));
        }
        update
            .install(artifact.into_bytes())
            .map_err(|_| fail("update_install_failed"))?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app, service, candidate, artifact);
        return Err(fail("platform_unsupported"));
    }
    #[allow(unreachable_code)]
    Ok(UpdateView {
        status: "verified".into(),
        current_version: updates::CURRENT_VERSION.into(),
        version: Some(candidate.version.clone()),
        reason: Some("更新已安装，等待启动后 schema 验证。".into()),
    })
}
