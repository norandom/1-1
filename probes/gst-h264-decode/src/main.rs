//! Bench: encode test streams with the installed GStreamer encoders, then
//! decode them through `GstH264Decoder` with every installed decoder.
//!
//! Answers per decoder: does every call return its own picture (no held-back
//! frames), how long does a call take (p50/p95/max), how much CPU does it
//! use, and is the output bit-exact with the software reference.
//!
//! Usage: probe-gst-h264-decode [frames] [WxH ...]

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use ironrdp_egfx::decode::H264Decoder;
use probe_gst_h264_decode::GstH264Decoder;

const DECODERS: &[&str] = &[
    "openh264dec", // software reference first
    "avdec_h264",
    "vah264dec",
    "varenderD129h264dec",
    "varenderD130h264dec",
    "vaapih264dec",
    "nvh264dec",
];

/// (label, encoder element with options). Every one is set up without B-frames,
/// as RDP servers encode.
const ENCODERS: &[(&str, &str)] = &[
    (
        "openh264enc baseline",
        "openh264enc complexity=low rate-control=bitrate bitrate=8000000 gop-size=120",
    ),
    (
        "vah264lpenc high",
        "vah264lpenc b-frames=0 key-int-max=120 ! video/x-h264,profile=high",
    ),
    (
        "vah264enc main",
        "vah264enc b-frames=0 key-int-max=120 ! video/x-h264,profile=main",
    ),
    (
        "varenderD129h264enc main",
        "varenderD129h264enc b-frames=0 key-int-max=120 ! video/x-h264,profile=main",
    ),
];

fn main() {
    gst::init().expect("GStreamer init");
    let mut args = std::env::args().skip(1);
    let frames: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(180);
    let mut sizes: Vec<(u32, u32)> = args
        .filter_map(|a| {
            a.split_once('x')
                .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
        })
        .collect();
    if sizes.is_empty() {
        sizes = vec![(1920, 1080), (3184, 1272), (3840, 2160)];
    }

    let installed: Vec<&str> = DECODERS
        .iter()
        .copied()
        .filter(|d| gst::ElementFactory::find(d).is_some())
        .collect();
    println!(
        "GStreamer {}; decoders installed: {installed:?}",
        gst::version_string()
    );

    for &(width, height) in &sizes {
        for &(label, encoder) in ENCODERS {
            let first = encoder.split_whitespace().next().unwrap();
            if gst::ElementFactory::find(first).is_none() {
                continue;
            }
            let stream = match encode(encoder, width, height, frames) {
                Ok(stream) if !stream.is_empty() => stream,
                Ok(_) => {
                    println!("\n## {width}x{height} {label}: encoder produced nothing");
                    continue;
                }
                Err(error) => {
                    println!("\n## {width}x{height} {label}: encode failed: {error}");
                    continue;
                }
            };
            let bytes: usize = stream.iter().map(Vec::len).sum();
            println!(
                "\n## {width}x{height}, {label}: {} access units, {:.0} KiB/frame avg",
                stream.len(),
                bytes as f64 / stream.len() as f64 / 1024.0
            );
            println!(
                "| decoder | ok | held/err | p50 ms | p95 ms | max ms | CPU ms/frame | to_rgba ms | fast ms (=) | bit-exact vs reference |"
            );
            println!("|---|---|---|---|---|---|---|---|---|---|");
            let mut reference: Option<Vec<u64>> = None;
            for &decoder in &installed {
                let result = bench(decoder, &stream);
                let exact = match (&reference, &result.hashes) {
                    (None, hashes) if !hashes.is_empty() => {
                        reference = Some(hashes.clone());
                        "reference".to_owned()
                    }
                    (Some(reference), hashes) => {
                        // Also try shifted alignments, to tell late pictures from wrong ones.
                        let (best_shift, same) = (-3i32..=3)
                            .map(|shift| {
                                let same = (0..reference.len())
                                    .filter(|&i| {
                                        let j = i as i32 + shift;
                                        j >= 0
                                            && (j as usize) < hashes.len()
                                            && hashes[j as usize] != 0
                                            && hashes[j as usize] == reference[i]
                                    })
                                    .count();
                                (shift, same)
                            })
                            .max_by_key(|&(_, same)| same)
                            .unwrap();
                        format!("{same}/{} (shift {best_shift})", reference.len())
                    }
                    _ => "-".to_owned(),
                };
                println!(
                    "| {decoder} | {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} ({}) | {exact} |",
                    result.ok,
                    result.failed_note,
                    result.percentile(0.50),
                    result.percentile(0.95),
                    result.percentile(1.0),
                    result.cpu_ms_per_frame,
                    result.to_rgba_ms,
                    result.fast_rgba_ms,
                    if result.fast_equal { "=" } else { "DIFF" },
                );
            }
        }
    }
}

struct Result {
    ok: usize,
    failed_note: String,
    latencies_ms: Vec<f64>,
    cpu_ms_per_frame: f64,
    to_rgba_ms: f64,
    fast_rgba_ms: f64,
    fast_equal: bool,
    hashes: Vec<u64>,
}

