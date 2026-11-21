//! Release-build timing of the decode hot path, for comparing build
//! settings (run: `cargo test --release --lib bench_ -- --ignored --nocapture`).
//! Ignored in the normal test run.

use std::time::Instant;

use crate::renderer::shared_frame::SharedFrame;

/// A terminal-like 1920x1080 picture: light glyph shapes on a dark
/// background, in BGRA for the encoder.
fn terminal_bgra(width: usize, height: usize, shift: usize) -> Vec<u8> {
    let mut data = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let (cell_x, cell_y) = ((x + shift * 9) / 9, y / 18);
            let (gx, gy) = ((x + shift * 9) % 9, y % 18);
            let glyph = (cell_x * 31 + cell_y * 17) % 7;
            let ink = gy > 3 && gy < 15 && gx < 7 && ((gx + gy + glyph) % 3 == 0 || gx == glyph);
            let color: [u8; 4] = if ink {
                [220, 220, 220, 255]
            } else {
                [30, 24, 20, 255]
            };
            data[(y * width + x) * 4..][..4].copy_from_slice(&color);
        }
    }
    data
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

#[test]
#[ignore = "timing for build comparisons; run with --ignored in release"]
fn bench_clearcodec_and_frame_store() {
    let (w, h) = (1920u16, 1080u16);
    let frames: Vec<Vec<u8>> = (0..8)
        .map(|shift| {
            ironrdp_graphics::clearcodec::ClearCodecEncoder::new().encode(
                &terminal_bgra(w.into(), h.into(), shift),
                w,
                h,
            )
        })
        .collect();

    let mut decode = Vec::new();
    for round in 0..40 {
        let data = &frames[round % frames.len()];
        let mut decoder = ironrdp_graphics::clearcodec::ClearCodecDecoder::new();
        let start = Instant::now();
        let rgba = decoder.decode(data, w, h).expect("decodes");
        decode.push(start.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(rgba);
    }

    // The native path: write a 1920x200 band (terminal scroll) into the
    // store and take it out again.
    let store = SharedFrame::new(w.into(), h.into());
    let band = vec![7u8; 1920 * 200 * 4];
    let mut copy = Vec::new();
    for round in 0..200 {
        let start = Instant::now();
        store.update_rect(0, (round % 800) as u16, 1920, 200, &band, 1920 * 4);
        std::hint::black_box(store.take_damage());
        copy.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    println!(
        "BENCH clearcodec_1080p_ms={:.3} frame_store_band_ms={:.3} encoded_kb={}",
        median(decode),
        median(copy),
        frames[0].len() / 1024
    );
}
