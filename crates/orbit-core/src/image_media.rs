//! Native-only decoders. Original SVG never enters the webview; GIF previews
//! are rebuilt from fully decoded, bounded frames without replacing originals.
use crate::{
    error::{Result, fail},
    images::{self, ValidatedImage},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use image::{DynamicImage, ImageFormat, RgbaImage};
use quick_xml::{Reader, events::Event};
use resvg::{tiny_skia, usvg};
use std::{
    collections::HashSet,
    io::{Cursor, Write},
    num::NonZeroU64,
    sync::{Arc, OnceLock},
};

pub(crate) const GIF_PREVIEW_EDGE: u32 = 768;
pub(crate) const MAX_GIF_PREVIEW_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SVG_NODES: usize = 50_000;
const MAX_SVG_DEPTH: usize = 128;

pub(crate) fn dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0
        || height == 0
        || width > images::MAX_DIMENSION
        || height > images::MAX_DIMENSION
        || u64::from(width) * u64::from(height) > images::MAX_PIXELS
    {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(())
}
pub(crate) fn png_preview(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut out = Cursor::new(Vec::new());
    image
        .thumbnail(1024, 1024)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|_| fail("invalid_image"))?;
    if out.get_ref().len() > 4 * 1024 * 1024 {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(out.into_inner())
}

/// Check references before usvg parses them. This also protects originals when
/// users later open an exported SVG in a browser or image editor.
fn svg_guard(bytes: &[u8]) -> Result<bool> {
    let source = std::str::from_utf8(bytes).map_err(|_| fail("invalid_image"))?;
    let mut reader = Reader::from_str(source);
    let mut nodes = 0;
    let mut depth = 0usize;
    let mut style_depth = None;
    let mut style = String::new();
    let mut has_text = false;
    let mut root = false;
    let mut embedded_pixels = 0u64;
    loop {
        match reader.read_event().map_err(|_| fail("invalid_image"))? {
            event @ (Event::Start(_) | Event::Empty(_)) => {
                if style_depth.is_some() {
                    return Err(fail("unsafe_image_content"));
                }
                let empty = matches!(event, Event::Empty(_));
                let tag = match event {
                    Event::Start(t) | Event::Empty(t) => t,
                    _ => unreachable!(),
                };
                let local = tag.local_name();
                let name = std::str::from_utf8(local.as_ref())
                    .map_err(|_| fail("invalid_image"))?
                    .to_ascii_lowercase();
                nodes += 1;
                if nodes > MAX_SVG_NODES || depth >= MAX_SVG_DEPTH {
                    return Err(fail("image_limit_exceeded"));
                }
                if !root {
                    if name != "svg" {
                        return Err(fail("invalid_image"));
                    }
                    root = true;
                }
                if [
                    "script",
                    "foreignobject",
                    "iframe",
                    "object",
                    "embed",
                    "audio",
                    "video",
                    "animate",
                    "animatetransform",
                    "animatemotion",
                    "set",
                    "discard",
                    "handler",
                    "listener",
                ]
                .contains(&name.as_str())
                    || name.starts_with("animate")
                {
                    return Err(fail("unsafe_image_content"));
                }
                has_text |= name == "text";
                for attribute in tag.attributes() {
                    let attr = attribute.map_err(|_| fail("invalid_image"))?;
                    let key = std::str::from_utf8(attr.key.as_ref())
                        .map_err(|_| fail("invalid_image"))?
                        .to_ascii_lowercase();
                    let local = key.rsplit(':').next().unwrap_or(&key);
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|_| fail("invalid_image"))?;
                    if local.starts_with("on") || key == "xml:base" {
                        return Err(fail("unsafe_image_content"));
                    }
                    if local == "href" || local == "src" {
                        if value.starts_with('#') && value.len() <= 512 {
                            continue;
                        }
                        if name != "image" {
                            return Err(fail("unsafe_image_content"));
                        }
                        let Some((prefix, encoded)) = value.split_once(',') else {
                            return Err(fail("unsafe_image_content"));
                        };
                        let mime = match prefix {
                            "data:image/png;base64" => "image/png",
                            "data:image/jpeg;base64" => "image/jpeg",
                            "data:image/webp;base64" => "image/webp",
                            _ => return Err(fail("unsafe_image_content")),
                        };
                        let data = STANDARD
                            .decode(encoded)
                            .map_err(|_| fail("invalid_image"))?;
                        let info = images::validate_input_image(&data)?;
                        embedded_pixels = embedded_pixels
                            .saturating_add(u64::from(info.width) * u64::from(info.height));
                        if info.mime != mime || embedded_pixels > images::MAX_PIXELS {
                            return Err(fail("image_limit_exceeded"));
                        }
                    } else {
                        safe_css(&value)?;
                    }
                }
                if !empty {
                    depth += 1;
                    if name == "style" {
                        style_depth = Some(depth);
                        style.clear();
                    }
                }
            }
            Event::End(_) => {
                if style_depth == Some(depth) {
                    safe_css(&style)?;
                    style_depth = None;
                }
                depth = depth.checked_sub(1).ok_or_else(|| fail("invalid_image"))?;
            }
            Event::Text(text) if style_depth.is_some() => {
                let decoded = text.decode().map_err(|_| fail("invalid_image"))?;
                style.push_str(&decoded);
            }
            Event::CData(text) if style_depth.is_some() => {
                style.push_str(&text.decode().map_err(|_| fail("invalid_image"))?);
            }
            Event::GeneralRef(reference) if style_depth.is_some() => {
                let entity = format!(
                    "&{};",
                    reference.decode().map_err(|_| fail("invalid_image"))?
                );
                style.push_str(
                    &quick_xml::escape::unescape(&entity).map_err(|_| fail("invalid_image"))?,
                );
            }
            Event::DocType(_) | Event::PI(_) => return Err(fail("unsafe_image_content")),
            Event::Eof => break,
            _ => {}
        }
    }
    if !root || depth != 0 {
        return Err(fail("invalid_image"));
    }
    Ok(has_text)
}
fn safe_css(value: &str) -> Result<()> {
    // Reject CSS escapes/comments rather than attempting to normalize a second
    // language with a different entity/escape grammar. Local paint refs remain.
    let compact = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.contains('\\')
        || compact.contains("/*")
        || compact.contains("@import")
        || compact.contains("@font-face")
        || compact.contains("javascript:")
        || compact.contains("expression(")
    {
        return Err(fail("unsafe_image_content"));
    }
    let mut rest = compact.as_str();
    while let Some((_, after)) = rest.split_once("url(") {
        let (reference, tail) = after
            .split_once(')')
            .ok_or_else(|| fail("unsafe_image_content"))?;
        let reference = reference.trim_matches(['\'', '"']);
        if !reference.starts_with('#') || reference.len() > 512 {
            return Err(fail("unsafe_image_content"));
        }
        rest = tail;
    }
    Ok(())
}
fn svg_tree(bytes: &[u8]) -> Result<usvg::Tree> {
    let text = svg_guard(bytes)?;
    let mut options = usvg::Options::default();
    options.image_href_resolver = usvg::ImageHrefResolver {
        resolve_string: Box::new(|_, _| None),
        resolve_data: Box::new(|mime, data, _| match mime {
            "image/png" => Some(usvg::ImageKind::PNG(data)),
            "image/jpeg" => Some(usvg::ImageKind::JPEG(data)),
            "image/webp" => Some(usvg::ImageKind::WEBP(data)),
            _ => None,
        }),
    };
    if text {
        static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
        options.fontdb = FONTS
            .get_or_init(|| {
                let mut fonts = usvg::fontdb::Database::new();
                fonts.load_system_fonts();
                Arc::new(fonts)
            })
            .clone();
        options.font_family = "Arial".into();
    }
    let tree = usvg::Tree::from_data(bytes, &options).map_err(|_| fail("invalid_image"))?;
    let size = tree.size().to_int_size();
    dimensions(size.width(), size.height())?;
    // Estimate retained offscreen layers at full clipboard resolution, before
    // rendering any preview. A tiny SVG cannot allocate an enormous filter.
    let mut budget = images::MAX_DECODE_BYTES * 4;
    let mut visited = HashSet::new();
    layer_budget(tree.root(), &mut budget, &mut visited)?;
    for pattern in tree.patterns() {
        layer_budget(pattern.root(), &mut budget, &mut visited)?;
    }
    for mask in tree.masks() {
        layer_budget(mask.root(), &mut budget, &mut visited)?;
    }
    for clip in tree.clip_paths() {
        layer_budget(clip.root(), &mut budget, &mut visited)?;
    }
    Ok(tree)
}
fn layer_budget(group: &usvg::Group, budget: &mut u64, visited: &mut HashSet<usize>) -> Result<()> {
    if !visited.insert(std::ptr::from_ref(group) as usize) {
        return Ok(());
    }
    if visited.len() > MAX_SVG_NODES {
        return Err(fail("image_limit_exceeded"));
    }
    if group.should_isolate() {
        let bounds = group
            .layer_bounding_box()
            .transform(group.abs_transform())
            .ok_or_else(|| fail("image_limit_exceeded"))?;
        let copies = group
            .filters()
            .iter()
            .map(|filter| filter.primitives().len() + 3)
            .sum::<usize>()
            .max(1);
        let bytes = f64::from(bounds.width().ceil() + 4.0)
            * f64::from(bounds.height().ceil() + 4.0)
            * 4.0
            * copies as f64;
        if !bytes.is_finite() || bytes > *budget as f64 {
            return Err(fail("image_limit_exceeded"));
        }
        *budget -= bytes as u64;
    }
    for child in group.children() {
        if let usvg::Node::Group(nested) = child {
            layer_budget(nested, budget, visited)?;
        }
        let mut result = Ok(());
        child.subroots(|root| {
            if result.is_ok() {
                result = layer_budget(root, budget, visited);
            }
        });
        result?;
    }
    Ok(())
}
fn render_svg(tree: &usvg::Tree, preview: bool) -> Result<tiny_skia::Pixmap> {
    let size = tree.size().to_int_size();
    let scale = if preview {
        (1024.0 / size.width().max(size.height()) as f32).min(1.0)
    } else {
        1.0
    };
    let width = (size.width() as f32 * scale).round().max(1.0) as u32;
    let height = (size.height() as f32 * scale).round().max(1.0) as u32;
    let mut pixels =
        tiny_skia::Pixmap::new(width, height).ok_or_else(|| fail("image_limit_exceeded"))?;
    let transform = tiny_skia::Transform::from_scale(
        width as f32 / tree.size().width(),
        height as f32 / tree.size().height(),
    );
    resvg::render(tree, transform, &mut pixels.as_mut());
    Ok(pixels)
}
pub(crate) fn svg(bytes: &[u8]) -> Result<ValidatedImage> {
    let tree = svg_tree(bytes)?;
    let size = tree.size().to_int_size();
    let preview = render_svg(&tree, true)?
        .encode_png()
        .map_err(|_| fail("invalid_image"))?;
    if preview.len() > 4 * 1024 * 1024 {
        return Err(fail("image_limit_exceeded"));
    }
    Ok(ValidatedImage {
        mime: "image/svg+xml",
        extension: "svg",
        width: size.width(),
        height: size.height(),
        preview,
        animation_preview: None,
    })
}
pub(crate) fn svg_pixels(bytes: &[u8]) -> Result<RgbaImage> {
    let tree = svg_tree(bytes)?;
    let pixmap = render_svg(&tree, false)?;
    let (width, height) = (pixmap.width(), pixmap.height());
    let mut bytes = pixmap.take();
    for pixel in bytes.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = if alpha == 0 {
                0
            } else {
                ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8
            };
        }
    }
    RgbaImage::from_raw(width, height, bytes).ok_or_else(|| fail("invalid_image"))
}