impl Result {
    fn percentile(&self, p: f64) -> f64 {
        if self.latencies_ms.is_empty() {
            return f64::NAN;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_by(f64::total_cmp);
        sorted[((sorted.len() - 1) as f64 * p).round() as usize]
    }
}

fn bench(element: &str, stream: &[Vec<u8>]) -> Result {
    let mut result = Result {
        ok: 0,
        failed_note: String::new(),
        latencies_ms: Vec::new(),
        cpu_ms_per_frame: f64::NAN,
        to_rgba_ms: f64::NAN,
        fast_rgba_ms: f64::NAN,
        fast_equal: false,
        hashes: Vec::new(),
    };
    let mut decoder = match GstH264Decoder::new(element, Duration::from_millis(250)) {
        Ok(decoder) => decoder,
        Err(error) => {
            result.failed_note = error.to_string();
            return result;
        }
    };
    if !decoder.wait_until_ready() {
        result.failed_note = "did not start".into();
        return result;
    }
    let (mut held, mut errors, mut first_error) = (0, 0, None);
    let cpu_start = cpu_ms();
    let mut last = None;
    for au in stream {
        let start = Instant::now();
        match decoder.decode_yuv420(au) {
            Ok(picture) => {
                result
                    .latencies_ms
                    .push(start.elapsed().as_secs_f64() * 1000.0);
                result.ok += 1;
                let mut hasher = DefaultHasher::new();
                for row in 0..usize::from(picture.height()) {
                    picture.luma_row(row).hash(&mut hasher);
                }
                for row in 0..usize::from(picture.height().div_ceil(2)) {
                    picture.u_row(row).hash(&mut hasher);
                    picture.v_row(row).hash(&mut hasher);
                }
                result.hashes.push(hasher.finish());
                if std::env::var_os("PROBE_DEBUG").is_some() && result.hashes.len() <= 3 {
                    eprintln!(
                        "  {element} #{}: {}x{} y0[..8]={:?}",
                        result.hashes.len() - 1,
                        picture.width(),
                        picture.height(),
                        &picture.luma_row(0).unwrap()[..8]
                    );
                }
                last = Some(picture);
            }
            Err(error) => {
                let text = error.to_string();
                if text.contains("held the picture back") {
                    held += 1;
                } else {
                    errors += 1;
                }
                if std::env::var_os("PROBE_DEBUG").is_some() {
                    eprintln!("  {element} #{}: {text}", result.hashes.len());
                }
                first_error.get_or_insert(text);
                // Keep the hash list aligned with frame numbers.
                result.hashes.push(0);
            }
        }
    }
    result.cpu_ms_per_frame = (cpu_ms() - cpu_start) / stream.len() as f64;
    if let Some(picture) = last {
        let start = Instant::now();
        let upstream = std::hint::black_box(picture.to_rgba());
        result.to_rgba_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let fast = std::hint::black_box(probe_gst_h264_decode::fast_to_rgba(&picture));
        result.fast_rgba_ms = start.elapsed().as_secs_f64() * 1000.0;
        result.fast_equal = fast == upstream.data();
    }
    let late = decoder.drain_late();
    result.failed_note = format!("{held} held, {errors} err, {late} late");
    if let Some(error) = first_error {
        eprintln!("  {element}: first failure: {error}");
    }
    result
}

/// Encodes a moving test pattern into byte-stream access units.
fn encode(
    encoder: &str,
    width: u32,
    height: u32,
    frames: usize,
) -> std::result::Result<Vec<Vec<u8>>, String> {
    let description = format!(
        "videotestsrc pattern=smpte horizontal-speed=4 num-buffers={frames} \
         ! video/x-raw,width={width},height={height},framerate=60/1 \
         ! videoconvert \
         ! {encoder} \
         ! h264parse config-interval=-1 \
         ! video/x-h264,stream-format=byte-stream,alignment=au \
         ! appsink name=sink sync=false"
    );
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| e.to_string())?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline".to_owned())?;
    let sink = pipeline
        .by_name("sink")
        .unwrap()
        .downcast::<gst_app::AppSink>()
        .unwrap();
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("{e:?}"))?;
    let mut out = Vec::new();
    while let Ok(sample) = sink.pull_sample() {
        if let Some(buffer) = sample.buffer() {
            out.push(buffer.map_readable().map_err(|e| e.to_string())?.to_vec());
        }
    }
    let error = pipeline
        .bus()
        .and_then(|bus| bus.pop_filtered(&[gst::MessageType::Error]));
    let _ = pipeline.set_state(gst::State::Null);
    match error.map(|m| match m.view() {
        gst::MessageView::Error(e) => e.error().to_string(),
        _ => String::new(),
    }) {
        Some(error) if out.is_empty() => Err(error),
        _ => Ok(out),
    }
}

/// User + system CPU time of the whole process (all threads), in ms.
fn cpu_ms() -> f64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit_once(')').map(|(_, rest)| rest).unwrap_or("");
    let fields: Vec<&str> = after.split_whitespace().collect();
    let ticks: f64 = fields
        .get(11..13)
        .map(|f| f.iter().filter_map(|t| t.parse::<f64>().ok()).sum())
        .unwrap_or(0.0);
    ticks * 10.0 // USER_HZ = 100
}
