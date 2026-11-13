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
pub const PARAMETER_CONTRACT: &str = "codex-images-static-parameters-v1";
pub const REQUEST_TIMEOUT_SECONDS: u64 = 630;
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
    let scope = store
        .preview_scope()?
        .ok_or_else(|| fail("preview_scope_unavailable"))?;
    preview_url_with_token(id, &scope.token)
}
pub fn preview_url_with_token(id: &str, token: &str) -> Result<String> {
    crate::identity::validate_uuid(id)?;
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fail("preview_scope_unavailable"));
    }
    // Opaque paths are resolved and authorized by the native asset protocol on
    // every read; no filesystem path is delivered to the webview.
    Ok(format!(
        "{}/{}/{id}.png",
        if cfg!(windows) {
            "http://asset.localhost"
        } else {
            "asset://localhost"
        },
        token
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
    let _files = store.image_files_lock()?;
    save_preview_inner(store, id, bytes)
}
fn save_preview_inner(store: &Store, id: &str, bytes: &[u8]) -> Result<Option<String>> {
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
    match preview_url(store, id) {
        Ok(url) => Ok(Some(url)),
        Err(e) if e.code == "preview_scope_unavailable" => Ok(None),
        Err(e) => Err(e),
    }
}
pub fn import_input(store: &Store, path: &Path, connection: &ConnectionView) -> Result<ImageInput> {
    let bytes = bounded_read(path, MAX_IMAGE_BYTES)?;
    let info = validate_image(&bytes)?;
    let _files = store.image_files_lock()?;
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
        preview_url: save_preview_inner(store, &id, &info.preview)?,
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
        .ok_or_else(|| fail("unsupported_image_model"))?
        .available_controls();
    if constraints
        .contract
        .as_deref()
        .is_some_and(|value| value != PARAMETER_CONTRACT)
    {
        return Err(fail("unsupported_image_parameter"));
    }
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
        (&parameters.background, &constraints.backgrounds),
        (&parameters.output_format, &constraints.output_formats),
        (&parameters.complexity, &constraints.complexities),
    ] {
        if value.as_ref().is_some_and(|v| !allowed.contains(v)) {
            return Err(fail("unsupported_image_parameter"));
        }
    }
    if parameters
        .quality
        .as_ref()
        .is_some_and(|v| !["auto", "low", "medium", "high", "xhigh", "max"].contains(&v.as_str()))
        || parameters
            .background
            .as_ref()
            .is_some_and(|v| !["auto", "opaque", "transparent"].contains(&v.as_str()))
    {
        return Err(fail("unsupported_image_parameter"));
    }
    let extended = parameters.output_format.is_some()
        || parameters.output_compression.is_some()
        || parameters.complexity.is_some()
        || parameters
            .quality
            .as_ref()
            .is_some_and(|v| ["xhigh", "max"].contains(&v.as_str()));
    if extended && constraints.contract.as_deref() != Some(PARAMETER_CONTRACT)
        || parameters
            .output_format
            .as_ref()
            .is_some_and(|v| !["png", "jpeg", "webp"].contains(&v.as_str()))
        || parameters
            .complexity
            .as_ref()
            .is_some_and(|v| !["low", "medium", "high"].contains(&v.as_str()))
        || parameters.background.as_deref() == Some("transparent")
            && parameters.output_format.as_deref() == Some("jpeg")
        || parameters.output_compression.is_some_and(|v| {
            v > 100
                || !constraints.output_compression
                || !matches!(parameters.output_format.as_deref(), Some("jpeg" | "webp"))
        })
    {
        return Err(fail("unsupported_image_parameter"));
    }
    if let Some(size) = &parameters.size {
        if !constraints.sizes.contains(size)
            && !constraints
                .custom_size
                .as_ref()
                .is_some_and(|limits| valid_size(size, limits))
        {
            return Err(fail("unsupported_image_parameter"));
        }
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
            if constraints.contract.as_deref() == Some(PARAMETER_CONTRACT)
                && !constraints
                    .custom_size
                    .as_ref()
                    .is_some_and(|limits| valid_size(size, limits))
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

pub fn valid_size(size: &str, limits: &ImageSizeConstraints) -> bool {
    let Some((width, height)) = size.split_once('x') else {
        return false;
    };
    if !(2..=4).contains(&width.len())
        || !(2..=4).contains(&height.len())
        || !width.bytes().all(|v| v.is_ascii_digit())
        || !height.bytes().all(|v| v.is_ascii_digit())
    {
        return false;
    }
    let (Ok(width), Ok(height)) = (width.parse::<u32>(), height.parse::<u32>()) else {
        return false;
    };
    let pixels = u64::from(width) * u64::from(height);
    // Local ceilings cannot be broadened by a stale or malformed catalog.
    limits.multiple == 16
        && limits.max_aspect_ratio > 0
        && limits.max_aspect_ratio <= 3
        && width >= limits.min_edge.max(16)
        && height >= limits.min_edge.max(16)
        && width <= limits.max_edge.min(3840)
        && height <= limits.max_edge.min(3840)
        && width % 16 == 0
        && height % 16 == 0
        && pixels >= limits.min_pixels.max(655360)
        && pixels <= limits.max_pixels.min(8294400)
        && u64::from(width) <= u64::from(height) * u64::from(limits.max_aspect_ratio)
        && u64::from(height) <= u64::from(width) * u64::from(limits.max_aspect_ratio)
}
pub fn request_bytes(store: &Store, snapshot: &TaskSnapshot) -> Result<Vec<u8>> {
    let limit = snapshot.maximum_request_bytes.min(MAX_REQUEST_BYTES) as usize;
    if snapshot.inputs.len() > 5 || snapshot.task.prompt.len() > MAX_PROMPT_BYTES {
        return Err(fail("image_limit_exceeded"));
    }
    // Serialize only empty URL placeholders first. This accounts for every JSON
    // escape, optional parameter and separator before reading/encoding any image.
    let skeleton = ImageRequest {
        model: &snapshot.task.requested_model,
        prompt: &snapshot.task.prompt,
        parameters: &snapshot.task.parameters,
        images: snapshot
            .inputs
            .iter()
            .map(|_| ImageReference {
                image_url: String::new(),
            })
            .collect(),
        response_format: "b64_json",
    };
    let mut estimated = serde_json::to_vec(&skeleton)
        .map_err(|_| fail("serialization_failed"))?
        .len();
    for input in &snapshot.inputs {
        if input.file_name.contains('/')
            || input.file_name.contains('\\')
            || input.file_name.contains("..")
        {
            return Err(fail("invalid_identifier"));
        }
        let metadata = fs::symlink_metadata(store.root.join("inputs").join(&input.file_name))
            .map_err(|_| fail("local_image_missing"))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != input.view.bytes
        {
            return Err(fail("input_changed"));
        }
        if metadata.len() > MAX_IMAGE_BYTES
            || !["image/png", "image/jpeg", "image/webp"].contains(&input.view.mime.as_str())
        {
            return Err(fail("image_limit_exceeded"));
        }
        let encoded = (metadata.len() as usize)
            .checked_add(2)
            .and_then(|n| (n / 3).checked_mul(4))
            .ok_or_else(|| fail("image_limit_exceeded"))?;
        estimated = estimated
            .checked_add(5 + input.view.mime.len() + 8)
            .and_then(|n| n.checked_add(encoded))
            .ok_or_else(|| fail("image_limit_exceeded"))?;
        if estimated > limit {
            return Err(fail("image_limit_exceeded"));
        }
    }
    if estimated > limit {
        return Err(fail("image_limit_exceeded"));
    }
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
    let mut writer = CappedRequestWriter {
        bytes: Vec::with_capacity(estimated),
        limit,
    };
    serde_json::to_writer(
        &mut writer,
        &ImageRequest {
            model: &snapshot.task.requested_model,
            prompt: &snapshot.task.prompt,
            parameters: &snapshot.task.parameters,
            images: references,
            response_format: "b64_json",
        },
    )
    .map_err(|_| fail("image_limit_exceeded"))?;
    let bytes = writer.bytes;
    if bytes.len() as u64 > snapshot.maximum_request_bytes.min(MAX_REQUEST_BYTES) {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(bytes)
}
struct CappedRequestWriter {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for CappedRequestWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let length = self.bytes.len().checked_add(bytes.len()).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "bounded request")
        })?;
        if length > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bounded request",
            ));
        }
        if length > self.bytes.capacity() {
            self.bytes.reserve_exact(length - self.bytes.len());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
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
    #[serde(default)]
    output_format: Option<String>,
    #[serde(default)]
    model: Option<String>,
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
pub struct ImageResult {
    pub images: Vec<ReceivedImage>,
    pub parameters: ImageResultParameters,
}
pub fn parse_response(bytes: &[u8], maximum_results: u8) -> Result<ImageResult> {
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(fail("response_too_large"));
    }
    let response: ImageResponse =
        serde_json::from_slice(bytes).map_err(|_| fail("invalid_image_response"))?;
    if response
        .quality
        .as_deref()
        .is_some_and(|value| !["auto", "low", "medium", "high", "xhigh", "max"].contains(&value))
        || response
            .background
            .as_deref()
            .is_some_and(|value| !["auto", "opaque", "transparent"].contains(&value))
        || response
            .output_format
            .as_deref()
            .is_some_and(|value| !["png", "jpeg", "webp"].contains(&value))
        || response.model.as_ref().is_some_and(|value| {
            value.is_empty()
                || value.len() > 255
                || !value.as_bytes()[0].is_ascii_alphanumeric()
                || !value
                    .bytes()
                    .all(|v| v.is_ascii_alphanumeric() || b"._:/@-".contains(&v))
        })
    {
        return Err(fail("invalid_image_response"));
    }
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
    Ok(ImageResult {
        images,
        parameters: ImageResultParameters {
            quality: response.quality,
            size: response.size,
            background: response.background,
            output_format: response.output_format,
            observed_model: response.model,
        },
    })
}

