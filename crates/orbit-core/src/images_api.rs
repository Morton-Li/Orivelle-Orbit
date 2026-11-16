//! The API image contract is independent from the tested subscription controls.
use crate::{
    dto::*,
    error::{Result, fail},
    images::{self, MAX_REQUEST_BYTES},
};

pub const CONTRACT: &str = "openai-images-api-v1";

pub fn normalize(mut caps: ImageApiParameters) -> Option<ImageApiParameters> {
    if caps.contract != CONTRACT
        || caps.parameters.contract.as_deref() != Some(CONTRACT)
        || caps.parameters.max_results == 0
        || caps.parameters.max_results > 10
        || caps.parameters.max_inputs == 0
        || caps.parameters.max_inputs > 16
        || caps.prompt_maximum_characters == 0
        || caps.prompt_maximum_characters > 32000
        || caps.partial_images_maximum > 3
        || caps.gif_frame_count.min < 2
        || caps.gif_frame_count.max > 16
        || caps.gif_frame_count.min > caps.gif_frame_count.max
        || caps.gif_frame_duration_ms.min < 20
        || caps.gif_frame_duration_ms.max > 655350
        || caps.gif_frame_duration_ms.min > caps.gif_frame_duration_ms.max
    {
        return None;
    }
    caps.parameters.maximum_request_bytes =
        caps.parameters.maximum_request_bytes.min(MAX_REQUEST_BYTES);
    caps.parameters.qualities.retain(|v| {
        matches!(
            v.as_str(),
            "auto" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    });
    caps.parameters
        .output_formats
        .retain(|v| matches!(v.as_str(), "png" | "jpeg" | "webp" | "svg" | "gif"));
    caps.parameters
        .backgrounds
        .retain(|v| matches!(v.as_str(), "auto" | "opaque" | "transparent"));
    caps.parameters
        .complexities
        .retain(|v| matches!(v.as_str(), "low" | "medium" | "high"));
    caps.input_fidelities
        .retain(|v| matches!(v.as_str(), "low" | "high"));
    caps.moderations
        .retain(|v| matches!(v.as_str(), "auto" | "low"));
    caps.svg_reasoning_efforts.retain(|v| {
        matches!(
            v.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        )
    });
    Some(caps)
}

pub fn adjusted(
    p: &ImageParameters,
    inputs: usize,
    prompt: &str,
    mask: bool,
    markup: bool,
) -> Vec<String> {
    let changes = [
        ("n", p.n.unwrap_or(1) > 1),
        ("quality", p.quality.as_deref().is_some_and(|v| v != "auto")),
        ("size", p.size.as_deref().is_some_and(|v| v != "auto")),
        (
            "output_format",
            p.output_format.as_deref().is_some_and(|v| v != "png"),
        ),
        (
            "output_compression",
            p.output_compression.is_some_and(|v| v != 100),
        ),
        ("complexity", p.complexity.is_some()),
        ("mask", mask),
        ("markup", markup),
        ("stream", p.stream == Some(true)),
        ("partial_images", p.partial_images.unwrap_or(0) > 0),
        (
            "moderation",
            p.moderation.as_deref().is_some_and(|v| v != "auto"),
        ),
        (
            "input_fidelity",
            p.input_fidelity.as_deref().is_some_and(|v| v != "low"),
        ),
        ("frame_count", p.frame_count.is_some_and(|v| v != 9)),
        (
            "frame_duration_ms",
            p.frame_duration_ms.is_some_and(|v| v != 200),
        ),
        ("svg", p.svg.is_some()),
        ("images", inputs > 5),
        ("prompt", prompt.len() > images::MAX_PROMPT_BYTES),
    ];
    changes
        .into_iter()
        .filter(|(_, v)| *v)
        .map(|(k, _)| k.to_owned())
        .collect()
}

pub fn validate(
    model: &ModelView,
    p: &ImageParameters,
    prompt: &str,
    inputs: usize,
    node_limit: u64,
    mask: bool,
    markup: bool,
) -> Result<(u64, u8, bool)> {
    let api = !adjusted(p, inputs, prompt, mask, markup).is_empty();
    if !api {
        // Defaults have their documented effect without requiring an API pool.
        let mut standard = p.clone();
        if standard.quality.as_deref() == Some("auto") {
            standard.quality = None;
        }
        if standard.size.as_deref() == Some("auto") {
            standard.size = None;
        }
        if standard.output_format.as_deref() == Some("png") {
            standard.output_format = None;
        }
        let (limit, n) =
            images::validate_subscription_parameters(model, &standard, prompt, inputs, node_limit)?;
        validate_shape(p, inputs, mask, markup)?;
        return Ok((limit, n, false));
    }
    let caps = model
        .image_api_parameters
        .clone()
        .and_then(normalize)
        .ok_or_else(|| fail("unsupported_image_parameter"))?;
    if model.category != "image"
        || model.execution_profile.as_deref() != Some("codex-images-json-v1")
        || model.support_status == "unsupported"
        || prompt.trim().is_empty()
        || prompt.chars().count() > caps.prompt_maximum_characters as usize
        || inputs > caps.parameters.max_inputs as usize
    {
        return Err(fail("image_limit_exceeded"));
    }
    validate_shape(p, inputs, mask, markup)?;
    let n = p.n.unwrap_or(1);
    if n == 0 || n > caps.parameters.max_results {
        return Err(fail("image_limit_exceeded"));
    }
    for (value, allowed) in [
        (&p.quality, &caps.parameters.qualities),
        (&p.background, &caps.parameters.backgrounds),
        (&p.output_format, &caps.parameters.output_formats),
        (&p.complexity, &caps.parameters.complexities),
        (&p.moderation, &caps.moderations),
        (&p.input_fidelity, &caps.input_fidelities),
    ] {
        if value.as_ref().is_some_and(|v| !allowed.contains(v)) {
            return Err(fail("unsupported_image_parameter"));
        }
    }
    if mask && !caps.mask
        || markup && !caps.markup
        || p.stream == Some(true) && !caps.stream
        || p.partial_images.unwrap_or(0) > caps.partial_images_maximum
    {
        return Err(fail("unsupported_image_parameter"));
    }
    if let Some(svg) = &p.svg {
        if !valid_model_id(&svg.model)
            || svg
                .reasoning
                .as_ref()
                .and_then(|r| r.effort.as_ref())
                .is_some_and(|v| !caps.svg_reasoning_efforts.contains(v))
        {
            return Err(fail("unsupported_image_parameter"));
        }
    }
    if let Some(size) = p.size.as_deref().filter(|v| *v != "auto") {
        let valid = if p.output_format.as_deref() == Some("gif") {
            valid_gif_size(size)
        } else if let Some(limits) = &caps.parameters.custom_size {
            images::valid_size(size, limits)
        } else {
            caps.parameters.sizes.iter().any(|v| v == size)
        };
        if !valid {
            return Err(fail("unsupported_image_parameter"));
        }
    }
    Ok((
        node_limit
            .min(caps.parameters.maximum_request_bytes)
            .min(MAX_REQUEST_BYTES),
        n,
        true,
    ))
}

pub fn validate_shape(p: &ImageParameters, inputs: usize, mask: bool, markup: bool) -> Result<()> {
    let format = p.output_format.as_deref().unwrap_or("png");
    let stream = p.stream.unwrap_or(false);
    let partial = p.partial_images.unwrap_or(0);
    if mask && markup
        || (mask || markup || p.input_fidelity.is_some()) && inputs == 0
        || partial > 3
        || partial > 0 && !stream
        || p.output_compression
            .is_some_and(|v| v > 100 || !matches!(format, "jpeg" | "webp"))
        || p.background.as_deref() == Some("transparent") && !matches!(format, "png" | "webp")
        || matches!(format, "svg" | "gif")
            && (stream || partial > 0 || p.output_compression.is_some())
        || format == "svg" && (inputs > 0 || p.n.unwrap_or(1) != 1)
        || format == "gif" && mask
        || p.svg.is_some() && format != "svg"
        || (p.frame_count.is_some() || p.frame_duration_ms.is_some()) && format != "gif"
        || p.frame_count.is_some_and(|v| !(2..=16).contains(&v))
        || p.frame_duration_ms
            .is_some_and(|v| !(20..=655350).contains(&v))
    {
        return Err(fail("unsupported_image_parameter"));
    }
    Ok(())
}
fn valid_gif_size(size: &str) -> bool {
    let Some((w, h)) = size.split_once('x') else {
        return false;
    };
    let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else {
        return false;
    };
    (16..=3840).contains(&w) && (16..=3840).contains(&h) && w % 16 == 0 && h % 16 == 0
}
fn valid_model_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/@-".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(api: bool) -> ModelView {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/contracts/images-api-catalog.json"
        ))
        .unwrap();
        let legacy: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/contracts/images-parameters.json"
        ))
        .unwrap();
        serde_json::from_value(serde_json::json!({"id":"public-image","label":"Image","provider_id":"openai","revision":"synthetic","category":"image","capabilities":["openai.images.generations","openai.images.edits"],"execution_profile":"codex-images-json-v1","image_billing_mode":"image_count","reasoning_efforts":[],"support_status":"unverified","image_parameters":legacy["codex_available_controls"],"image_api_parameters":if api {fixture["models"]["gpt-image-2.6-flare"].clone()}else{serde_json::Value::Null},"sales_price":null,"official_price":null})).unwrap()
    }
    #[test]
    fn defaults_remain_subscription_and_explicit_changes_require_api_metadata() {
        let defaults = ImageParameters {
            n: Some(1),
            quality: Some("auto".into()),
            size: Some("auto".into()),
            output_format: Some("png".into()),
            background: Some("transparent".into()),
            stream: Some(false),
            partial_images: Some(0),
            moderation: Some("auto".into()),
            ..Default::default()
        };
        assert!(adjusted(&defaults, 5, "ordinary", false, false).is_empty());
        assert_eq!(
            validate(
                &model(false),
                &defaults,
                "ordinary",
                5,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .unwrap()
            .2,
            false
        );
        let advanced = ImageParameters {
            quality: Some("high".into()),
            size: Some("1536x864".into()),
            ..Default::default()
        };
        assert_eq!(
            adjusted(&advanced, 1, "ordinary", false, false),
            ["quality", "size"]
        );
        assert!(
            validate(
                &model(false),
                &advanced,
                "ordinary",
                1,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .is_err()
        );
        assert!(
            validate(
                &model(true),
                &advanced,
                "ordinary",
                1,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .unwrap()
            .2
        );
        assert_eq!(
            validate(
                &model(true),
                &ImageParameters {
                    n: Some(10),
                    ..Default::default()
                },
                "ordinary",
                16,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .unwrap()
            .1,
            10
        );
        assert!(
            adjusted(
                &ImageParameters::default(),
                0,
                &"中".repeat(10922),
                false,
                false
            )
            .is_empty()
        );
        assert_eq!(
            adjusted(
                &ImageParameters::default(),
                0,
                &"中".repeat(10923),
                false,
                false
            ),
            ["prompt"]
        );
        assert!(
            validate(
                &model(true),
                &ImageParameters::default(),
                &"中".repeat(32000),
                0,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .unwrap()
            .2
        );
        assert_eq!(
            validate(
                &model(true),
                &ImageParameters::default(),
                &"中".repeat(32001),
                0,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .unwrap_err()
            .code,
            "image_limit_exceeded"
        );
    }
    #[test]
    fn api_combinations_and_unknown_capability_versions_fail_before_network() {
        let mut unsupported = model(true);
        unsupported.image_api_parameters.as_mut().unwrap().contract = "future".into();
        assert!(
            validate(
                &unsupported,
                &ImageParameters {
                    quality: Some("high".into()),
                    ..Default::default()
                },
                "ordinary",
                0,
                MAX_REQUEST_BYTES,
                false,
                false
            )
            .is_err()
        );
        for p in [
            ImageParameters {
                output_format: Some("svg".into()),
                n: Some(2),
                ..Default::default()
            },
            ImageParameters {
                output_format: Some("gif".into()),
                stream: Some(true),
                ..Default::default()
            },
            ImageParameters {
                partial_images: Some(1),
                ..Default::default()
            },
        ] {
            assert!(
                validate(
                    &model(true),
                    &p,
                    "ordinary",
                    0,
                    MAX_REQUEST_BYTES,
                    false,
                    false
                )
                .is_err()
            );
        }
    }
}