/// A fast structural pass bounds frame count/rectangles before any LZW decode.
pub(crate) fn gif_structure(bytes: &[u8], preview: bool) -> Result<(u32, u32, usize, gif::Repeat)> {
    if bytes.last() != Some(&0x3b) {
        return Err(fail("invalid_image"));
    }
    let mut options = gif::DecodeOptions::new();
    options.skip_frame_decoding(true);
    options.check_frame_consistency(true);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        NonZeroU64::new(images::MAX_IMAGE_BYTES).unwrap(),
    ));
    let mut reader = options
        .read_info(Cursor::new(bytes))
        .map_err(|_| fail("invalid_image"))?;
    let (width, height) = (u32::from(reader.width()), u32::from(reader.height()));
    dimensions(width, height)?;
    if preview && (width > GIF_PREVIEW_EDGE || height > GIF_PREVIEW_EDGE) {
        return Err(fail("image_limit_exceeded"));
    }
    let mut count = 0;
    while let Some(frame) = reader
        .read_next_frame()
        .map_err(|_| fail("invalid_image"))?
    {
        count += 1;
        if count > 16 || frame.width == 0 || frame.height == 0 {
            return Err(fail("image_limit_exceeded"));
        }
    }
    if count == 0 {
        return Err(fail("invalid_image"));
    }
    Ok((width, height, count, reader.repeat()))
}
pub(crate) fn gif(bytes: &[u8]) -> Result<ValidatedImage> {
    let (width, height, count, repeat) = gif_structure(bytes, false)?;
    let mut decoder = gif_decoder(bytes)?;
    let mut canvas = RgbaImage::new(width, height);
    let mut output = LimitedGif(Vec::new());
    let mut png = None;
    let mut actual = 0;
    {
        let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(&mut output, 20);
        match repeat {
            gif::Repeat::Infinite => encoder.set_repeat(image::codecs::gif::Repeat::Infinite),
            gif::Repeat::Finite(n) if n > 0 => {
                encoder.set_repeat(image::codecs::gif::Repeat::Finite(n))
            }
            _ => Ok(()),
        }
        .map_err(|_| fail("invalid_image"))?;
        while let Some(frame) = decoder
            .read_next_frame()
            .map_err(|_| fail("invalid_image"))?
        {
            actual += 1;
            if actual > count {
                return Err(fail("invalid_image"));
            }
            let previous = (frame.dispose == gif::DisposalMethod::Previous).then(|| canvas.clone());
            composite_frame(&mut canvas, frame)?;
            let scale = (GIF_PREVIEW_EDGE as f32 / width.max(height) as f32).min(1.0);
            let scaled = image::imageops::thumbnail(
                &canvas,
                (width as f32 * scale).round().max(1.0) as u32,
                (height as f32 * scale).round().max(1.0) as u32,
            );
            if png.is_none() {
                png = Some(png_preview(&DynamicImage::ImageRgba8(scaled.clone()))?);
            }
            let delay = image::Delay::from_numer_denom_ms(u32::from(frame.delay) * 10, 1);
            encoder
                .encode_frame(image::Frame::from_parts(scaled, 0, 0, delay))
                .map_err(|_| fail("image_limit_exceeded"))?;
            match frame.dispose {
                gif::DisposalMethod::Previous => canvas = previous.unwrap(),
                gif::DisposalMethod::Background => {
                    for y in u32::from(frame.top)..u32::from(frame.top) + u32::from(frame.height) {
                        for x in
                            u32::from(frame.left)..u32::from(frame.left) + u32::from(frame.width)
                        {
                            canvas.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if actual != count {
        return Err(fail("invalid_image"));
    }
    Ok(ValidatedImage {
        mime: "image/gif",
        extension: "gif",
        width,
        height,
        preview: png.ok_or_else(|| fail("invalid_image"))?,
        animation_preview: Some(output.0),
    })
}
fn gif_decoder(bytes: &[u8]) -> Result<gif::Decoder<Cursor<&[u8]>>> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.check_frame_consistency(true);
    options.check_lzw_end_code(true);
    options.set_memory_limit(gif::MemoryLimit::Bytes(
        NonZeroU64::new(images::MAX_DECODE_BYTES).unwrap(),
    ));
    options
        .read_info(Cursor::new(bytes))
        .map_err(|_| fail("invalid_image"))
}
fn composite_frame(canvas: &mut RgbaImage, frame: &gif::Frame<'_>) -> Result<()> {
    if u32::from(frame.left) + u32::from(frame.width) > canvas.width()
        || u32::from(frame.top) + u32::from(frame.height) > canvas.height()
        || frame.buffer.len() != usize::from(frame.width) * usize::from(frame.height) * 4
    {
        return Err(fail("invalid_image"));
    }
    for (index, pixel) in frame.buffer.chunks_exact(4).enumerate() {
        if pixel[3] != 0 {
            canvas.put_pixel(
                u32::from(frame.left) + index as u32 % u32::from(frame.width),
                u32::from(frame.top) + index as u32 / u32::from(frame.width),
                image::Rgba(pixel.try_into().unwrap()),
            );
        }
    }
    Ok(())
}
pub(crate) fn gif_first_frame(bytes: &[u8]) -> Result<RgbaImage> {
    let (width, height, _, _) = gif_structure(bytes, false)?;
    let mut canvas = RgbaImage::new(width, height);
    let mut decoder = gif_decoder(bytes)?;
    let frame = decoder
        .read_next_frame()
        .map_err(|_| fail("invalid_image"))?
        .ok_or_else(|| fail("invalid_image"))?;
    composite_frame(&mut canvas, frame)?;
    Ok(canvas)
}

struct LimitedGif(Vec<u8>);
impl Write for LimitedGif {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) as u64 > MAX_GIF_PREVIEW_BYTES {
            return Err(std::io::Error::other("preview limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::AnimationDecoder;
    pub(crate) const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><defs><linearGradient id="paint"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient></defs><style>.fill { fill: url(#paint) }</style><rect class="fill" width="32" height="16"/></svg>"##;
    pub(crate) fn animation(frames: usize) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Finite(3))
                .unwrap();
            for index in 0..frames {
                let buffer = RgbaImage::from_pixel(
                    16,
                    16,
                    image::Rgba(if index % 2 == 0 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    }),
                );
                encoder
                    .encode_frame(image::Frame::from_parts(
                        buffer,
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(100 + index as u32 * 20, 1),
                    ))
                    .unwrap();
            }
        }
        out
    }
    #[test]
    fn svg_is_rasterized_for_preview_and_clipboard_with_straight_alpha() {
        let info = images::validate_image(SVG).unwrap();
        assert_eq!(
            (info.mime, info.extension, info.width, info.height),
            ("image/svg+xml", "svg", 32, 16)
        );
        assert_eq!(
            image::guess_format(&info.preview).unwrap(),
            ImageFormat::Png
        );
        let pixels = images::decode_pixels(br#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect fill="red" opacity="0.5" width="16" height="16"/></svg>"#).unwrap();
        assert_eq!(pixels.get_pixel(4, 4).0, [255, 0, 0, 128]);
        let text = images::validate_image("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"30\"><text x=\"1\" y=\"20\">Orbit 图像</text></svg>".as_bytes()).unwrap();
        assert_eq!(text.mime, "image/svg+xml");
        assert_eq!(
            images::validate_input_image(SVG).err().unwrap().code,
            "unsupported_image_input"
        );
    }
    #[test]
    fn svg_active_content_and_obfuscated_external_references_are_rejected() {
        for inner in [
            "<script>alert(1)</script>",
            "<rect onload='alert(1)'/>",
            "<foreignObject/>",
            "<image href='file:///private/canary.png'/>",
            "<image href='https://example.test/private.png'/>",
            "<set attributeName='href' to='https://example.test'/>",
            "<handler>unsafe</handler>",
            "<style>.a{fill:u&#114;l(https://example.test)}</style>",
            "<style>.a{fill:u<![CDATA[rl(https://example.test)]]>}</style>",
            "<style>.a{fill:u\\72l(https://example.test)}</style>",
            "<style>@import 'https://example.test/style.css';</style>",
            "<rect style='fill:url(file:///private/canary)'/>",
            "<g xml:base='https://example.test/'><use href='#paint'/></g>",
        ] {
            let source = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>{inner}</svg>"
            );
            assert_eq!(
                images::validate_image(source.as_bytes())
                    .err()
                    .unwrap()
                    .code,
                "unsafe_image_content",
                "{inner}"
            );
        }
        let dtd = b"<!DOCTYPE svg [<!ENTITY x SYSTEM 'file:///private/canary'>]><svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'/>";
        assert_eq!(
            images::validate_image(dtd).err().unwrap().code,
            "unsafe_image_content"
        );
    }
    #[test]
    fn vector_dimensions_depth_filters_and_embedded_rasters_are_bounded() {
        for source in [
            "<svg xmlns='http://www.w3.org/2000/svg' width='9000' height='16'/>".to_owned(),
            format!("<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'>{}<rect width='16' height='16'/>{}</svg>", "<g>".repeat(130), "</g>".repeat(130)),
            "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><filter id='f' x='-100000' y='-100000' width='200000' height='200000' filterUnits='userSpaceOnUse'><feGaussianBlur stdDeviation='1'/></filter><rect filter='url(#f)' width='16' height='16'/></svg>".to_owned(),
        ] {
            assert_eq!(images::validate_image(source.as_bytes()).err().unwrap().code, "image_limit_exceeded");
        }
        let mut png = Cursor::new(Vec::new());
        DynamicImage::new_rgba8(16, 16)
            .write_to(&mut png, ImageFormat::Png)
            .unwrap();
        let embedded = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><image width='16' height='16' href='data:image/png;base64,{}'/></svg>",
            STANDARD.encode(png.into_inner())
        );
        assert_eq!(
            images::validate_image(embedded.as_bytes()).unwrap().mime,
            "image/svg+xml"
        );
    }
    #[test]
    fn gif_large_legal_frame_uses_the_explicit_allocation_budget() {
        let (width, height) = (3840, 3328);
        let mut bytes = Vec::new();
        {
            let mut encoder =
                gif::Encoder::new(&mut bytes, width, height, &[255, 0, 0, 0, 0, 255]).unwrap();
            let frame = gif::Frame {
                width,
                height,
                buffer: std::borrow::Cow::Owned(vec![0; usize::from(width) * usize::from(height)]),
                ..Default::default()
            };
            encoder.write_frame(&frame).unwrap();
        }
        let pixels = images::decode_pixels(&bytes).unwrap();
        assert!(pixels.as_raw().len() > 50_000_000);
        assert_eq!(pixels.dimensions(), (u32::from(width), u32::from(height)));
        drop(pixels);
        let info = images::validate_image(&bytes).unwrap();
        let animation = info.animation_preview.unwrap();
        let (preview_width, preview_height, count, _) = gif_structure(&animation, true).unwrap();
        assert!(preview_width <= GIF_PREVIEW_EDGE && preview_height <= GIF_PREVIEW_EDGE);
        assert_eq!(count, 1);
    }

    #[test]
    fn gif_subframes_and_disposal_match_the_reference_compositor() {
        let mut bytes = Vec::new();
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 4, 4, &[255, 0, 0, 0, 0, 255]).unwrap();
            for (index, disposal) in [
                gif::DisposalMethod::Keep,
                gif::DisposalMethod::Previous,
                gif::DisposalMethod::Background,
                gif::DisposalMethod::Keep,
            ]
            .into_iter()
            .enumerate()
            {
                let frame = gif::Frame {
                    width: 2,
                    height: 2,
                    left: (index % 2) as u16,
                    top: (index / 2) as u16,
                    dispose: disposal,
                    delay: 10,
                    buffer: std::borrow::Cow::Owned(vec![(index % 2) as u8; 4]),
                    ..Default::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        let expected = image::codecs::gif::GifDecoder::new(Cursor::new(&bytes))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        let preview = images::validate_image(&bytes)
            .unwrap()
            .animation_preview
            .unwrap();
        let actual = image::codecs::gif::GifDecoder::new(Cursor::new(preview))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(actual.len(), expected.len());
        for (expected, actual) in expected.iter().zip(actual) {
            assert_eq!(expected.buffer(), actual.buffer());
        }
    }

    #[test]
    fn gif_preview_preserves_frames_timing_and_repeat_and_copy_uses_first_frame() {
        let shared = include_bytes!("../../../tests/contracts/images-api-preview.gif");
        assert_eq!(
            gif_structure(shared, false).unwrap(),
            (16, 16, 2, gif::Repeat::Finite(3))
        );
        assert_eq!(
            images::decode_pixels(shared).unwrap().get_pixel(4, 4).0,
            [255, 0, 0, 255]
        );
        let bytes = animation(2);
        let info = images::validate_image(&bytes).unwrap();
        assert_eq!(
            (info.mime, info.extension, info.width, info.height),
            ("image/gif", "gif", 16, 16)
        );
        let preview = info.animation_preview.unwrap();
        assert_eq!(gif_structure(&preview, true).unwrap().2, 2);
        assert_eq!(
            gif_structure(&preview, true).unwrap().3,
            gif::Repeat::Finite(3)
        );
        let decoded = image::codecs::gif::GifDecoder::new(Cursor::new(&preview))
            .unwrap()
            .into_frames()
            .collect_frames()
            .unwrap();
        assert_eq!(decoded[0].delay().numer_denom_ms(), (100, 1));
        assert_eq!(decoded[1].delay().numer_denom_ms(), (120, 1));
        assert_eq!(decoded[0].buffer().get_pixel(4, 4).0, [255, 0, 0, 255]);
        assert_eq!(decoded[1].buffer().get_pixel(4, 4).0, [0, 0, 255, 255]);
        assert_eq!(
            images::decode_pixels(&bytes).unwrap().get_pixel(4, 4).0,
            [255, 0, 0, 255]
        );
        assert!(images::validate_input_image(&bytes).is_err());
        assert!(images::validate_image(&bytes[..bytes.len() - 1]).is_err());
        assert_eq!(
            images::validate_image(&animation(17)).err().unwrap().code,
            "image_limit_exceeded"
        );
        let mut huge = bytes.clone();
        huge[6..8].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            images::validate_image(&huge).err().unwrap().code,
            "image_limit_exceeded"
        );
    }
}