pub fn generation_failure(status: u16, bytes: &[u8]) -> crate::error::PublicError {
    // Never surface arbitrary provider messages, credentials, URLs or field names.
    let code = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .and_then(|value| value.get("error")?.get("code")?.as_str().map(str::to_owned));
    let safe = code.as_deref().filter(|code| {
        matches!(
            *code,
            "image_request_timeout"
                | "image_access_denied"
                | "invalid_image_request"
                | "unsupported_image_parameter"
                | "image_parameters_upgrade_required"
                | "image_metering_upgrade_required"
                | "model_unavailable"
                | "insufficient_balance"
                | "api_key_spending_limit_exceeded"
                | "invalid_image_response"
                | "image_response_incomplete"
        )
    });
    fail(safe.unwrap_or(match status {
        400 | 413 | 415 | 422 => "invalid_image_request",
        401 | 403 => "image_access_denied",
        429 => "image_rate_limited",
        426 => "image_parameters_upgrade_required",
        _ => "generation_response_failed",
    }))
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
    let _files = store.image_files_lock()?;
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
    let _files = store.image_files_lock()?;
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
            preview_url: save_preview_inner(store, &item.id, &info.preview)?,
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
                requested_model_label: Some("Image Model".into()),
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
                result_parameters: None,
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
        assert_eq!(parsed.images.len(), 1);
        assert!(parsed.parameters.observed_model.is_none());
        assert!(parse_response(br#"{"created":1,"data":[{"b64_json":"bad!"}]}"#, 1).is_err());
        assert!(parse_response(&serde_json::to_vec(&payload).unwrap(), 0).is_err());
    }
    #[test]
    fn legacy_parameter_fixture_still_serializes_without_losing_zero_compression() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/contracts/images-parameters.json"
        ))
        .unwrap();
        let model = |id: &str| -> ModelView {
            serde_json::from_value(serde_json::json!({"id":id,"label":id,"provider_id":"openai","revision":"synthetic",
                "category":"image","capabilities":["openai.images.generations","openai.images.edits"],
                "execution_profile":"codex-images-json-v1","image_billing_mode":"image_count","reasoning_efforts":[],
                "reasoning":null,"codex_catalog_binding":null,"support_status":"unverified",
                "image_parameters":fixture["models"][id],"sales_price":null,"official_price":null})).unwrap()
        };
        for raw in fixture["valid_requests"].as_array().unwrap() {
            let id = raw["model"].as_str().unwrap();
            let mut params = raw.as_object().unwrap().clone();
            params.remove("model");
            params.remove("prompt");
            let parameters: ImageParameters = serde_json::from_value(params.into()).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path()).unwrap();
            let mut snapshot = snapshot();
            snapshot.task.requested_model = id.into();
            snapshot.task.parameters = parameters;
            let bytes = request_bytes(&store, &snapshot).unwrap();
            let actual: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            for (key, value) in raw.as_object().unwrap() {
                if key != "prompt" {
                    assert_eq!(&actual[key], value, "{key}");
                }
            }
        }
        let model = model("gpt-image-2.5-flare");
        for raw in fixture["invalid_requests"].as_array().unwrap() {
            if let Ok(parameters) = serde_json::from_value::<ImageParameters>(raw.clone()) {
                assert!(
                    validate_parameters(&model, &parameters, "synthetic", 0, MAX_REQUEST_BYTES)
                        .is_err(),
                    "{raw}"
                );
            }
        }
    }
    #[test]
    fn available_controls_restrict_legacy_catalogs_and_reject_hidden_new_task_parameters() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/contracts/images-parameters.json"
        ))
        .unwrap();
        let constraints: ImageParameterConstraints =
            serde_json::from_value(fixture["models"]["gpt-image-2.5-flare"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(constraints.available_controls()).unwrap(),
            fixture["codex_available_controls"]
        );
        let model: ModelView = serde_json::from_value(serde_json::json!({"id":"synthetic-image","label":"Synthetic","provider_id":"openai","revision":"synthetic",
            "category":"image","capabilities":["openai.images.generations","openai.images.edits"],
            "execution_profile":"codex-images-json-v1","image_billing_mode":"image_count","reasoning_efforts":[],
            "reasoning":null,"codex_catalog_binding":null,"support_status":"unverified",
            "image_parameters":constraints,"sales_price":null,"official_price":null})).unwrap();
        for background in [None, Some("opaque"), Some("transparent")] {
            let parameters = ImageParameters {
                background: background.map(str::to_owned),
                ..Default::default()
            };
            let (_, results) =
                validate_parameters(&model, &parameters, "synthetic", 5, MAX_REQUEST_BYTES)
                    .unwrap();
            assert_eq!(results, 1);
            assert!(
                validate_parameters(&model, &parameters, "synthetic", 6, MAX_REQUEST_BYTES)
                    .is_err()
            );
        }
        for raw in [
            serde_json::json!({"n":2}),
            serde_json::json!({"quality":"low"}),
            serde_json::json!({"quality":"high"}),
            serde_json::json!({"size":"auto"}),
            serde_json::json!({"size":"1536x1024"}),
            serde_json::json!({"output_format":"png"}),
            serde_json::json!({"output_format":"jpeg"}),
            serde_json::json!({"output_compression":0}),
            serde_json::json!({"complexity":"low"}),
            serde_json::json!({"background":"auto"}),
        ] {
            let parameters: ImageParameters = serde_json::from_value(raw.clone()).unwrap();
            assert!(
                validate_parameters(&model, &parameters, "synthetic", 0, MAX_REQUEST_BYTES)
                    .is_err(),
                "{raw}"
            );
        }
    }
    #[test]
    fn observed_parameters_are_separate_from_decoded_image_facts_and_private_errors_are_filtered() {
        let raw = serde_json::json!({"created":1,"quality":"max","size":"1254x1254","output_format":"jpeg","model":"gpt-image-2.5-flare",
            "data":[{"b64_json":STANDARD.encode(png())}]});
        let result = parse_response(&serde_json::to_vec(&raw).unwrap(), 1).unwrap();
        assert_eq!(result.parameters.quality.as_deref(), Some("max"));
        assert_eq!(result.parameters.size.as_deref(), Some("1254x1254"));
        assert_eq!(result.parameters.output_format.as_deref(), Some("jpeg"));
        assert_eq!(
            (
                result.images[0].info.mime,
                result.images[0].info.width,
                result.images[0].info.height
            ),
            ("image/png", 16, 16)
        );
        let error = generation_failure(403,br#"{"error":{"code":"image_access_denied","message":"SYNTHETIC_PRIVATE_PROVIDER_MESSAGE"}}"#);
        assert_eq!(error.code, "image_access_denied");
        assert!(!error.message.contains("SYNTHETIC_PRIVATE"));
        assert_eq!(
            generation_failure(504, br#"{"error":{"code":"image_request_timeout"}}"#).code,
            "image_request_timeout"
        );
        assert_eq!(
            generation_failure(429, br#"{"error":{"code":"unknown_private_code"}}"#).code,
            "image_rate_limited"
        );
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
            requested_model_label: Some("New Image Model".into()),
            prompt: "图像".into(),
            parameters: ImageParameters::default(),
            state: "queued".into(),
            local_save_available: false,
            started_at: None,
            request_id: None,
            result_state: "unconfirmed".into(),
            outcome: None,
            charge_state: "unavailable".into(),
            result_parameters: None,
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
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["model"], "new-image-model");
        assert!(payload.get("requested_model_label").is_none());
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
    fn total_budget_rejects_sparse_large_input_before_read_hash_or_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let mut task = snapshot();
        task.maximum_request_bytes = 2 * 1024 * 1024;
        let id = uuid();
        let name = format!("{id}.png");
        let file = File::create(store.root.join("inputs").join(&name)).unwrap();
        file.set_len(MAX_IMAGE_BYTES).unwrap();
        task.inputs.push(InputRecord {
            view: ImageInput {
                id,
                name: "synthetic.png".into(),
                mime: "image/png".into(),
                bytes: MAX_IMAGE_BYTES,
                width: 1,
                height: 1,
                preview_url: None,
            },
            account_id: task.task.account_id.clone(),
            connection_id: task.task.connection_id.clone(),
            connection_revision: task.task.connection_revision,
            hash: "intentionally-not-the-file-hash".into(),
            file_name: name,
        });
        // A read/hash/format attempt would produce input_changed/invalid_image.
        assert_eq!(
            request_bytes(&store, &task).unwrap_err().code,
            "image_limit_exceeded"
        );
        let mut writer = CappedRequestWriter {
            bytes: Vec::new(),
            limit: 3,
        };
        writer.write_all(b"12").unwrap();
        let capacity = writer.bytes.capacity();
        assert!(writer.write_all(b"34").is_err());
        assert_eq!(writer.bytes, b"12");
        assert_eq!(writer.bytes.capacity(), capacity);
    }
    #[test]
    fn concurrent_image_and_preview_writes_cannot_overrun_disk_quotas() {
        use std::sync::{Arc, Barrier};
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("data")).unwrap());
        let bytes = png();
        let source = dir.path().join("public-fixture.png");
        fs::write(&source, &bytes).unwrap();
        let mut settings = store.settings().unwrap();
        settings.image_disk_limit_bytes = bytes.len() as u64;
        settings.preview_cache_limit_bytes = 1024;
        store.put("settings", "local", "local", &settings).unwrap();
        let connection = ConnectionView {
            id: uuid(),
            label: "synthetic".into(),
            panel_origin: "https://fixture.example.test".into(),
            account_id: "a".into(),
            node_id: None,
            node_label: None,
            edge_base_url: None,
            provider_id: "openai".into(),
            revision: 1,
            credential_revision: None,
            permission_state: "allowed".into(),
        };
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = vec![];
        for _ in 0..2 {
            let store = store.clone();
            let source = source.clone();
            let connection = connection.clone();
            let gate = barrier.clone();
            threads.push(std::thread::spawn(move || {
                gate.wait();
                import_input(&store, &source, &connection)
            }));
        }
        barrier.wait();
        let successes = threads
            .into_iter()
            .map(|t| t.join().unwrap())
            .filter(Result::is_ok)
            .count();
        assert_eq!(successes, 1);
        assert_eq!(
            disk_usage(&store.root.join("inputs")).unwrap(),
            bytes.len() as u64
        );
        settings.preview_cache_limit_bytes = bytes.len() as u64;
        store.put("settings", "local", "local", &settings).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = vec![];
        for _ in 0..2 {
            let store = store.clone();
            let bytes = bytes.clone();
            let gate = barrier.clone();
            threads.push(std::thread::spawn(move || {
                gate.wait();
                save_preview(&store, &uuid(), &bytes)
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        assert!(
            disk_usage(&store.root.join("previews")).unwrap() <= settings.preview_cache_limit_bytes
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
            requested_model_label: Some("Image Model".into()),
            prompt: "p".into(),
            parameters: ImageParameters::default(),
            state: "save_failed".into(),
            local_save_available: true,
            started_at: None,
            request_id: None,
            result_state: "unconfirmed".into(),
            outcome: None,
            charge_state: "unavailable".into(),
            result_parameters: None,
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
