use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp::session::image::DecodedImage;

use crate::rdp::display::FrameBuffer;

pub const PACKET_KIND_FULL_FRAME: u8 = 1;
pub const PACKET_KIND_DIRTY_RECTS: u8 = 2;
pub const PACKET_KIND_H264: u8 = 3;

const PACKET_HEADER_LEN: usize = 1 + 2 + 2 + 2;
const RECT_HEADER_LEN: usize = 2 + 2 + 2 + 2;
const BYTES_PER_PIXEL: usize = 4;

pub fn encode_full_frame_packet(frame_buffer: &FrameBuffer) -> Vec<u8> {
    let rect = InclusiveRectangle {
        left: 0,
        top: 0,
        right: frame_buffer.width.saturating_sub(1) as u16,
        bottom: frame_buffer.height.saturating_sub(1) as u16,
    };

    encode_packet(
        PACKET_KIND_FULL_FRAME,
        frame_buffer.width as u16,
        frame_buffer.height as u16,
        &[rect],
        &frame_buffer.data,
    )
}

pub fn encode_image_update_packet(
    image: &DecodedImage,
    rects: &[InclusiveRectangle],
    scratch: &mut Vec<u8>,
) -> Vec<u8> {
    debug_assert_eq!(image.bytes_per_pixel(), BYTES_PER_PIXEL);

    scratch.clear();
    scratch.reserve(
        rects
            .iter()
            .map(|rect| usize::from(rect.width()) * usize::from(rect.height()) * BYTES_PER_PIXEL)
            .sum(),
    );

    append_rect_pixels(scratch, image.data(), image.stride(), rects);

    encode_packet(
        PACKET_KIND_DIRTY_RECTS,
        image.width(),
        image.height(),
        rects,
        scratch,
    )
}

/// Append the RGBA payload for `rects`, row by row, from a strided source
/// buffer into `scratch` in rect order. Extracted verbatim from
/// `encode_image_update_packet` (task 3.4 seam) so stride handling can be
/// characterized with synthetic data.
fn append_rect_pixels(
    scratch: &mut Vec<u8>,
    image_data: &[u8],
    stride: usize,
    rects: &[InclusiveRectangle],
) {
    for rect in rects {
        let row_bytes = usize::from(rect.width()) * BYTES_PER_PIXEL;
        let x_offset = usize::from(rect.left) * BYTES_PER_PIXEL;

        for y in rect.top..=rect.bottom {
            let start = usize::from(y) * stride + x_offset;
            let end = start + row_bytes;
            scratch.extend_from_slice(&image_data[start..end]);
        }
    }
}

/// Encode an H.264 bitstream into a frame transport packet.
/// Layout: [kind=3 (1 byte)] [width (2 bytes LE)] [height (2 bytes LE)] [h264_data_len (4 bytes LE)] [h264_data...]
pub fn encode_h264_packet(width: u16, height: u16, h264_data: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(1 + 2 + 2 + 4 + h264_data.len());
    packet.push(PACKET_KIND_H264);
    packet.extend_from_slice(&width.to_le_bytes());
    packet.extend_from_slice(&height.to_le_bytes());
    packet.extend_from_slice(&(h264_data.len() as u32).to_le_bytes());
    packet.extend_from_slice(h264_data);
    packet
}

