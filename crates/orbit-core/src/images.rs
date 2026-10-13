use crate::{
    config::atomic_write,
    dto::*,
    error::{Result, fail},
    identity::{sha256, uuid},
    store::{Store, private_file},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{GenericImageView, ImageFormat, ImageReader};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_REQUEST_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_PROMPT_BYTES: usize = 32768;
pub const MAX_PIXELS: u64 = 16_777_216;
pub const MAX_DIMENSION: u32 = 8192;
pub const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Serialize, Deserialize)]
pub struct InputRecord {
    pub view: ImageInput,
    pub account_id: String,
    pub connection_id: String,
    pub connection_revision: u64,
    pub hash: String,
    pub file_name: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task: TaskView,
    pub panel_origin: String,
    pub edge_base_url: String,
    pub provider_id: String,
    pub installation_id: String,
    pub secret_reference: String,
    pub model_revision: String,
    pub maximum_request_bytes: u64,
    pub maximum_results: u8,
    pub inputs: Vec<InputRecord>,
}
#[derive(Clone)]
pub struct ValidatedImage {
    pub mime: &'static str,
    pub extension: &'static str,
    pub width: u32,
    pub height: u32,
    pub preview: Vec<u8>,
}

/// Verify the encoded format, dimensions and bounded allocation before decoding.
pub fn validate_image(bytes: &[u8]) -> Result<ValidatedImage> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(fail("image_limit_exceeded"));
    }
    let format = image::guess_format(bytes).map_err(|_| fail("invalid_image"))?;
    let (mime, extension) = match format {
        ImageFormat::Png => ("image/png", "png"),
        ImageFormat::Jpeg => ("image/jpeg", "jpg"),
        ImageFormat::WebP => ("image/webp", "webp"),
        _ => return Err(fail("invalid_image")),
    };
    let reader = ImageReader::with_format(Cursor::new(bytes), format);
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| fail("invalid_image"))?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || (width as u64) * (height as u64) > MAX_PIXELS
    {
        return Err(fail("image_limit_exceeded"));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|_| fail("invalid_image"))?;
    if decoded.dimensions() != (width, height) {
        return Err(fail("invalid_image"));
    }
    let thumb = decoded.thumbnail(1024, 1024);
    let mut output = Cursor::new(Vec::new());
    thumb
        .write_to(&mut output, ImageFormat::Png)
        .map_err(|_| fail("invalid_image"))?;
    let preview = output.into_inner();
    if preview.len() > 4 * 1024 * 1024 {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(ValidatedImage {
        mime,
        extension,
        width,
        height,
        preview,
    })
}
pub fn bounded_read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|_| fail("local_image_missing"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return Err(fail("image_limit_exceeded"));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(limit + 1).read_to_end(&mut bytes))
        .map_err(|_| fail("local_image_missing"))?;
    if bytes.len() as u64 > limit {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| fail("local_save_failed"))?;
    private_file(path)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| fail("local_save_failed"))?;
    Ok(())
}
pub fn owned_image_path(store: &Store, name: &str) -> Result<PathBuf> {
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.len() > 80 {
        return Err(fail("invalid_identifier"));
    }
    let path = store.root.join("images").join(name);
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(fail("invalid_path"));
    }
    Ok(path)
}
pub fn preview_url(store: &Store, id: &str) -> Result<String> {
    crate::identity::validate_uuid(id)?;
    let path = store.root.join("previews").join(format!("{id}.png"));
    // The webview receives only paths under the separate preview asset scope.
    Ok(format!(
        "{}/{}",
        if cfg!(windows) {
            "http://asset.localhost"
        } else {
            "asset://localhost"
        },
        url::form_urlencoded::byte_serialize(path.to_string_lossy().as_bytes()).collect::<String>()
    ))
}
pub fn disk_usage(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in fs::read_dir(root).map_err(|_| fail("local_storage_unavailable"))? {
        let entry = entry.map_err(|_| fail("local_storage_unavailable"))?;
        let meta = entry
            .metadata()
            .map_err(|_| fail("local_storage_unavailable"))?;
        if meta.is_file() {
            total = total
                .checked_add(meta.len())
                .ok_or_else(|| fail("image_limit_exceeded"))?;
        }
    }
    Ok(total)
}
pub fn ensure_disk_budget(store: &Store, new_bytes: u64) -> Result<()> {
    let settings = store.settings()?;
    let used = disk_usage(&store.root.join("images"))?
        .saturating_add(disk_usage(&store.root.join("inputs"))?)
        .saturating_add(disk_usage(&store.root.join("pending-save"))?);
    if used.saturating_add(new_bytes) > settings.image_disk_limit_bytes {
        return Err(fail("image_disk_limit"));
    }
    Ok(())
}
pub fn save_preview(store: &Store, id: &str, bytes: &[u8]) -> Result<Option<String>> {
    crate::identity::validate_uuid(id)?;
    let root = store.root.join("previews");
    let limit = store.settings()?.preview_cache_limit_bytes;
    if bytes.len() as u64 > limit {
        return Ok(None);
    }
    let mut entries = fs::read_dir(&root)
        .map_err(|_| fail("local_storage_unavailable"))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.metadata()
                .ok()
                .filter(|m| m.is_file() && !m.file_type().is_symlink())
                .map(|m| (e.path(), m.len(), m.modified().ok()))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|v| v.2);
    let mut total = entries.iter().map(|v| v.1).sum::<u64>();
    for (path, size, _) in entries {
        if total.saturating_add(bytes.len() as u64) <= limit {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
    if total.saturating_add(bytes.len() as u64) > limit {
        return Ok(None);
    }
    atomic_write(&root.join(format!("{id}.png")), bytes)?;
    Ok(Some(preview_url(store, id)?))
}
pub fn import_input(store: &Store, path: &Path, connection: &ConnectionView) -> Result<ImageInput> {
    let bytes = bounded_read(path, MAX_IMAGE_BYTES)?;
    let info = validate_image(&bytes)?;
    ensure_disk_budget(store, bytes.len() as u64)?;
    let id = uuid();
    let file_name = format!("{id}.{}", info.extension);
    write_new(&store.root.join("inputs").join(&file_name), &bytes)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image")
        .chars()
        .filter(|c| !c.is_control())
        .take(180)
        .collect();
    let view = ImageInput {
        id: id.clone(),
        name,
        mime: info.mime.into(),
        bytes: bytes.len() as u64,
        width: info.width,
        height: info.height,
        preview_url: save_preview(store, &id, &info.preview)?,
    };
    store.put(
        "input",
        &id,
        &connection.account_id,
        &InputRecord {
            view: view.clone(),
            account_id: connection.account_id.clone(),
            connection_id: connection.id.clone(),
            connection_revision: connection.revision,
            hash: sha256(&bytes),
            file_name,
        },
    )?;
    Ok(view)
}
#[derive(Serialize)]
struct ImageReference {
    image_url: String,
}
#[derive(Serialize)]
struct ImageRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    #[serde(flatten)]
    parameters: &'a ImageParameters,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<ImageReference>,
    response_format: &'static str,
}
pub fn validate_parameters(
    model: &ModelView,
    parameters: &ImageParameters,
    prompt: &str,
    inputs: usize,
    node_limit: u64,
) -> Result<(u64, u8)> {
    let constraints = model
        .image_parameters
        .as_ref()
        .ok_or_else(|| fail("unsupported_image_model"))?;
    if model.category != "image"
        || model.execution_profile.as_deref() != Some("codex-images-json-v1")
        || model.support_status == "unsupported"
        || prompt.trim().is_empty()
        || prompt.len() > MAX_PROMPT_BYTES
        || prompt.len() as u64 > constraints.prompt_maximum_bytes
        || inputs > 5
        || inputs > constraints.max_inputs as usize
    {
        return Err(fail("image_limit_exceeded"));
    }
    let n = parameters.n.unwrap_or(1);
    if n == 0 || n > 5 || n > constraints.max_results {
        return Err(fail("image_limit_exceeded"));
    }
    for (value, allowed) in [
        (&parameters.quality, &constraints.qualities),
        (&parameters.size, &constraints.sizes),
        (&parameters.background, &constraints.backgrounds),
    ] {
        if value.as_ref().is_some_and(|v| !allowed.contains(v)) {
            return Err(fail("unsupported_image_parameter"));
        }
    }
    if parameters
        .quality
        .as_ref()
        .is_some_and(|v| !["auto", "low", "medium", "high"].contains(&v.as_str()))
        || parameters
            .background
            .as_ref()
            .is_some_and(|v| !["auto", "opaque", "transparent"].contains(&v.as_str()))
    {
        return Err(fail("unsupported_image_parameter"));
    }
    if let Some(size) = &parameters.size {
        if size != "auto" {
            let dimensions = size
                .split('x')
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|_| fail("unsupported_image_parameter"))?;
            if dimensions.len() != 2
                || dimensions
                    .iter()
                    .any(|n| *n < 16 || *n > 4096 || *n % 16 != 0)
            {
                return Err(fail("unsupported_image_parameter"));
            }
        }
    }
    Ok((
        node_limit
            .min(constraints.maximum_request_bytes)
            .min(MAX_REQUEST_BYTES),
        n,
    ))
}
pub fn request_bytes(store: &Store, snapshot: &TaskSnapshot) -> Result<Vec<u8>> {
    let mut references = Vec::new();
    for input in &snapshot.inputs {
        if input.account_id != snapshot.task.account_id
            || input.connection_id != snapshot.task.connection_id
            || input.connection_revision != snapshot.task.connection_revision
        {
            return Err(fail("input_scope_mismatch"));
        }
        if input.file_name.contains('/')
            || input.file_name.contains('\\')
            || input.file_name.contains("..")
        {
            return Err(fail("invalid_identifier"));
        }
        let bytes = bounded_read(
            &store.root.join("inputs").join(&input.file_name),
            MAX_IMAGE_BYTES,
        )?;
        if sha256(&bytes) != input.hash {
            return Err(fail("input_changed"));
        }
        let format = image::guess_format(&bytes).map_err(|_| fail("invalid_image"))?;
        let mime = match format {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::WebP => "image/webp",
            _ => return Err(fail("invalid_image")),
        };
        if mime != input.view.mime {
            return Err(fail("invalid_image"));
        }
        references.push(ImageReference {
            image_url: format!("data:{mime};base64,{}", STANDARD.encode(&bytes)),
        });
    }
    let bytes = serde_json::to_vec(&ImageRequest {
        model: &snapshot.task.requested_model,
        prompt: &snapshot.task.prompt,
        parameters: &snapshot.task.parameters,
        images: references,
        response_format: "b64_json",
    })
    .map_err(|_| fail("serialization_failed"))?;
    if bytes.len() as u64 > snapshot.maximum_request_bytes.min(MAX_REQUEST_BYTES) {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(bytes)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageResponse {
    #[allow(dead_code)]
    created: u64,
    data: Vec<ImageData>,
    #[allow(dead_code)]
    #[serde(default)]
    background: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    quality: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    size: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImageData {
    b64_json: String,
    #[allow(dead_code)]
    #[serde(default)]
    generation_id: Option<String>,
}
pub struct ReceivedImage {
    pub id: String,
    pub bytes: Vec<u8>,
    pub info: ValidatedImage,
}
pub fn parse_response(bytes: &[u8], maximum_results: u8) -> Result<Vec<ReceivedImage>> {
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(fail("response_too_large"));
    }
    let response: ImageResponse =
        serde_json::from_slice(bytes).map_err(|_| fail("invalid_image_response"))?;
    if response.data.is_empty() || response.data.len() > maximum_results.min(5) as usize {
        return Err(fail("invalid_image_response"));
    }
    let mut images = Vec::new();
    for data in response.data {
        if data.b64_json.len() as u64 > ((MAX_IMAGE_BYTES + 2) / 3) * 4
            || data.b64_json.bytes().any(|b| b.is_ascii_whitespace())
        {
            return Err(fail("image_limit_exceeded"));
        }
        let bytes = STANDARD
            .decode(data.b64_json)
            .map_err(|_| fail("invalid_image_response"))?;
        let info = validate_image(&bytes)?;
        images.push(ReceivedImage {
            id: uuid(),
            bytes,
            info,
        });
    }
    Ok(images)
}
#[derive(Clone, Serialize, Deserialize)]
pub struct PendingImage {
    pub id: String,
    pub task_id: String,
    pub account_id: String,
    pub file_name: String,
    pub hash: String,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct SaveManifest {
    pub task_id: String,
    pub account_id: String,
    pub panel_origin: String,
    pub phase: String,
    pub images: Vec<PendingImage>,
}
pub fn local_material_available(store: &Store, snapshot: &TaskSnapshot) -> Result<bool> {
    let Some(manifest) = store.get::<SaveManifest>("local-save-manifest", &snapshot.task.id)?
    else {
        return Ok(false);
    };
    if manifest.task_id != snapshot.task.id
        || manifest.account_id != snapshot.task.account_id
        || manifest.panel_origin != snapshot.panel_origin
        || manifest.images.is_empty()
        || manifest.images.len() > snapshot.maximum_results as usize
    {
        return Ok(false);
    }
    for image in manifest.images {
        crate::identity::validate_uuid(&image.id)?;
        let source = store.root.join("pending-save").join(&image.file_name);
        let target = owned_image_path(store, &image.file_name)?;
        let bytes = match bounded_read(
            if source.exists() { &source } else { &target },
            MAX_IMAGE_BYTES,
        ) {
            Ok(b) => b,
            Err(_) => return Ok(false),
        };
        if sha256(&bytes) != image.hash || bytes.len() as u64 != image.bytes {
            return Ok(false);
        }
    }
    Ok(true)
}
/// This is local result material only. There is no endpoint or mechanism to fetch it again.
pub fn stage_received(
    store: &Store,
    snapshot: &TaskSnapshot,
    images: &[ReceivedImage],
) -> Result<Vec<PendingImage>> {
    let mut out = images
        .iter()
        .map(|image| {
            let id = image.id.clone();
            PendingImage {
                id: id.clone(),
                task_id: snapshot.task.id.clone(),
                account_id: snapshot.task.account_id.clone(),
                file_name: format!("{id}.{}", image.info.extension),
                hash: sha256(&image.bytes),
                mime: image.info.mime.into(),
                width: image.info.width,
                height: image.info.height,
                bytes: image.bytes.len() as u64,
            }
        })
        .collect::<Vec<_>>();
    if out.is_empty() || out.len() > snapshot.maximum_results as usize {
        return Err(fail("invalid_image_response"));
    }
    let total = out
        .iter()
        .filter(|i| {
            !store.root.join("pending-save").join(&i.file_name).exists()
                && !store.root.join("images").join(&i.file_name).exists()
        })
        .map(|i| i.bytes)
        .sum();
    // Persist the complete observed result set before writing its first binary file.
    store.put(
        "local-save-manifest",
        &snapshot.task.id,
        &snapshot.task.account_id,
        &SaveManifest {
            task_id: snapshot.task.id.clone(),
            account_id: snapshot.task.account_id.clone(),
            panel_origin: snapshot.panel_origin.clone(),
            phase: "received".into(),
            images: out.clone(),
        },
    )?;
    ensure_disk_budget(store, total)?;
    out.clear();
    for image in images {
        validate_uuid_image(&image.id)?;
        let id = image.id.clone();
        let file_name = format!("{id}.{}", image.info.extension);
        let path = store.root.join("pending-save").join(&file_name);
        if path.exists() {
            if sha256(&bounded_read(&path, MAX_IMAGE_BYTES)?) != sha256(&image.bytes) {
                atomic_write(&path, &image.bytes)?;
            }
        } else {
            write_new(&path, &image.bytes)?;
        }
        let pending = PendingImage {
            id: id.clone(),
            task_id: snapshot.task.id.clone(),
            account_id: snapshot.task.account_id.clone(),
            file_name,
            hash: sha256(&image.bytes),
            mime: image.info.mime.into(),
            width: image.info.width,
            height: image.info.height,
            bytes: image.bytes.len() as u64,
        };
        store.put("pending-image", &id, &snapshot.task.account_id, &pending)?;
        out.push(pending);
    }
    Ok(out)
}
fn validate_uuid_image(id: &str) -> Result<()> {
    crate::identity::validate_uuid(id)
}
pub fn finish_local_save(store: &Store, snapshot: &TaskSnapshot) -> Result<Vec<HistoryImage>> {
    let mut manifest = store
        .get::<SaveManifest>("local-save-manifest", &snapshot.task.id)?
        .ok_or_else(|| fail("local_result_incomplete"))?;
    if manifest.task_id != snapshot.task.id
        || manifest.account_id != snapshot.task.account_id
        || manifest.panel_origin != snapshot.panel_origin
        || manifest.images.is_empty()
        || manifest.images.len() > snapshot.maximum_results as usize
    {
        return Err(fail("local_result_incomplete"));
    }
    // Missing any observed material stays incomplete, even if another image was saved.
    for item in &manifest.images {
        crate::identity::validate_uuid(&item.id)?;
        let source = store.root.join("pending-save").join(&item.file_name);
        let target = owned_image_path(store, &item.file_name)?;
        let bytes = bounded_read(
            if source.exists() { &source } else { &target },
            MAX_IMAGE_BYTES,
        )
        .map_err(|_| fail("local_result_incomplete"))?;
        if bytes.len() as u64 != item.bytes || sha256(&bytes) != item.hash {
            return Err(fail("local_result_incomplete"));
        }
    }
    let pending = manifest.images.clone();
    let mut out = Vec::new();
    for item in pending {
        let source = store.root.join("pending-save").join(&item.file_name);
        let target = owned_image_path(store, &item.file_name)?;
        let bytes = if source.exists() {
            bounded_read(&source, MAX_IMAGE_BYTES)?
        } else {
            bounded_read(&target, MAX_IMAGE_BYTES)?
        };
        if sha256(&bytes) != item.hash {
            return Err(fail("local_image_corrupt"));
        }
        let info = validate_image(&bytes)?;
        if !target.exists() {
            fs::rename(&source, &target).map_err(|_| fail("local_save_failed"))?;
            private_file(&target)?;
        }
        let view = HistoryImage {
            id: item.id.clone(),
            task_id: item.task_id.clone(),
            account_id: item.account_id.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
            requested_model: snapshot.task.requested_model.clone(),
            observed_model: None,
            mime: item.mime,
            width: item.width,
            height: item.height,
            bytes: item.bytes,
            preview_url: save_preview(store, &item.id, &info.preview)?,
        };
        store.put("image", &view.id, &view.account_id, &view)?;
        store.put("image-file", &view.id, &view.account_id, &item.file_name)?;
        out.push(view);
    }
    manifest.phase = "completed".into();
    store.put(
        "local-save-manifest",
        &snapshot.task.id,
        &snapshot.task.account_id,
        &manifest,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(16, 16)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }
    fn snapshot() -> TaskSnapshot {
        TaskSnapshot {
            task: TaskView {
                id: uuid(),
                account_id: "a".into(),
                connection_id: uuid(),
                connection_revision: 1,
                credential_revision: "k".into(),
                node_id: "n".into(),
                kind: "generation".into(),
                requested_model: "m".into(),
                prompt: "synthetic".into(),
                parameters: ImageParameters {
                    n: Some(2),
                    ..Default::default()
                },
                state: "saving".into(),
                local_save_available: true,
                started_at: None,
                request_id: None,
                result_state: "unconfirmed".into(),
                outcome: None,
                charge_state: "unavailable".into(),
                charges: vec![],
                image_ids: vec![],
                error: None,
            },
            panel_origin: "https://panel.test".into(),
            edge_base_url: "https://edge.test/v1".into(),
            provider_id: "openai".into(),
            installation_id: uuid(),
            secret_reference: "opaque".into(),
            model_revision: "r".into(),
            maximum_request_bytes: 1024,
            maximum_results: 2,
            inputs: vec![],
        }
    }
    #[test]
    fn interrupted_two_image_material_cannot_become_single_image_success() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let snap = snapshot();
        let received = vec![
            ReceivedImage {
                id: uuid(),
                bytes: png(),
                info: validate_image(&png()).unwrap(),
            },
            ReceivedImage {
                id: uuid(),
                bytes: png(),
                info: validate_image(&png()).unwrap(),
            },
        ];
        stage_received(&store, &snap, &received).unwrap();
        let manifest = store
            .get::<SaveManifest>("local-save-manifest", &snap.task.id)
            .unwrap()
            .unwrap();
        let missing = store
            .root
            .join("pending-save")
            .join(&manifest.images[1].file_name);
        let keep = fs::read(&missing).unwrap();
        fs::remove_file(&missing).unwrap();
        assert!(!local_material_available(&store, &snap).unwrap());
        assert!(
            matches!(finish_local_save(&store,&snap),Err(e)if e.code=="local_result_incomplete")
        );
        assert!(
            store
                .list::<HistoryImage>("image", Some("a"))
                .unwrap()
                .is_empty()
        );
        fs::write(&missing, keep).unwrap();
        assert!(local_material_available(&store, &snap).unwrap());
        assert_eq!(finish_local_save(&store, &snap).unwrap().len(), 2);
    }
    #[test]
    fn startup_synchronizes_saving_task_and_snapshot_without_network_replay() {
        let dir = tempfile::tempdir().unwrap();
        let snap = snapshot();
        {
            let store = Store::open(dir.path()).unwrap();
            stage_received(
                &store,
                &snap,
                &[
                    ReceivedImage {
                        id: uuid(),
                        bytes: png(),
                        info: validate_image(&png()).unwrap(),
                    },
                    ReceivedImage {
                        id: uuid(),
                        bytes: png(),
                        info: validate_image(&png()).unwrap(),
                    },
                ],
            )
            .unwrap();
            store.put("task", &snap.task.id, "a", &snap.task).unwrap();
            store
                .put("task-snapshot", &snap.task.id, "a", &snap)
                .unwrap();
        }
        let store = Store::open(dir.path()).unwrap();
        let task = store
            .get::<TaskView>("task", &snap.task.id)
            .unwrap()
            .unwrap();
        let recovered = store
            .get::<TaskSnapshot>("task-snapshot", &snap.task.id)
            .unwrap()
            .unwrap();
        assert_eq!(task.state, "unconfirmed");
        assert_eq!(recovered.task.state, "unconfirmed");
        assert!(task.local_save_available);
        assert_eq!(finish_local_save(&store, &recovered).unwrap().len(), 2);
    }
    #[test]
    fn jpeg_is_never_force_labeled_png() {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(16, 32)
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .unwrap();
        let info = validate_image(&bytes.into_inner()).unwrap();
        assert_eq!(info.mime, "image/jpeg");
        assert_eq!(info.extension, "jpg");
        assert_eq!((info.width, info.height), (16, 32));
    }
    #[test]
    fn detects_actual_format_and_rejects_svg() {
        let info = validate_image(&png()).unwrap();
        assert_eq!((info.mime, info.width, info.height), ("image/png", 16, 16));
        assert!(validate_image(b"<svg/>").is_err());
    }
    #[test]
    fn response_bounds_and_no_model_inference() {
        let payload = serde_json::json!({"created":1,"data":[{"b64_json":STANDARD.encode(png())}]});
        let parsed = parse_response(&serde_json::to_vec(&payload).unwrap(), 1).unwrap();
        assert_eq!(parsed.len(), 1);
        assert!(parse_response(br#"{"created":1,"data":[{"b64_json":"bad!"}]}"#, 1).is_err());
        assert!(parse_response(&serde_json::to_vec(&payload).unwrap(), 0).is_err());
    }
    #[test]
    fn full_request_includes_base64_and_utf8_prompt_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let connection = ConnectionView {
            id: uuid(),
            label: "fixture".into(),
            panel_origin: "https://panel.test".into(),
            account_id: "a".into(),
            node_id: Some("n".into()),
            node_label: None,
            edge_base_url: Some("https://edge.test/v1".into()),
            provider_id: "openai".into(),
            revision: 1,
            credential_revision: Some("k1".into()),
            permission_state: "allowed".into(),
        };
        let file = dir.path().join("input.png");
        fs::write(&file, png()).unwrap();
        let view = import_input(&store, &file, &connection).unwrap();
        let input = store
            .get::<InputRecord>("input", &view.id)
            .unwrap()
            .unwrap();
        let task = TaskView {
            id: uuid(),
            account_id: "a".into(),
            connection_id: connection.id.clone(),
            connection_revision: 1,
            credential_revision: "k1".into(),
            node_id: "n".into(),
            kind: "edit".into(),
            requested_model: "new-image-model".into(),
            prompt: "图像".into(),
            parameters: ImageParameters::default(),
            state: "queued".into(),
            local_save_available: false,
            started_at: None,
            request_id: None,
            result_state: "unconfirmed".into(),
            outcome: None,
            charge_state: "unavailable".into(),
            charges: vec![],
            image_ids: vec![],
            error: None,
        };
        let mut snap = TaskSnapshot {
            task,
            panel_origin: "https://panel.test".into(),
            edge_base_url: "https://edge.test/v1".into(),
            provider_id: "openai".into(),
            installation_id: uuid(),
            secret_reference: "opaque".into(),
            model_revision: "r1".into(),
            maximum_request_bytes: 1024,
            maximum_results: 1,
            inputs: vec![input],
        };
        let body = request_bytes(&store, &snap).unwrap();
        assert!(body.len() > view.bytes as usize);
        assert!(
            String::from_utf8(body.clone())
                .unwrap()
                .contains("data:image/png;base64,")
        );
        snap.maximum_request_bytes = body.len() as u64 - 1;
        assert_eq!(
            request_bytes(&store, &snap).unwrap_err().code,
            "image_limit_exceeded"
        );
    }
    #[test]
    fn save_retry_only_reads_received_local_material() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let task = TaskView {
            id: uuid(),
            account_id: "a".into(),
            connection_id: uuid(),
            connection_revision: 1,
            credential_revision: "k".into(),
            node_id: "n".into(),
            kind: "generation".into(),
            requested_model: "m".into(),
            prompt: "p".into(),
            parameters: ImageParameters::default(),
            state: "save_failed".into(),
            local_save_available: true,
            started_at: None,
            request_id: None,
            result_state: "unconfirmed".into(),
            outcome: None,
            charge_state: "unavailable".into(),
            charges: vec![],
            image_ids: vec![],
            error: None,
        };
        let snap = TaskSnapshot {
            task,
            panel_origin: "https://panel.test".into(),
            edge_base_url: "https://edge.test/v1".into(),
            provider_id: "openai".into(),
            installation_id: uuid(),
            secret_reference: "opaque".into(),
            model_revision: "r".into(),
            maximum_request_bytes: 1024,
            maximum_results: 1,
            inputs: vec![],
        };
        stage_received(
            &store,
            &snap,
            &[ReceivedImage {
                id: uuid(),
                bytes: png(),
                info: validate_image(&png()).unwrap(),
            }],
        )
        .unwrap();
        let first = finish_local_save(&store, &snap).unwrap();
        let second = finish_local_save(&store, &snap).unwrap();
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(
            store
                .list::<HistoryImage>("image", Some("a"))
                .unwrap()
                .len(),
            1
        );
    }
}
