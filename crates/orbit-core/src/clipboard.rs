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
        let image = images::decode_pixels(bytes)?;
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
