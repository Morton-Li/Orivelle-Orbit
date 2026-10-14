use crate::{
    error::{Result, fail},
    images,
};

/// An owned, bounded RGBA image. It carries no filesystem path or credentials.
pub struct ClipboardImage {
    width: usize,
    height: usize,
    bytes: Vec<u8>,
}
impl ClipboardImage {
    /// The export plan already validated these immutable encoded bytes.
    pub(crate) fn decode(bytes: &[u8], dimensions: (u32, u32)) -> Result<Self> {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| fail("invalid_image"))?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(images::MAX_DECODE_BYTES);
        limits.max_image_width = Some(images::MAX_DIMENSION);
        limits.max_image_height = Some(images::MAX_DIMENSION);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|_| fail("invalid_image"))?
            .into_rgba8();
        if image.dimensions() != dimensions
            || image.as_raw().len() as u64 > images::MAX_DECODE_BYTES
        {
            return Err(fail("image_limit_exceeded"));
        }
        Ok(Self {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: image.into_raw(),
        })
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Trusted synchronous native sink; implementations must not reenter Service.
/// Service holds its account gate across this final external write.
pub trait ImageClipboardSink {
    fn set_image(&mut self, image: ClipboardImage) -> Result<()>;
}