fn encode_packet(
    kind: u8,
    surface_width: u16,
    surface_height: u16,
    rects: &[InclusiveRectangle],
    pixels: &[u8],
) -> Vec<u8> {
    let mut packet =
        Vec::with_capacity(PACKET_HEADER_LEN + rects.len() * RECT_HEADER_LEN + pixels.len());

    packet.push(kind);
    packet.extend_from_slice(&surface_width.to_le_bytes());
    packet.extend_from_slice(&surface_height.to_le_bytes());
    packet.extend_from_slice(&(rects.len() as u16).to_le_bytes());

    for rect in rects {
        packet.extend_from_slice(&rect.left.to_le_bytes());
        packet.extend_from_slice(&rect.top.to_le_bytes());
        packet.extend_from_slice(&rect.width().to_le_bytes());
        packet.extend_from_slice(&rect.height().to_le_bytes());
    }

    packet.extend_from_slice(pixels);
    packet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_frame_packet_contains_header_and_pixels() {
        let frame = FrameBuffer {
            width: 2,
            height: 1,
            data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };

        let packet = encode_full_frame_packet(&frame);

        assert_eq!(packet[0], PACKET_KIND_FULL_FRAME);
        assert_eq!(u16::from_le_bytes([packet[1], packet[2]]), 2);
        assert_eq!(u16::from_le_bytes([packet[3], packet[4]]), 1);
        assert_eq!(u16::from_le_bytes([packet[5], packet[6]]), 1);
        assert_eq!(u16::from_le_bytes([packet[7], packet[8]]), 0);
        assert_eq!(u16::from_le_bytes([packet[9], packet[10]]), 0);
        assert_eq!(u16::from_le_bytes([packet[11], packet[12]]), 2);
        assert_eq!(u16::from_le_bytes([packet[13], packet[14]]), 1);
        assert_eq!(&packet[15..], frame.data.as_slice());
    }

    fn rect(left: u16, top: u16, right: u16, bottom: u16) -> InclusiveRectangle {
        InclusiveRectangle {
            left,
            top,
            right,
            bottom,
        }
    }

    /// RGBA source of a 4x2 image with a distinct value per pixel-byte
    /// (pixel (x, y) is filled with the byte 16*y + x*4 .. +3).
    fn synthetic_4x2() -> Vec<u8> {
        (0..2u16)
            .flat_map(|y| (0..4u16).flat_map(move |x| [(16 * y + 4 * x) as u8; 4]))
            .collect()
    }

    #[test]
    fn dirty_rect_payload_extracts_rows_using_the_source_stride() {
        // Stride wider than the row bytes proves rows are walked by stride,
        // not packed: each 32-byte row holds 4 pixels (16 value bytes,
        // value 16*y) followed by 16 bytes of 0xEE padding.
        let source: Vec<u8> = (0..2u16)
            .flat_map(|y| {
                let mut row = vec![(16 * y) as u8; 16]; // 4 pixels of value 16*y
                row.resize(32, 0xEE);
                row
            })
            .collect();

        let mut scratch = Vec::new();
        append_rect_pixels(&mut scratch, &source, 32, &[rect(1, 0, 2, 1)]);

        // 2x2 rect (left=1..=2, top=0..=1): row 0 pixels read from bytes
        // 4..12 (value 0), row 1 from 36..44 (value 16); the 0xEE padding
        // is never read.
        assert_eq!(scratch.len(), 2 * 2 * BYTES_PER_PIXEL);
        assert_eq!(&scratch[..8], &[0u8; 8]);
        assert_eq!(&scratch[8..], &[16u8; 8]);
        assert!(!scratch.contains(&0xEE));
    }

    #[test]
    fn dirty_rect_payload_concatenates_multiple_rects_in_order() {
        let source = synthetic_4x2();
        let mut scratch = Vec::new();

        append_rect_pixels(
            &mut scratch,
            &source,
            16,
            &[rect(0, 0, 0, 0), rect(3, 1, 3, 1)],
        );

        // First rect: pixel (0,0) = bytes 0..4; second rect: pixel (3,1)
        // starts at 1*16 + 3*4 = byte 28.
        assert_eq!(&scratch[..4], &source[0..4]);
        assert_eq!(&scratch[4..], &source[28..32]);
    }

    #[test]
    fn encode_image_update_packet_layout_tracks_the_decoded_image() {
        use ironrdp::graphics::image_processing::PixelFormat;
        use ironrdp::session::image::DecodedImage;

        // DecodedImage has no public data setter; a zeroed image pins the
        // delegation (kind, surface dims, rect headers, payload length).
        let image = DecodedImage::new(PixelFormat::RgbA32, 4, 2);
        let mut scratch = Vec::new();

        let packet = encode_image_update_packet(&image, &[rect(1, 0, 2, 1)], &mut scratch);

        assert_eq!(packet[0], PACKET_KIND_DIRTY_RECTS);
        assert_eq!(u16::from_le_bytes([packet[1], packet[2]]), 4);
        assert_eq!(u16::from_le_bytes([packet[3], packet[4]]), 2);
        assert_eq!(u16::from_le_bytes([packet[5], packet[6]]), 1);
        assert_eq!(u16::from_le_bytes([packet[7], packet[8]]), 1); // left
        assert_eq!(u16::from_le_bytes([packet[9], packet[10]]), 0); // top
        assert_eq!(u16::from_le_bytes([packet[11], packet[12]]), 2); // width
        assert_eq!(u16::from_le_bytes([packet[13], packet[14]]), 2); // height
        assert_eq!(packet.len(), 15 + 2 * 2 * BYTES_PER_PIXEL);
        assert!(packet[15..].iter().all(|&b| b == 0));
    }

    #[test]
    fn h264_packet_layout_is_kind3_with_explicit_length() {
        let packet = encode_h264_packet(2, 1, &[0x11, 0x22, 0x33, 0x44]);

        assert_eq!(packet[0], PACKET_KIND_H264);
        assert_eq!(u16::from_le_bytes([packet[1], packet[2]]), 2);
        assert_eq!(u16::from_le_bytes([packet[3], packet[4]]), 1);
        assert_eq!(
            u32::from_le_bytes([packet[5], packet[6], packet[7], packet[8]]),
            4
        );
        assert_eq!(&packet[9..], &[0x11, 0x22, 0x33, 0x44]);
    }
}
