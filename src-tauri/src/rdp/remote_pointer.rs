//! The remote pointer, shown as the local cursor over the session canvas.
//!
//! IronRDP can composite the pointer into the desktop image
//! (`pointer_software_rendering`), but then every mouse move is a new frame.
//! On Linux each frame repaints the whole GTK window in software, which made
//! the pointer lag. Like mstsc, 1-1 asks for the shape instead and lets the
//! system draw the cursor: moving the mouse costs no frame at all.

use std::io::Cursor;
use std::sync::Arc;

use base64::Engine as _;
use ironrdp_graphics::pointer::DecodedPointer;
use serde::{Deserialize, Serialize};

/// A pointer shape for the web view: a PNG data URL with its hotspot, in
/// remote desktop pixels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointerImage {
    pub url: String,
    pub width: u16,
    pub height: u16,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
}

/// Shapes the server switches between (arrow, I-beam, resize) come back from
/// its pointer cache as the same decoded pointer, so they are encoded once.
const ENCODED_SHAPES: usize = 32;

/// What the host asked the client to show.
#[derive(Debug, Default)]
pub struct RemotePointer {
    hidden: bool,
    shape: Option<Arc<DecodedPointer>>,
    encoded: Vec<(Arc<DecodedPointer>, Option<PointerImage>)>,
}

impl RemotePointer {
    /// The system's default arrow.
    pub fn show_default(&mut self) {
        self.hidden = false;
        self.shape = None;
    }

    /// No pointer at all (the host hid it).
    pub fn hide(&mut self) {
        self.hidden = true;
        self.shape = None;
    }

    pub fn show(&mut self, pointer: Arc<DecodedPointer>) {
        self.hidden = false;
        if !self
            .encoded
            .iter()
            .any(|(known, _)| Arc::ptr_eq(known, &pointer))
        {
            if self.encoded.len() == ENCODED_SHAPES {
                self.encoded.remove(0);
            }
            let image = encode(&pointer);
            self.encoded.push((Arc::clone(&pointer), image));
        }
        self.shape = Some(pointer);
    }

    /// Identity of the current state, to publish only real changes.
    pub fn key(&self) -> (bool, usize) {
        (
            self.hidden,
            self.shape
                .as_ref()
                .map_or(0, |shape| Arc::as_ptr(shape) as usize),
        )
    }

    /// The local cursor is hidden: by the host, or because its shape is empty.
    pub fn is_hidden(&self) -> bool {
        self.hidden || (self.shape.is_some() && self.image().is_none())
    }

    pub fn image(&self) -> Option<&PointerImage> {
        let shape = self.shape.as_ref()?;
        self.encoded
            .iter()
            .find(|(known, _)| Arc::ptr_eq(known, shape))
            .and_then(|(_, image)| image.as_ref())
    }
}

/// PNG data URL of a decoded pointer; `None` for an empty or fully
/// transparent shape (shown as no cursor).
fn encode(pointer: &DecodedPointer) -> Option<PointerImage> {
    let (width, height) = (u32::from(pointer.width), u32::from(pointer.height));
    if width == 0
        || height == 0
        || pointer
            .bitmap_data
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| pixel[3] == 0)
    {
        return None;
    }
    let image = image::RgbaImage::from_raw(width, height, pointer.bitmap_data.clone())?;
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .inspect_err(|error| log::warn!("Could not encode the remote pointer: {error}"))
        .ok()?;
    Some(PointerImage {
        url: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ),
        width: pointer.width,
        height: pointer.height,
        hotspot_x: pointer.hotspot_x.min(pointer.width - 1),
        hotspot_y: pointer.hotspot_y.min(pointer.height - 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(width: u16, height: u16, alpha: u8) -> Arc<DecodedPointer> {
        Arc::new(DecodedPointer {
            width,
            height,
            hotspot_x: 1,
            hotspot_y: 2,
            bitmap_data: [0x10, 0x20, 0x30, alpha].repeat(usize::from(width) * usize::from(height)),
        })
    }

    #[test]
    fn a_shape_becomes_a_png_cursor_with_its_hotspot() {
        let mut remote = RemotePointer::default();
        remote.show(pointer(4, 3, 0xff));
        let image = remote.image().unwrap();
        assert!(image.url.starts_with("data:image/png;base64,"));
        assert_eq!(
            (image.width, image.height, image.hotspot_x, image.hotspot_y),
            (4, 3, 1, 2)
        );
        let png = base64::engine::general_purpose::STANDARD
            .decode(image.url.trim_start_matches("data:image/png;base64,"))
            .unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (4, 3));
        assert_eq!(decoded.get_pixel(0, 0).0, [0x10, 0x20, 0x30, 0xff]);
        assert!(!remote.is_hidden());
    }

    #[test]
    fn hidden_default_and_transparent_shapes() {
        let mut remote = RemotePointer::default();
        assert!(!remote.is_hidden() && remote.image().is_none());
        remote.hide();
        assert!(remote.is_hidden());
        remote.show(pointer(2, 2, 0));
        assert!(
            remote.is_hidden(),
            "a fully transparent shape shows no cursor"
        );
        remote.show_default();
        assert!(!remote.is_hidden() && remote.image().is_none());
    }

    #[test]
    fn switching_back_to_a_cached_shape_reuses_its_encoding_and_changes_the_key() {
        let mut remote = RemotePointer::default();
        let (arrow, beam) = (pointer(2, 2, 0xff), pointer(3, 3, 0xff));
        remote.show(Arc::clone(&arrow));
        let arrow_key = remote.key();
        remote.show(Arc::clone(&beam));
        assert_ne!(remote.key(), arrow_key);
        remote.show(arrow);
        assert_eq!(remote.key(), arrow_key);
        assert_eq!(remote.encoded.len(), 2);
    }
}
