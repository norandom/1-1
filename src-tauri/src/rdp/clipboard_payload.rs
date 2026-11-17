use base64::Engine;
use image::{DynamicImage, ImageDecoder, ImageEncoder, RgbaImage};
use std::io::Cursor;

pub const MAX_PNG_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 16 * 1024 * 1024;
pub const MAX_WIRE_BYTES: usize = 64 * 1024 * 1024 + 4096;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
const IMAGE_ERROR: &str =
    "Clipboard image is invalid or exceeds the 16 MiB PNG / 16 megapixel limit.";

/// Text remains wire-compatible with the existing command; images are bounded PNG.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum ClipboardPayload {
    Text(String),
    Image { png: String },
}
impl std::fmt::Debug for ClipboardPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Text(_) => "ClipboardPayload::Text(<redacted>)",
            Self::Image { .. } => "ClipboardPayload::Image(<redacted>)",
        })
    }
}
fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        Err(IMAGE_ERROR.into())
    } else {
        Ok(())
    }
}
fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    limits
}
fn decode<D: ImageDecoder>(mut decoder: D) -> Result<RgbaImage, String> {
    let (width, height) = decoder.dimensions();
    dimensions(width, height)?;
    decoder.set_limits(limits()).map_err(|_| IMAGE_ERROR)?;
    DynamicImage::from_decoder(decoder)
        .map(|image| image.into_rgba8())
        .map_err(|_| IMAGE_ERROR.into())
}
impl ClipboardPayload {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Text(text) if text.len() > MAX_TEXT_BYTES => {
                Err("Clipboard text exceeds the 1 MiB limit.".into())
            }
            Self::Text(_) => Ok(()),
            Self::Image { .. } => self.rgba().map(|_| ()),
        }
    }
    pub fn png_bytes(&self) -> Result<Vec<u8>, String> {
        let Self::Image { png } = self else {
            return Err(IMAGE_ERROR.into());
        };
        if png.len() > MAX_PNG_BYTES.div_ceil(3) * 4 {
            return Err(IMAGE_ERROR.into());
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(png)
            .map_err(|_| IMAGE_ERROR)?;
        if bytes.len() > MAX_PNG_BYTES {
            return Err(IMAGE_ERROR.into());
        }
        Ok(bytes)
    }
    pub fn rgba(&self) -> Result<RgbaImage, String> {
        let bytes = self.png_bytes()?;
        let decoder = image::codecs::png::PngDecoder::with_limits(Cursor::new(bytes), limits())
            .map_err(|_| IMAGE_ERROR)?;
        decode(decoder)
    }
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Result<Self, String> {
        dimensions(width, height)?;
        if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err(IMAGE_ERROR.into());
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
            .map_err(|_| IMAGE_ERROR)?;
        if png.len() > MAX_PNG_BYTES {
            return Err(IMAGE_ERROR.into());
        }
        Ok(Self::Image {
            png: base64::engine::general_purpose::STANDARD.encode(png),
        })
    }
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PNG_BYTES {
            return Err(IMAGE_ERROR.into());
        }
        let payload = Self::Image {
            png: base64::engine::general_purpose::STANDARD.encode(bytes),
        };
        payload.validate()?;
        Ok(payload)
    }
    pub fn from_dib(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_WIRE_BYTES {
            return Err(IMAGE_ERROR.into());
        }
        let field = |offset: usize| -> Option<u32> {
            Some(u32::from_le_bytes(
                bytes.get(offset..offset + 4)?.try_into().ok()?,
            ))
        };
        if field(0) == Some(124) && (field(112) != Some(0) || field(116) != Some(0)) {
            return Err("Clipboard DIB with an external color profile is unsupported.".into());
        }
        // image 0.25's headerless path skips 12 bytes after V4/V5 embedded
        // masks. An ordinary BMP file header supplies the correct pixel offset;
        // the same upstream decoder still handles all pixel formats and masks.
        if matches!(field(0), Some(108 | 124)) && field(16) == Some(3) {
            let header_size = field(0).ok_or(IMAGE_ERROR)? as usize;
            if bytes.len() < header_size
                || field(32) != Some(0)
                || (header_size == 124 && (field(112) != Some(0) || field(116) != Some(0)))
            {
                return Err(
                    "Clipboard DIB with an external color profile or palette is unsupported."
                        .into(),
                );
            }
            let mut bmp = Vec::with_capacity(bytes.len() + 14);
            bmp.extend_from_slice(b"BM");
            bmp.extend_from_slice(&((bytes.len() + 14) as u32).to_le_bytes());
            bmp.extend_from_slice(&[0; 4]);
            bmp.extend_from_slice(&((header_size + 14) as u32).to_le_bytes());
            bmp.extend_from_slice(bytes);
            let decoder =
                image::codecs::bmp::BmpDecoder::new(Cursor::new(bmp)).map_err(|_| IMAGE_ERROR)?;
            let rgba = decode(decoder)?;
            return Self::from_rgba(rgba.width(), rgba.height(), rgba.as_raw());
        }
        let decoder = image::codecs::bmp::BmpDecoder::new_without_file_header(Cursor::new(bytes))
            .map_err(|_| IMAGE_ERROR)?;
        let rgba = decode(decoder)?;
        Self::from_rgba(rgba.width(), rgba.height(), rgba.as_raw())
    }
    pub fn to_dib(&self) -> Result<Vec<u8>, String> {
        let rgba = self.rgba()?;
        // Standard CF_DIB uses a 40-byte BITMAPINFOHEADER / RGB24; PNG retains alpha.
        let rgb = DynamicImage::ImageRgba8(rgba).into_rgb8();
        let mut bmp = Vec::new();
        image::codecs::bmp::BmpEncoder::new(&mut bmp)
            .encode(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|_| IMAGE_ERROR)?;
        Ok(bmp.split_off(14)) // Strip the BMP file header, retaining the encoder's DIB.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_image_roundtrip_and_limits() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 255];
        let payload = ClipboardPayload::from_rgba(2, 1, &rgba).unwrap();
        let dib = payload.to_dib().unwrap();
        let decoded = ClipboardPayload::from_dib(&dib).unwrap();
        assert_eq!(decoded.rgba().unwrap().into_raw(), rgba);
        assert!(ClipboardPayload::from_rgba(u32::MAX, 2, &[]).is_err());
        assert!(ClipboardPayload::from_dib(&[0; 40]).is_err());
        assert!(ClipboardPayload::from_png(b"not a PNG").is_err());
        let mut oversized_header = dib;
        oversized_header[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(ClipboardPayload::from_dib(&oversized_header).is_err());
        assert!(
            ClipboardPayload::Image {
                png: "A".repeat(MAX_PNG_BYTES.div_ceil(3) * 4 + 4)
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn clipboard_png_and_dibv5_retain_alpha_and_dimensions() {
        let rgba = [11, 22, 33, 128];
        let payload = ClipboardPayload::from_rgba(1, 1, &rgba).unwrap();
        assert_eq!(
            ClipboardPayload::from_png(&payload.png_bytes().unwrap())
                .unwrap()
                .rgba()
                .unwrap()
                .into_raw(),
            rgba
        );
        let mut bmp = Vec::new();
        image::codecs::bmp::BmpEncoder::new(&mut bmp)
            .encode(&rgba, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        let mut dib = bmp.split_off(14);
        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 108);
        dib.splice(108..108, [0u8; 16]);
        dib[0..4].copy_from_slice(&124u32.to_le_bytes());
        assert_eq!(
            ClipboardPayload::from_dib(&dib)
                .unwrap()
                .rgba()
                .unwrap()
                .into_raw(),
            rgba
        );
        dib[112..116].copy_from_slice(&124u32.to_le_bytes());
        assert!(ClipboardPayload::from_dib(&dib).is_err());
    }
}
