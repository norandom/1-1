//! Task 3.5 — reproducible performance and interoperability baselines.
//!
//! Two explicitly-invoked harnesses live here, plus the deterministic pure
//! helpers they share. Neither harness runs in normal test tiers:
//!
//! - [`real_server_tls_nla_smoke`] (`#[ignore]`): protocol-level, headless
//!   old-stack TLS/NLA smoke against the LAN hosts staged in
//!   `D:\BuildArtifacts\drift-rdp\secrets\rdp-smoke.env`
//!   (numbered triples `RDP_SMOKE_HOSTn/USERn/PASSn`, read data-driven;
//!   maintainer-defined estate 2026-11-12: host1 Windows 11 native RDP,
//!   host2 FreeRDP/Linux, host3 IronRDP/Linux). It uses
//!   the EXISTING connector (`crate::rdp::client::attempt_rdp_connection`)
//!   and records redacted evidence per design.md L453 (client/server build,
//!   auth policy, negotiated route, result, timings). Credentials are read
//!   from the process environment only — never printed, never persisted
//!   (Req 12.6/12.9).
//! - [`workload_matrix_baseline`] (`#[ignore]`): the design.md §Testing
//!   Strategy workload matrix (L481): deterministic text-scroll fixture —
//!   32 dirty 64x32 rectangles per frame at 30 updates/s plus one
//!   full-frame update per second — at `1x1920x1080`, `10x1920x1080` and
//!   `1x3840x2160`, 10 s warmup + 60 s sample, three repeats, median
//!   p50/p95 decode-ready-to-packet-emitted latency and delivered FPS.
//!   Process CPU/memory sampling is performed by the runner script
//!   (`scripts/Invoke-Baseline.ps1`), which polls the test process.
//!
//! Fixture route (recorded honestly in every artifact): the only
//! server-update route the old stack decodes headlessly without a live
//! server is an uncompressed 16-bpp FastPath `TS_UPDATE_BITMAP`
//! (`ironrdp-session` 0.8 rejects uncompressed 32-bpp; ironrdp-graphics 0.7
//! ships no RDP6/RLE bitmap *encoder*). The fixture therefore drives the
//! real `ActiveStage::process` decode -> `DecodedImage` -> `SharedFrame`
//! write -> `encode_image_update_packet` pipeline (the in-process prefix of
//! the design's decode-ready-to-canvas-consumed metric); the webview
//! canvas-consumed suffix is a recorded gap with a budget proposal.
//!
//! Measurement code (real wall-clock pacing and timing) is intentionally
//! isolated in the two `#[ignore]`d harnesses; the `tests` module below is
//! fully deterministic (Req 12.7 isolation: no filesystem, no network, no
//! sleeps).

// Every item in this module exists for the explicitly-invoked baseline
// harnesses and their deterministic unit tests; the shipping lib build does
// not call them (the harnesses are `#[ignore]`d tests + an external runner
// script), so dead-code analysis of the non-test lib would otherwise flag
// the entire module.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::time::Duration;

use ironrdp_connector::ConnectionResult;
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};

/// Text-scroll fixture geometry (design.md L481): 32 dirty rectangles of
/// 64x32 pixels per frame, arranged as a 16x2 band that scrolls downward
/// one tile row per frame and wraps.
pub const TEXT_RECT_WIDTH: u16 = 64;
pub const TEXT_RECT_HEIGHT: u16 = 32;
pub const TEXT_RECTS_PER_FRAME: usize = 32;
pub const TEXT_BAND_COLUMNS: usize = 16;
pub const TEXT_BAND_ROWS: usize = 2;

/// Fixture cadence: 30 updates/s, one full-frame update each second.
pub const UPDATES_PER_SECOND: u32 = 30;
/// A full-frame update replaces the 32-rect text update once per second.
pub const FRAMES_PER_FULL_FRAME: usize = 30;

/// Design workload matrix (design.md L481).
pub const WORKLOAD_MATRIX: &[(&str, u16, u16, usize)] = &[
    ("1x1920x1080", 1920, 1080, 1),
    ("10x1920x1080", 1920, 1080, 10),
    ("1x3840x2160", 3840, 2160, 1),
];

/// Redact a username to its fixed evidence label. The input value never
/// reaches the output, so no username content can leak into artifacts
/// regardless of what the environment contained.
pub fn mask_user(label: &str) -> String {
    format!("<{label}>")
}

/// Deterministic 5:6:5 pixel value for fixture content. A splitmix-style
/// integer hash of (frame, rect, x, y) keeps every run bit-identical while
/// producing glyph-like density variation between frames and columns.
pub fn fixture_pixel_565(frame: usize, rect: usize, x: usize, y: usize) -> u16 {
    let mut h = (frame as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ ((rect as u64) << 32)
        ^ ((x as u64) << 13)
        ^ (y as u64);
    h = h.wrapping_add(0x9E37_79B9_7F4A_7C15);
    h ^= h >> 30;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 31;
    h as u16
}

/// The 32 dirty rectangles of frame `frame` (design L481 text-scroll).
///
/// The band is `TEXT_BAND_COLUMNS * TEXT_RECT_WIDTH` wide (1024 px) and
/// `TEXT_BAND_ROWS * TEXT_RECT_HEIGHT` tall (64 px); its top edge starts at
/// `frame * TEXT_RECT_HEIGHT` and wraps around so it stays inside the
/// surface. All rectangles are exactly 64x32 and inside bounds.
pub fn text_scroll_rects(
    frame: usize,
    surface_width: u16,
    surface_height: u16,
) -> Vec<InclusiveRectangle> {
    let band_h = TEXT_RECT_HEIGHT as usize * TEXT_BAND_ROWS;
    debug_assert!(
        (TEXT_RECT_WIDTH as usize * TEXT_BAND_COLUMNS) <= surface_width as usize
            && band_h <= surface_height as usize,
        "fixture band must fit the surface"
    );
    let surface_h = surface_height as usize;
    let scroll_span = (surface_h - band_h).max(1);
    let band_top = ((frame * TEXT_RECT_HEIGHT as usize) % scroll_span) as u16;
    let _ = surface_width; // band fits 1024 px; asserted by callers' geometry

    let mut rects = Vec::with_capacity(TEXT_RECTS_PER_FRAME);
    for row in 0..TEXT_BAND_ROWS {
        for col in 0..TEXT_BAND_COLUMNS {
            let left = (col * TEXT_RECT_WIDTH as usize) as u16;
            let top = band_top + (row * TEXT_RECT_HEIGHT as usize) as u16;
            rects.push(InclusiveRectangle {
                left,
                top,
                right: left + TEXT_RECT_WIDTH - 1,
                bottom: top + TEXT_RECT_HEIGHT - 1,
            });
        }
    }
    rects
}

/// Expected union shape of the once-per-second full-frame update (design
/// L481). Full frames are ENCODED as bands ([`full_frame_band_rects`])
/// because `TS_BITMAP_DATA` cannot carry a whole surface in one rectangle.
pub fn full_frame_rect(surface_width: u16, surface_height: u16) -> InclusiveRectangle {
    InclusiveRectangle {
        left: 0,
        top: 0,
        right: surface_width.saturating_sub(1),
        bottom: surface_height.saturating_sub(1),
    }
}

/// Union of `rects` (the coalesced region the old-stack session loop
/// actually delivers per PDU — see session.rs L675+).
pub fn union_rects(rects: &[InclusiveRectangle]) -> InclusiveRectangle {
    let mut iter = rects.iter();
    let first = iter.next().expect("union_rects requires at least one rect");
    iter.fold(first.clone(), |acc, r| acc.union(r))
}

/// Fill the bottom-up 16-bpp bitmap rows for one 64x32 text rectangle.
///
/// `ironrdp-session`'s uncompressed path (`apply_rgb16_bitmap`) consumes
/// rows via `chunks_exact(width * 2).rev()`, i.e. bitmap row 0 is the
/// BOTTOM row of the rectangle; this fills that layout directly.
pub fn fill_text_rect_rows_16bpp(
    rows: &mut [u8],
    frame: usize,
    rect_idx: usize,
    width: usize,
    height: usize,
) {
    let row_len = width * 2;
    debug_assert_eq!(rows.len(), row_len * height);
    for chunk_row in 0..height {
        // chunk_row 0 is the bottom source row (y = height-1-chunk_row)
        let src_y = height - 1 - chunk_row;
        let row = &mut rows[chunk_row * row_len..(chunk_row + 1) * row_len];
        for x in 0..width {
            let px = fixture_pixel_565(frame, rect_idx, x, src_y);
            row[x * 2..x * 2 + 2].copy_from_slice(&px.to_le_bytes());
        }
    }
}

/// Maximum bytes of bitmap data a single `TS_BITMAP_DATA` rectangle may
/// carry (its length field is u16) — larger surfaces must be tiled.
pub const BITMAP_DATA_MAX_BYTES: usize = 65_535;

/// Band height for tiling a full-frame update into `TS_BITMAP_DATA` rects
/// that each respect [`BITMAP_DATA_MAX_BYTES`].
pub fn full_frame_band_height(width: u16) -> u16 {
    let rows = BITMAP_DATA_MAX_BYTES / (width as usize * 2);
    rows.clamp(1, u16::MAX as usize) as u16
}

/// The full-surface band tiling for the once-per-second full-frame update.
/// Bands are horizontal strips of [`full_frame_band_height`] rows (the last
/// band may be shorter); their union is the whole surface.
pub fn full_frame_band_rects(surface_width: u16, surface_height: u16) -> Vec<InclusiveRectangle> {
    let band_h = full_frame_band_height(surface_width) as u32;
    let h = surface_height as u32;
    let mut bands = Vec::with_capacity((h / band_h + 1) as usize);
    let mut top: u32 = 0;
    while top < h {
        let bh = band_h.min(h - top);
        bands.push(InclusiveRectangle {
            left: 0,
            top: top as u16,
            right: surface_width.saturating_sub(1),
            bottom: (top + bh - 1) as u16,
        });
        top += bh;
    }
    bands
}

/// Slice of the bottom-up full-surface row buffer that belongs to `band`.
///
/// `full_rows` holds the whole surface bottom-up (chunk row 0 = surface row
/// H-1). The uncompressed decoder also consumes each RECT's rows bottom-up,
/// so band chunk 0 must be the band's bottom row (surface row
/// `top + height - 1`), which in `full_rows` lives at chunk index
/// `H - top - height`.
pub fn full_frame_band_rows<'a>(
    full_rows: &'a [u8],
    surface_height: u16,
    band: &InclusiveRectangle,
) -> &'a [u8] {
    let row_len = band.width() as usize * 2;
    let bh = band.height() as usize;
    let top = band.top as usize;
    let h = surface_height as usize;
    let chunk0 = h - top - bh;
    &full_rows[chunk0 * row_len..(chunk0 + bh) * row_len]
}

/// Fill the bottom-up 16-bpp bitmap rows for a full-surface frame with
/// deterministic static content (generated once per run, reused each
/// second; content reuse is recorded in the route description).
pub fn fill_full_frame_rows_16bpp(rows: &mut [u8], width: usize, height: usize) {
    let row_len = width * 2;
    debug_assert_eq!(rows.len(), row_len * height);
    for chunk_row in 0..height {
        let src_y = height - 1 - chunk_row;
        let row = &mut rows[chunk_row * row_len..(chunk_row + 1) * row_len];
        for x in 0..width {
            let px = fixture_pixel_565(usize::MAX, usize::MAX, x, src_y);
            row[x * 2..x * 2 + 2].copy_from_slice(&px.to_le_bytes());
        }
    }
}

/// Maximum payload carried by one FastPath fragment. `TS_FP_UPDATE_PDU`
/// length fields are bounded (16-bit), so a 32-rect bitmap update
/// (~131 KiB uncompressed) must be fragmented exactly as a real server
/// fragments large updates; `CompleteData` in ironrdp-session reassembles
/// `First`/`Next`/`Last` chains before decoding.
const FASTPATH_FRAGMENT_MAX: usize = 60_000;

/// Encode one uncompressed 16-bpp FastPath bitmap-update PDU sequence
/// (`FastPathHeader` + fragmented `FastPathUpdatePdu(BitmapUpdateData)`),
/// the exact byte shape `ActiveStage::process(Action::FastPath, ..)`
/// decodes. Returns one Vec per fragment; feed fragments to `process` in
/// order — only the final fragment yields the graphics update.
pub fn encode_bitmap_update_pdu_16bpp(
    rects: &[InclusiveRectangle],
    rect_rows: &[&[u8]],
) -> Vec<Vec<u8>> {
    use ironrdp_pdu::bitmap::Compression;
    use ironrdp_pdu::encode_vec as pdu_encode_vec;
    use ironrdp_pdu::fast_path::Fragmentation;

    assert_eq!(rects.len(), rect_rows.len());
    for (idx, rows) in rect_rows.iter().enumerate() {
        assert!(
            rows.len() <= BITMAP_DATA_MAX_BYTES,
            "rect {idx}: TS_BITMAP_DATA carries at most {BITMAP_DATA_MAX_BYTES} bytes \
             (u16 length field) — tile large surfaces into bands"
        );
    }
    let rectangles = rects
        .iter()
        .zip(rect_rows)
        .map(|(rect, rows)| ironrdp_pdu::bitmap::BitmapData {
            rectangle: rect.clone(),
            width: rect.width(),
            height: rect.height(),
            bits_per_pixel: 16,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: rows,
        })
        .collect::<Vec<_>>();
    let update_bytes = pdu_encode_vec(&ironrdp_pdu::bitmap::BitmapUpdateData { rectangles })
        .expect("bitmap update encodes");

    let mut out = Vec::new();
    if update_bytes.len() <= FASTPATH_FRAGMENT_MAX {
        out.push(encode_fragment(&update_bytes, Fragmentation::Single));
        return out;
    }

    let mut chunks = update_bytes.chunks(FASTPATH_FRAGMENT_MAX).peekable();
    while let Some(chunk) = chunks.next() {
        let fragmentation = match (out.is_empty(), chunks.peek().is_some()) {
            (true, true) => Fragmentation::First,
            (true, false) => unreachable!("multi-chunk sequence always has a next chunk"),
            (false, true) => Fragmentation::Next,
            (false, false) => Fragmentation::Last,
        };
        out.push(encode_fragment(chunk, fragmentation));
    }
    out
}

fn encode_fragment(
    payload: &[u8],
    fragmentation: ironrdp_pdu::fast_path::Fragmentation,
) -> Vec<u8> {
    use ironrdp_pdu::encode_vec as pdu_encode_vec;
    use ironrdp_pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, UpdateCode};

    let header = FastPathHeader::new(EncryptionFlags::empty(), payload.len());
    let pdu = FastPathUpdatePdu {
        fragmentation,
        update_code: UpdateCode::Bitmap,
        compression_flags: None,
        compression_type: None,
        data: payload,
    };
    let mut out = pdu_encode_vec(&header).expect("fastpath header encodes");
    out.extend_from_slice(&pdu_encode_vec(&pdu).expect("fastpath update encodes"));
    out
}

/// Build a synthetic `ConnectionResult` so a real `ActiveStage` can decode
/// fixture PDUs headlessly (no server, no network). Uses the production
/// `build_connector_config` shape; channel ids mirror typical server values
/// (1003 io / 1004 user); the empty `StaticChannelSet` matches a connection
/// whose only attached channel (cliprdr) is irrelevant to graphics decoding.
pub fn synthetic_connection_result(width: u16, height: u16) -> ConnectionResult {
    use ironrdp_connector::DesktopSize;
    use ironrdp_connector::connection_activation::ConnectionActivationFactory;

    let config = crate::store::connections::ConnectionConfig {
        id: "baseline-fixture".to_string(),
        name: "baseline-fixture".to_string(),
        host: "127.0.0.1".to_string(),
        port: 3389,
        username: String::new(),
        domain: String::new(),
        color_accent: None,
        tags: vec![],
        display_width: None,
        display_height: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        last_connected_at: None,
        mock: false,
    };
    let connector_config =
        crate::rdp::client::build_connector_config(&config, "", width as u32, height as u32);

    // 0.12 ConnectionResult: `connection_activation` became `activation_factory`
    // (same constructor shape), and the negotiation fields the session layer now
    // consumes are explicit. The fixture pins the neutral values that the old
    // stack produced implicitly: no message channel, no compression, and no
    // refresh/suppress support asserted.
    ConnectionResult {
        io_channel_id: 1003,
        user_channel_id: 1004,
        message_channel_id: None,
        share_id: 0,
        // `ironrdp_svc` carries no default feature wiring into this build; the
        // empty set is expressed through `Default` instead of naming the type
        // (an empty channel set is exactly what fixtures need — graphics only).
        static_channels: Default::default(),
        desktop_size: DesktopSize { width, height },
        input_flags: ironrdp_pdu::rdp::capability_sets::InputFlags::empty(),
        enable_server_pointer: false,
        pointer_software_rendering: false,
        refresh_rect_support: false,
        suppress_output_support: false,
        window_support_level: None,
        monitor_layout: None,
        activation_factory: ConnectionActivationFactory::new(connector_config, 1003, 1004),
        compression_type: None,
    }
}

/// Percentile helper (nearest-rank on a sorted copy); `pct` in `0..=100`.
pub fn percentile_us(sorted_samples: &[f64], pct: f64) -> f64 {
    if sorted_samples.is_empty() {
        return 0.0;
    }
    let idx = ((pct / 100.0) * (sorted_samples.len() - 1) as f64).round() as usize;
    sorted_samples[idx.min(sorted_samples.len() - 1)]
}

/// The fixture frame schedule as a pure function: `true` when frame
/// `frame_idx` (0-based, 30 per second) is the once-per-second full-frame
/// update.
pub fn frame_is_full_frame(frame_idx: usize) -> bool {
    frame_idx.is_multiple_of(FRAMES_PER_FULL_FRAME)
}

/// Warmup/sample/repeat durations (design L481). `DRIFT_BENCH_QUICK=1`
/// shrinks them so the harness itself can be validated cheaply; it never
/// changes the fixture or metric definitions.
pub fn bench_durations() -> (Duration, Duration, usize) {
    if std::env::var("DRIFT_BENCH_QUICK")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        (Duration::from_secs(1), Duration::from_secs(3), 1)
    } else {
        (Duration::from_secs(10), Duration::from_secs(60), 3)
    }
}

/// Default output directory for baseline artifacts (never a repository
/// path; override with `DRIFT_BASELINE_OUT_DIR`).
pub fn baseline_out_dir() -> std::path::PathBuf {
    std::env::var("DRIFT_BASELINE_OUT_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(r"D:\BuildArtifacts\drift-rdp\perf-baseline"))
}

/// Sorted per-workload sample summary reused by the report writer.
#[derive(serde::Serialize, Debug, Clone)]
pub struct WorkloadStat {
    pub workload: String,
    pub repeat: usize,
    pub frames: usize,
    pub full_frames: usize,
    pub delivered_fps: f64,
    pub p50_latency_us: f64,
    pub p95_latency_us: f64,
    pub avg_decode_us: f64,
    pub avg_shared_frame_write_us: f64,
    pub avg_packet_encode_us: f64,
    pub deadline_overruns: usize,
}

/// Median across repeats of one workload's per-repeat values.
pub fn median_of(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    percentile_us(values, 50.0)
}

// ---------------------------------------------------------------------------
// Harness 1: real-server TLS/NLA smoke (`#[ignore]`)
// ---------------------------------------------------------------------------

/// Redacted per-host evidence row (design.md L453). Usernames are replaced
/// by fixed labels; passwords never have a field here at all.
#[derive(serde::Serialize, Debug)]
struct HostSmokeReport {
    host_label: String,
    host: String,
    port: u16,
    user_label: String,
    auth_variant: Option<String>,
    outcome: String,
    connect_ms: Option<f64>,
    tls_protocol: Option<String>,
    tls_cipher_suite: Option<String>,
    peer_cert_count: Option<usize>,
    negotiated_route: Option<String>,
    negotiation_lines: Vec<String>,
    /// BitmapCodecs capability fragments from connection cycle 1 (the codec
    /// observation: what the server advertised, what the client confirmed).
    codec_negotiation_lines: Vec<String>,
    /// Notable negotiation lines recorded during connection cycle 2
    /// (reconnect) — additive schema field; kept separate so cycle-1
    /// evidence (`negotiation_lines`) is never mixed with reconnect lines
    /// (debug-round root cause 2 was exactly that cross-cycle mixing).
    reconnect_negotiation_lines: Vec<String>,
    credssp_nla: Option<String>,
    desktop_size: Option<(u16, u16)>,
    io_channel_id: Option<u16>,
    user_channel_id: Option<u16>,
    cliprdr_channel_joined: Option<bool>,
    first_frame_ms: Option<f64>,
    frames_observed: Option<usize>,
    frames_window_ms: Option<f64>,
    session_errors: Option<usize>,
    session_error_first: Option<String>,
    fastpath_mouse_move: Option<String>,
    display_control_resize: Option<String>,
    graceful_shutdown: Option<String>,
    /// Server-side reaction observed during the bounded post-shutdown drain
    /// (task 4.2): whether the server closed the connection or kept
    /// streaming after the client's ShutdownRequest.
    teardown_observation: Option<String>,
    teardown_ms: Option<f64>,
    reconnect_cycle: Option<BTreeMap<String, serde_json::Value>>,
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Harness 1: real-server TLS/NLA smoke (`#[ignore]`)
// ---------------------------------------------------------------------------

/// Transcript logger: prefixes every captured `log` record with a
/// millisecond timestamp so the runner's transcript doubles as phase-level
/// connect-timing evidence (client.rs logs each protocol phase).
///
/// Every line is scrubbed of the staged secret values (plaintext AND the
/// encoded forms sspi dumps credential buffers in) before it is written
/// anywhere: ironrdp's X.224 negotiation DEBUG lines embed the username in
/// the connection cookie, so raw pass-through would leak it into evidence.
/// Credential-buffer-bearing sspi records are additionally redacted
/// WHOLESALE (secret-agnostic) — see [`is_credential_bearing_sspi_record`].
struct TranscriptLogger {
    start: std::time::Instant,
    secrets: Vec<String>,
    notable: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    /// BitmapCodecs capability fragments (codec observation, task 3.5):
    /// what each server advertises in ServerDemandActive and what the
    /// old-stack client confirms in ClientConfirmActive.
    codecs: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    file: std::sync::Mutex<std::fs::File>,
}

impl TranscriptLogger {
    fn scrub(&self, line: &str) -> String {
        scrub_secrets(line, &self.secrets)
    }
}

/// Case-insensitive replace of `needle` in `haystack` by `replacement`.
///
/// Falls back to exact-case replacement when lowercasing would change byte
/// lengths (possible for some Unicode), keeping byte indices valid.
fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() || haystack.len() < needle.len() {
        return haystack.to_string();
    }
    let hay_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    if hay_lower.len() != haystack.len() || needle_lower.len() != needle.len() {
        return haystack.replace(needle, replacement);
    }
    let is_alnum = |b: u8| b.is_ascii_alphanumeric();
    let mut out = String::with_capacity(haystack.len());
    let mut i = 0;
    while let Some(pos) = hay_lower[i..].find(&needle_lower) {
        let abs = i + pos;
        let end = abs + needle.len();
        // short secrets scrub only as standalone words — an unbounded
        // 2-char needle would mangle ordinary words ("NtlmConfig")
        if needle.len() < 4 {
            let prev_ok = abs == 0 || !is_alnum(haystack.as_bytes()[abs - 1]);
            let next_ok = end == haystack.len() || !is_alnum(haystack.as_bytes()[end]);
            if !prev_ok || !next_ok {
                out.push_str(&haystack[i..end]);
                i = end;
                if i >= haystack.len() {
                    break;
                }
                continue;
            }
        }
        out.push_str(&haystack[i..abs]);
        out.push_str(replacement);
        i = end;
    }
    out.push_str(&haystack[i..]);
    out
}

/// UTF-16LE hex encoding of `s` — the format sspi's `AuthIdentityBuffers`
/// Debug output uses for the user/domain/password buffers (each byte as two
/// hex digits, little-endian u16s).
fn utf16le_hex(s: &str) -> String {
    s.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// ASCII/UTF-8 byte-level hex encoding of `s`.
fn ascii_hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// `", "`-joined decimal UTF-16LE byte list — the format sspi's
/// `write_password_credentials` and credssp TSRequest traces use for
/// credential buffers.
fn utf16le_decimal_list(s: &str) -> String {
    s.encode_utf16()
        .flat_map(u16::to_le_bytes)
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Redact every staged secret — the full value AND every separator-split
/// component of at least 4 characters — CASE-INSENSITIVELY: sspi logs the
/// NTLM domain uppercased (`domain="ANALYSISDESK"` while the staged form is
/// lowercase), so exact-case scrubbing leaks (reviewer finding, remediation
/// round 1). Each of those plaintext forms is ALSO scrubbed in the three
/// encodings sspi dumps credential buffers in — UTF-16LE hex, ASCII hex and
/// the `", "`-joined decimal UTF-16LE byte list (debug-round finding: the
/// `AuthIdentityBuffers`/`encrypt_message` DEBUG records embed the secrets
/// in exactly these recoverable forms, which a text-only scrub passes
/// through). Encoded needles are pure ASCII and at least 8 characters, so
/// the short-needle word-boundary rule never applies and ordinary prose
/// cannot false-positive on them.
fn scrub_secrets(line: &str, secrets: &[String]) -> String {
    let mut scrubbed = line.to_string();
    for secret in secrets {
        if secret.is_empty() {
            continue;
        }
        // plaintext forms: the full value, then (sspi logs logon names
        // SPLIT at '\'/'@' separators — username="..." domain="...") every
        // component of at least 4 characters
        let mut forms = vec![secret.clone()];
        for part in secret.split(['\\', '@']) {
            if part.len() >= 4 {
                forms.push(part.to_string());
            }
        }
        for form in &forms {
            scrubbed = replace_case_insensitive(scrubbed.as_str(), form, "<redacted>");
            // encoded forms of the plaintext AND its ASCII case variants:
            // NTLM uppercases the domain inside its messages, and the
            // encoded bytes of "ANALYSISDESK" differ from the staged
            // "analysisdesk" — the as-staged needles alone cannot catch
            // them (extension of the round-1 case-insensitivity finding)
            let mut encoded_forms: Vec<String> = Vec::new();
            for variant in [form.clone(), form.to_uppercase(), form.to_lowercase()] {
                if variant.is_ascii() {
                    encoded_forms.push(utf16le_hex(&variant));
                    encoded_forms.push(ascii_hex(&variant));
                    encoded_forms.push(utf16le_decimal_list(&variant));
                }
            }
            for encoded in encoded_forms {
                if encoded.len() >= 8 {
                    scrubbed = replace_case_insensitive(&scrubbed, &encoded, "<redacted>");
                }
            }
        }
    }
    scrubbed
}

/// Credential-buffer signature of an sspi DEBUG record. sspi logs whole
/// credential buffers at Debug level: `AuthIdentityBuffers` with the
/// user/domain/password buffers as UTF-16LE hex, `SecurityBufferRef`s
/// carrying the plaintext TSCredentials (password included) inside
/// `encrypt_message`/`decrypt_message` traces, the
/// `write_password_credentials` decimal DER array, and — on
/// `sspi::credssp*` targets — full TSRequest round-trips as non-empty
/// decimal byte arrays in `return=Ok([...])`. Such records are redacted
/// wholesale because the buffers are recoverable secrets REGARDLESS of
/// which staged value they embed — this secret-agnostic control also
/// covers credential material that was never staged (e.g. a mistyped
/// password), which value-based scrubbing can never catch (debug-round
/// root cause 1).
///
/// Boundary (pinned by unit tests): negotiation/NTLM wire-level ANALYSIS
/// records keep flowing — `negotiate_protocol` lines, `tscredentials has
/// been written`, `CredSSp finished NLA stage.` and every
/// `ironrdp_connector` line (the notable negotiation markers) are not
/// credential buffers and must not be redacted.
fn is_credential_bearing_sspi_record(target: &str, args: &str) -> bool {
    if !target.starts_with("sspi") {
        return false;
    }
    if args.contains("AuthIdentityBuffers")
        || args.contains("SecurityBufferRef")
        || args.contains("write_password_credentials")
    {
        return true;
    }
    let trimmed = args.trim_start();
    if trimmed.starts_with("encrypt_message") || trimmed.starts_with("decrypt_message") {
        return true;
    }
    // credssp TSRequest traces: on credssp targets a NON-EMPTY decimal byte
    // array return value is wire payload (credential-bearing); the empty
    // array `return=Ok([])` is not.
    if target.starts_with("sspi::credssp")
        && let Some(pos) = args.find("return=Ok([")
    {
        let after = &args[pos + "return=Ok([".len()..];
        if after.starts_with(|c: char| c.is_ascii_digit()) {
            return true;
        }
    }
    false
}

/// The transcript body for one record (pure, unit-tested): wholesale
/// redaction — target and level kept, every argument byte dropped — for
/// credential-bearing sspi records; scrubbed passthrough otherwise.
fn transcript_record_line(prefix: &str, target: &str, args: &str, secrets: &[String]) -> String {
    if is_credential_bearing_sspi_record(target, args) {
        format!("{prefix} <credential-bearing sspi record redacted>")
    } else {
        scrub_secrets(&format!("{prefix} {args}"), secrets)
    }
}

/// Extract the balanced-paren `BitmapCodecs(...)` capability fragment from
/// a capability-negotiation log line (the codec advertisement evidence for
/// the codec observation). `None` when the line carries no codec list.
fn extract_bitmap_codecs_fragment(line: &str) -> Option<String> {
    let start = line.find("BitmapCodecs(")?;
    let mut depth = 0usize;
    for (i, c) in line[start..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(line[start..start + i + c.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

impl log::Log for TranscriptLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Debug
    }
    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let prefix = format!(
            "[{:>10.3}s {:<5} {}]",
            self.start.elapsed().as_secs_f64(),
            record.level().to_string(),
            record.target()
        );
        let args = record.args().to_string();
        let line = format!(
            "{}\n",
            transcript_record_line(&prefix, record.target(), &args, &self.secrets)
        );
        // Server negotiation policy evidence worth surfacing in the report
        // (selected security protocol, nego flags, joined channels, NLA
        // routing decisions inside connect_finalize).
        if (args.contains("Server confirmed connection")
            || args.contains("static_channel_ids=")
            || args.contains("Could not encode a resize")
            || args.contains("CredSSP is disabled")
            || args.contains("Begin NLA using CredSSP"))
            && let Ok(mut n) = self.notable.lock()
        {
            n.push(line.trim().to_string());
        }
        // Codec capability evidence (task 3.5 codec observation): the
        // DemandActive/ConfirmActive DEBUG records of the activation phase
        // carry the negotiated BitmapCodecs capability list.
        if record.target() == "ironrdp_connector::connection_activation"
            && let Some(fragment) = extract_bitmap_codecs_fragment(&args)
        {
            let direction = if args.starts_with("Received message=ServerDemandActive") {
                "server DemandActive"
            } else if args.starts_with("Send message=ClientConfirmActive") {
                "client ConfirmActive"
            } else {
                "capability record"
            };
            if let Ok(mut c) = self.codecs.lock() {
                c.push(format!("{direction}: {}", self.scrub(&fragment)));
            }
        }
        eprint!("{line}");
        if let Ok(mut f) = self.file.lock() {
            let _ = std::io::Write::write_all(&mut *f, line.as_bytes());
        }
    }
    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = std::io::Write::flush(&mut *f);
        }
    }
}

fn read_smoke_env() -> Vec<(&'static str, String, u16, String, String)> {
    // The staged estate is a numbered host list in the env file
    // (maintainer-defined 2026-11-12: host1 Windows 11 native RDP, host2
    // FreeRDP/Linux, host3 IronRDP/Linux). The list is DATA-DRIVEN: hosts
    // are read in label order until the first missing triple — hosts 1 and
    // 2 are mandatory (panic, as before), any further host runs simply by
    // staging RDP_SMOKE_HOSTn/USERn/PASSn, with no code edit.
    let mut hosts = Vec::new();
    for (i, n) in ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
        .iter()
        .enumerate()
    {
        let (host, user, pass) = match (
            std::env::var(format!("RDP_SMOKE_HOST{n}")),
            std::env::var(format!("RDP_SMOKE_USER{n}")),
            std::env::var(format!("RDP_SMOKE_PASS{n}")),
        ) {
            (Ok(h), Ok(u), Ok(p)) if !h.is_empty() && !u.is_empty() && !p.is_empty() => (h, u, p),
            _ if i >= 2 => break,
            _ => panic!(
                "RDP_SMOKE_HOST{n}/USER{n}/PASS{n} must be loaded into the process \
                 environment (scripts/Invoke-Baseline.ps1 -Mode Smoke) before running \
                 this explicitly-invoked smoke test"
            ),
        };
        // the staged env file may carry "host:port" or a bare host
        let (host, port) = match host.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() => (h.to_string(), p.parse::<u16>().unwrap()),
            _ => (host, 3389),
        };
        hosts.push((*n, host, port, user, pass));
    }
    hosts
}

/// Classify the negotiated security route from ONE connection cycle's
/// notable lines (pure). The per-cycle drain guarantees the bucket holds
/// only that cycle's lines: the round-3 bug was host1's cycle-2 `Begin NLA`
/// lines leaking into host2's bucket, flipping host2's classification to
/// CredSSP while the transcript proved SSL at host2's own timestamps.
/// `Begin NLA` wins over `CredSSP is disabled` when both are present (a
/// mixed bucket means the drain regressed — see the shared-bucket test).
fn classify_nla_route(cycle1_lines: &[String], tls_protocol: Option<&str>) -> (String, String) {
    let nla_ran = cycle1_lines
        .iter()
        .any(|l| l.contains("Begin NLA using CredSSP"));
    let nla_skipped = cycle1_lines
        .iter()
        .any(|l| l.contains("CredSSP is disabled"));
    let route = format!(
        "X.224 (connect_begin) -> TLS rustls {} -> {}",
        tls_protocol.unwrap_or("?"),
        if nla_ran {
            "CredSSP/NLA (connect_finalize)"
        } else if nla_skipped {
            "basic settings over TLS (server selected SSL; CredSSP skipped, no NLA)"
        } else {
            "connect_finalize (CredSSP routing not captured)"
        }
    );
    let credssp_nla = if nla_ran {
        "ran: CredSSP/NTLM exchange over TLS completed during connect_finalize".to_string()
    } else if nla_skipped {
        "skipped by server policy: X.224 response selected SSL only (no NLA); credentials carried by the Client Info PDU inside TLS".to_string()
    } else {
        "outcome not captured in transcript".to_string()
    };
    (route, credssp_nla)
}

/// Old-stack TLS/NLA smoke against both staged LAN hosts (design L453).
///
/// Gate semantics (task 3.5 done-criterion): both hosts must complete
/// X.224 -> TLS -> CredSSP/NLA finalize, produce a first graphics frame,
/// and shut down gracefully. Missing evidence fails this test and blocks
/// the protocol upgrade. The reconnect cycle is recorded but does not
/// gate (Req 12.9 lists reconnect evidence at release, not at upgrade).
#[test]
#[ignore = "real-server smoke: run explicitly via scripts/Invoke-Baseline.ps1 -Mode Smoke (task 3.5)"]
fn real_server_tls_nla_smoke() {
    // B3-15 resolved in task 4.2: the connector's TLS upgrade pins the ring
    // CryptoProvider on its own ClientConfig (client.rs do_tls_upgrade), so
    // this headless harness no longer needs to compensate with a process-wide
    // install — the connector path is standalone-reusable now.

    let out_dir = baseline_out_dir();
    std::fs::create_dir_all(&out_dir).expect("create baseline output dir");
    let transcript_path = out_dir.join("smoke-transcript.log");
    let transcript_file = std::fs::File::create(&transcript_path).expect("create transcript");
    let hosts = read_smoke_env();
    let secrets = hosts
        .iter()
        .flat_map(|(_, _, _, user, pass)| [user.clone(), pass.clone()])
        .collect::<Vec<_>>();
    let notable = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let codecs = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let _ = log::set_boxed_logger(Box::new(TranscriptLogger {
        start: std::time::Instant::now(),
        secrets,
        notable: std::sync::Arc::clone(&notable),
        codecs: std::sync::Arc::clone(&codecs),
        file: std::sync::Mutex::new(transcript_file),
    }));
    log::set_max_level(log::LevelFilter::Debug);

    eprintln!("SMOKE out_dir={}", out_dir.display());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime for smoke");

    let mut reports = Vec::new();
    for (n, host, port, user, pass) in &hosts {
        let report = rt.block_on(smoke_host(n, host, *port, user, pass, &notable, &codecs));
        reports.push(report);
    }

    let json = serde_json::to_string_pretty(&reports).expect("serialize smoke report");
    std::fs::write(out_dir.join("smoke-report.json"), &json).expect("write smoke report");
    eprintln!(
        "SMOKE report written: {}",
        out_dir.join("smoke-report.json").display()
    );

    // Gate: every host must have produced the mandatory evidence chain.
    for r in &reports {
        let label = format!("{} (<{}>)", r.host, r.host_label);
        assert_eq!(
            r.outcome,
            "connected",
            "{}: TLS/NLA smoke did not connect — upgrade gate blocked ({})",
            label,
            r.error.clone().unwrap_or_default()
        );
        assert!(r.tls_protocol.is_some(), "{label}: TLS details missing");
        assert!(
            r.credssp_nla.is_some(),
            "{label}: CredSSP/NLA outcome missing"
        );
        assert!(
            r.first_frame_ms.is_some(),
            "{label}: no first frame within budget"
        );
        assert_eq!(
            r.graceful_shutdown.as_deref(),
            Some("clean"),
            "{label}: graceful shutdown failed"
        );
    }
    eprintln!(
        "SMOKE gate: PASS ({} hosts: TLS+NLA route evidence, first frame, clean shutdown)",
        reports.len()
    );
}

async fn smoke_host(
    n: &str,
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
    notable: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    codecs: &std::sync::Arc<std::sync::Mutex<Vec<String>>>,
) -> HostSmokeReport {
    use crate::rdp::client::{ConnectionOutcome, attempt_rdp_connection};
    use crate::store::connections::ConnectionConfig;

    eprintln!("SMOKE host{n}={host}:{port} user=<user{n}> starting");
    // Per-CYCLE drain (debug-round root cause 2): the accumulators are NOT
    // drained at host start — the previous host drained them after its own
    // cycle 2 — so the take() after cycle 1 sees only THIS host's cycle-1
    // lines. (Taking here used to import the previous host's cycle-2
    // `Begin NLA` lines and flip this host's route classification.)
    let mut notable_taken: Vec<String> = Vec::new();
    let mut codec_taken: Vec<String> = Vec::new();

    let mut config = ConnectionConfig {
        id: format!("baseline-smoke-{n}"),
        name: format!("baseline-smoke-{n}"),
        host: host.to_string(),
        port,
        username: user.to_string(),
        domain: String::new(),
        color_accent: None,
        tags: vec!["baseline-smoke".to_string()],
        display_width: Some(1920),
        display_height: Some(1080),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        last_connected_at: None,
        mock: false,
    };

    let mut report = HostSmokeReport {
        host_label: format!("host{n}"),
        host: host.to_string(),
        port,
        user_label: mask_user(&format!("user{n}")),
        auth_variant: Some("as-staged".into()),
        outcome: "failed".into(),
        connect_ms: None,
        tls_protocol: None,
        tls_cipher_suite: None,
        peer_cert_count: None,
        negotiated_route: None,
        negotiation_lines: Vec::new(),
        codec_negotiation_lines: Vec::new(),
        reconnect_negotiation_lines: Vec::new(),
        credssp_nla: None,
        desktop_size: None,
        io_channel_id: None,
        user_channel_id: None,
        cliprdr_channel_joined: None,
        first_frame_ms: None,
        frames_observed: None,
        frames_window_ms: None,
        session_errors: None,
        session_error_first: None,
        fastpath_mouse_move: None,
        display_control_resize: None,
        graceful_shutdown: None,
        teardown_observation: None,
        teardown_ms: None,
        reconnect_cycle: None,
        error: None,
    };

    // --- connection cycle 1 (the gated evidence) ---------------------------
    let t0 = std::time::Instant::now();
    let mut outcome = attempt_rdp_connection(&config, Some(pass), 1920, 1080).await;
    report.connect_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);

    // Single diagnostic retry: a UPN-style staged username can fail NTLM
    // logon when the account is machine-local; retrying once with the bare
    // local part distinguishes a name-form problem from a wrong password.
    // Both attempts are recorded; no guessing beyond the staged secret.
    if let ConnectionOutcome::Failed(reason) = &outcome
        && reason.contains("CredSSP")
        && config.username.contains('@')
    {
        let local_part = config.username.split('@').next().unwrap_or("").to_string();
        if !local_part.is_empty() {
            eprintln!(
                "SMOKE host{n}: CredSSP failed with the staged UPN form; retrying once with the local-part form (diagnostic)"
            );
            config.username = local_part;
            config.domain = String::new();
            report.auth_variant = Some("local-part retry after UPN CredSSP failure".into());
            report.error = Some(reason.clone());
            let t0 = std::time::Instant::now();
            outcome = attempt_rdp_connection(&config, Some(pass), 1920, 1080).await;
            report.connect_ms = Some(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }

    let (connection_result, framed) = match outcome {
        ConnectionOutcome::RealSession {
            connection_result,
            framed,
        } => (*connection_result, *framed),
        ConnectionOutcome::Failed(reason) => {
            if report.error.is_none() {
                report.error = Some(reason.clone());
            }
            eprintln!("SMOKE host{n}: connect FAILED: {reason}");
            // no cycle 2 runs on this path — draining here leaves the
            // accumulators empty for the next host
            notable_taken.extend(std::mem::take(&mut *notable.lock().expect("notable lock")));
            codec_taken.extend(std::mem::take(&mut *codecs.lock().expect("codecs lock")));
            report.negotiation_lines = notable_taken;
            report.codec_negotiation_lines = codec_taken;
            return report;
        }
    };
    report.outcome = "connected".into();

    // Negotiated TLS details from the live rustls session (the stream is
    // still framed; get_inner() borrows the underlying TLS stream).
    {
        let (tls_stream, _leftover) = framed.get_inner();
        let (_tcp, tls_conn) = tls_stream.get_ref();
        report.tls_protocol = tls_conn
            .protocol_version()
            .map(|v| format!("{v:?}"))
            .or_else(|| Some("unknown".into()));
        report.tls_cipher_suite = tls_conn.negotiated_cipher_suite().map(|c| format!("{c:?}"));
        report.peer_cert_count = tls_conn.peer_certificates().map(|certs| certs.len());
    }

    report.desktop_size = Some((
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    ));
    report.io_channel_id = Some(connection_result.io_channel_id);
    report.user_channel_id = Some(connection_result.user_channel_id);
    // Server policy evidence available from negotiation: the cliprdr static
    // channel (attached by attempt_rdp_connection) was joined by the server.
    report.cliprdr_channel_joined = Some(
        connection_result
            .static_channels
            .get_channel_id_by_type::<ironrdp_cliprdr::CliprdrClient>()
            .is_some(),
    );
    eprintln!(
        "SMOKE host{n}: connected in {:.0} ms, desktop {}x{}, cliprdr_joined={}",
        report.connect_ms.unwrap_or(0.0),
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
        report.cliprdr_channel_joined.unwrap_or(false),
    );

    // --- session-layer loop: first frame, input, resize, shutdown ----------
    session_layer_checks(&mut report, connection_result, framed).await;

    // cycle-1 evidence: drain BOTH accumulators now — the buckets hold only
    // this cycle's lines (per-cycle drain) — and classify from the pure fn
    notable_taken.extend(std::mem::take(&mut *notable.lock().expect("notable lock")));
    codec_taken.extend(std::mem::take(&mut *codecs.lock().expect("codecs lock")));
    report.negotiation_lines = notable_taken;
    report.codec_negotiation_lines = codec_taken;
    let (route, credssp_nla) =
        classify_nla_route(&report.negotiation_lines, report.tls_protocol.as_deref());
    report.negotiated_route = Some(route);
    report.credssp_nla = Some(credssp_nla);

    // --- connection cycle 2 (reconnect viability, non-gating) --------------
    let t1 = std::time::Instant::now();
    let second = attempt_rdp_connection(&config, Some(pass), 1920, 1080).await;
    let mut cycle2 = BTreeMap::new();
    match second {
        ConnectionOutcome::RealSession {
            connection_result: cr2,
            framed: f2,
        } => {
            cycle2.insert(
                "outcome".into(),
                serde_json::json!("connected (reconnect viable at connector level)"),
            );
            cycle2.insert(
                "connect_ms".into(),
                serde_json::json!(t1.elapsed().as_secs_f64() * 1000.0),
            );
            cycle2.insert(
                "desktop_size".into(),
                serde_json::json!(format!(
                    "{}x{}",
                    cr2.desktop_size.width, cr2.desktop_size.height
                )),
            );
            // immediately release the second session
            use ironrdp_session::ActiveStageOutput;
            use ironrdp_tokio::FramedWrite as _;
            let stage = crate::rdp::client::active_stage_from_connection_result(*cr2);
            let (mut _r, mut w) = ironrdp_tokio::split_tokio_framed(*f2);
            if let Ok(outputs) = stage.graceful_shutdown() {
                for out in outputs {
                    if let ActiveStageOutput::ResponseFrame(frame) = out {
                        let _ = w.write_all(&frame).await;
                    }
                }
            }
            cycle2.insert(
                "shutdown".into(),
                serde_json::json!("graceful_shutdown sent"),
            );
        }
        ConnectionOutcome::Failed(reason) => {
            cycle2.insert(
                "outcome".into(),
                serde_json::json!(format!("failed: {reason}")),
            );
        }
    }
    report.reconnect_cycle = Some(cycle2);
    // cycle-2 evidence: the reconnect's notable lines go to their OWN field
    // (never mixed into cycle-1 classification); reconnect codec fragments
    // negotiate identically to cycle 1 and are dropped. Both accumulators
    // are empty on return, so the next host starts clean.
    report.reconnect_negotiation_lines =
        std::mem::take(&mut *notable.lock().expect("notable lock"));
    let _reconnect_codec_lines = std::mem::take(&mut *codecs.lock().expect("codecs lock"));
    eprintln!("SMOKE host{n}: reconnect cycle recorded");
    report
}

async fn session_layer_checks(
    report: &mut HostSmokeReport,
    connection_result: ironrdp_connector::ConnectionResult,
    framed: crate::rdp::client::TlsFramed,
) {
    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_session::ActiveStageOutput;
    use ironrdp_session::image::DecodedImage;
    use ironrdp_tokio::FramedWrite as _;

    // Task 4.3 harness fix (found behavior): size the decode image from the
    // NEGOTIATED desktop, mirroring production session.rs (which sizes its
    // DecodedImage from connection_result.desktop_size). The harness
    // previously hardcoded 1920x1080 — silently matching the requested size
    // on hosts that honor it. Host3 (IronRDP server) negotiates 3064x1264
    // regardless of the request, and the 0.13 fast-path processor drops
    // every bitmap update whose declared destination exceeds the image
    // ("Ignoring bitmap update with an invalid declared destination"), so
    // the old harness observed 0–1 frames on host3 (the 4.2 runs passed on
    // a single in-bounds update at 12.3 s/30.5 s — knife's edge, run
    // smoke-20261112-182323 then measured 0 and failed the gate).
    let (desktop_width, desktop_height) = (
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );
    let mut stage = crate::rdp::client::active_stage_from_connection_result(connection_result);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, desktop_width, desktop_height);
    let (mut reader, mut writer) = ironrdp_tokio::split_tokio_framed(framed);

    let loop_start = std::time::Instant::now();
    let connect_start = loop_start
        .checked_sub(std::time::Duration::from_millis(
            report.connect_ms.unwrap_or(0.0) as u64,
        ))
        .unwrap_or(loop_start);
    let mut frames = 0usize;
    let mut first_frame_ms: Option<f64> = None;
    const FIRST_FRAME_BUDGET: Duration = Duration::from_secs(30);
    const FRAMES_WINDOW: Duration = Duration::from_secs(5);

    loop {
        if loop_start.elapsed() > FIRST_FRAME_BUDGET + FRAMES_WINDOW {
            break;
        }
        let pdu = match tokio::time::timeout(Duration::from_secs(10), reader.read_pdu()).await {
            Ok(Ok(pdu)) => pdu,
            Ok(Err(e)) => {
                eprintln!("SMOKE read_pdu error after {frames} frames: {e}");
                break;
            }
            Err(_) => {
                eprintln!("SMOKE read_pdu timed out after {frames} frames");
                break;
            }
        };
        // `received_at: None` preserves the pre-0.13 behavior (the timestamp
        // only feeds upstream's new bandwidth auto-detect measurement).
        let outputs = match stage.process(&mut image, pdu.0, &pdu.1, None) {
            Ok(outputs) => outputs,
            Err(e) => {
                // Production diverges here: the session loop treats a
                // process error as fatal (session.rs L627-636 sets Error and
                // returns ConnectionLost). The smoke continues reading to
                // characterize how often the old stack chokes on this
                // server's PDUs — recorded as session_errors.
                report.session_errors = Some(report.session_errors.unwrap_or(0) + 1);
                if report.session_error_first.is_none() {
                    report.session_error_first = Some(format!("{e}"));
                    eprintln!("SMOKE active_stage process error (continuing for evidence): {e}");
                }
                continue;
            }
        };
        let mut got_graphics = false;
        for out in outputs {
            match out {
                ActiveStageOutput::GraphicsUpdate(_) => {
                    got_graphics = true;
                }
                ActiveStageOutput::ResponseFrame(frame) if !frame.is_empty() => {
                    let _ = writer.write_all(&frame).await;
                }
                _ => {}
            }
        }
        if got_graphics {
            frames += 1;
            if first_frame_ms.is_none() {
                first_frame_ms = Some(loop_start.elapsed().as_secs_f64() * 1000.0);
                report.first_frame_ms = Some(connect_start.elapsed().as_secs_f64() * 1000.0);
                eprintln!(
                    "SMOKE first frame: {:.0} ms after connect start ({frames} window frames)",
                    report.first_frame_ms.unwrap_or(0.0)
                );

                // FastPath input on the wire: a pointer move is the least
                // intrusive real input event. Coordinates (16,16) are safe.
                use ironrdp_pdu::input::fast_path::FastPathInputEvent;
                use ironrdp_pdu::input::mouse::{MousePdu, PointerFlags};
                let event = FastPathInputEvent::MouseEvent(MousePdu {
                    flags: PointerFlags::MOVE,
                    number_of_wheel_rotation_units: 0,
                    x_position: 16,
                    y_position: 16,
                });
                match stage.process_fastpath_input(&mut image, &[event]) {
                    Ok(outs) => {
                        let mut sent = false;
                        for out in outs {
                            if let ActiveStageOutput::ResponseFrame(frame) = out
                                && !frame.is_empty()
                            {
                                sent = writer.write_all(&frame).await.is_ok();
                            }
                        }
                        report.fastpath_mouse_move = Some(if sent {
                            "sent-ok".into()
                        } else {
                            "encoded-no-response".into()
                        });
                    }
                    Err(e) => report.fastpath_mouse_move = Some(format!("error: {e}")),
                }

                // Display-control resize probe: the connector never attaches
                // the DisplayControlClient DVC (session-layer work), so
                // encode_resize is expected to return None — recorded as
                // found behavior.
                report.display_control_resize =
                    Some(match stage.encode_resize(1280, 720, None, None) {
                        None => "None (display-control DVC not negotiated by the connector)".into(),
                        Some(Ok(frame)) => {
                            let ok = writer.write_all(&frame).await.is_ok();
                            format!("encoded+sent (write ok: {ok})")
                        }
                        Some(Err(e)) => format!("encode error: {e}"),
                    });
            }
            if loop_start.elapsed() >= FRAMES_WINDOW || frames >= 150 {
                break;
            }
        }
    }
    report.frames_observed = Some(frames);
    report.frames_window_ms = Some(loop_start.elapsed().as_secs_f64() * 1000.0);
    eprintln!(
        "SMOKE frames window: {frames} graphics updates in {:.0} ms",
        report.frames_window_ms.unwrap_or(0.0)
    );

    // Graceful teardown through the production path.
    let t_teardown = std::time::Instant::now();
    let mut clean = false;
    if let Ok(outputs) = stage.graceful_shutdown() {
        for out in outputs {
            if let ActiveStageOutput::ResponseFrame(frame) = out
                && !frame.is_empty()
            {
                let _ = writer.write_all(&frame).await;
            }
        }
        clean = true;
    }
    // Drain briefly so the shutdown PDU is flushed and the server's reaction
    // (close or continued streaming) is observed. Bounded at 10s total (task
    // 4.2): the 0.12 connector negotiates the MCS message channel and network
    // characteristics auto-detection, after which servers keep sending
    // auto-detect/graphics traffic indefinitely — an unbounded
    // "drain until quiet" never terminates on the new stack (found by this
    // smoke). The bound covers the old stack's worst observed teardown window
    // (~8.2s), so observation semantics are unchanged for quiet servers; the
    // observation itself is recorded in `teardown_observation`.
    let drain_deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut drained = 0usize;
    let mut server_closed = false;
    loop {
        if std::time::Instant::now() >= drain_deadline {
            break;
        }
        match tokio::time::timeout(Duration::from_secs(2), reader.read_pdu()).await {
            Ok(Ok(_pdu)) => drained += 1,
            Ok(Err(_)) => {
                server_closed = true;
                break;
            }
            Err(_) => break, // quiet: >2s without a PDU
        }
    }
    report.teardown_observation = Some(if server_closed {
        format!("server closed the connection after the shutdown request ({drained} pdus drained)")
    } else {
        format!(
            "server kept streaming after the shutdown request; drain bounded at 10s ({drained} pdus drained)"
        )
    });
    report.graceful_shutdown = Some(if clean {
        "clean".into()
    } else {
        "graceful_shutdown errored".into()
    });
    report.teardown_ms = Some(t_teardown.elapsed().as_secs_f64() * 1000.0);
    eprintln!(
        "SMOKE teardown: {} in {:.0} ms ({})",
        report.graceful_shutdown.as_deref().unwrap_or("?"),
        report.teardown_ms.unwrap_or(0.0),
        report.teardown_observation.as_deref().unwrap_or("?")
    );
}

// ---------------------------------------------------------------------------
// Harness 2: design workload-matrix benchmark baseline (`#[ignore]`)
// ---------------------------------------------------------------------------

struct SessionBenchResult {
    latencies_us: Vec<f64>,
    decode_us_total: f64,
    shared_frame_us_total: f64,
    packet_encode_us_total: f64,
    deadline_overruns: usize,
    frames: usize,
    full_frames: usize,
}

fn run_session_pipeline(
    width: u16,
    height: u16,
    warmup: Duration,
    sample: Duration,
    frame_offset: &std::sync::atomic::AtomicUsize,
) -> SessionBenchResult {
    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_pdu::Action;
    use ironrdp_session::ActiveStageOutput;
    use ironrdp_session::image::DecodedImage;

    let mut image = DecodedImage::new(PixelFormat::RgbA32, width, height);
    let mut stage = crate::rdp::client::active_stage_from_connection_result(
        synthetic_connection_result(width, height),
    );
    let shared = crate::renderer::shared_frame::SharedFrame::new(width as u32, height as u32);
    let mut scratch: Vec<u8> = Vec::new();

    let rect_w = TEXT_RECT_WIDTH as usize;
    let rect_h = TEXT_RECT_HEIGHT as usize;
    let mut text_rows = vec![0u8; rect_w * rect_h * 2];
    let mut full_rows = vec![0u8; width as usize * height as usize * 2];
    fill_full_frame_rows_16bpp(&mut full_rows, width as usize, height as usize);

    let period = Duration::from_nanos(1_000_000_000 / UPDATES_PER_SECOND as u64);
    let mut result = SessionBenchResult {
        latencies_us: Vec::new(),
        decode_us_total: 0.0,
        shared_frame_us_total: 0.0,
        packet_encode_us_total: 0.0,
        deadline_overruns: 0,
        frames: 0,
        full_frames: 0,
    };

    let sample_end = std::time::Instant::now() + warmup + sample;
    let warmup_end = std::time::Instant::now() + warmup;
    let mut next_deadline = std::time::Instant::now();
    let mut frame_idx = frame_offset.load(std::sync::atomic::Ordering::Relaxed);

    loop {
        let now = std::time::Instant::now();
        if now >= sample_end {
            break;
        }
        let in_sample = now >= warmup_end;

        // Fixture generation happens BEFORE the measured window (the server
        // produces update bytes before the client decodes them).
        let is_full = frame_is_full_frame(frame_idx);
        let fragments: Vec<Vec<u8>> = if is_full {
            let bands = full_frame_band_rects(width, height);
            let slices: Vec<&[u8]> = bands
                .iter()
                .map(|band| full_frame_band_rows(&full_rows, height, band))
                .collect();
            encode_bitmap_update_pdu_16bpp(&bands, &slices)
        } else {
            let rects = text_scroll_rects(frame_idx, width, height);
            fill_text_rect_rows_16bpp(&mut text_rows, frame_idx, 0, rect_w, rect_h);
            let slices: Vec<&[u8]> = rects.iter().map(|_| text_rows.as_slice()).collect();
            encode_bitmap_update_pdu_16bpp(&rects, &slices)
        };

        // --- measured: decode-ready -> packet-emitted (in-process prefix
        // --- of the design's decode-ready-to-canvas-consumed metric)
        let t0 = std::time::Instant::now();
        let mut updated_regions: Vec<InclusiveRectangle> = Vec::new();
        for fragment in &fragments {
            let outputs = stage
                .process(&mut image, Action::FastPath, fragment, None)
                .expect("fixture fragment decodes");
            for out in outputs {
                match out {
                    ActiveStageOutput::GraphicsUpdate(region) => updated_regions.push(region),
                    ActiveStageOutput::ResponseFrame(_) => {
                        // bitmap fixtures produce no response frames; if the
                        // fixture ever triggers one it must be counted, not
                        // silently dropped.
                    }
                    _ => {}
                }
            }
        }
        let t1 = std::time::Instant::now();
        if !updated_regions.is_empty() {
            // SharedFrame write mirrors session.rs L680-L701 exactly.
            let stride = image.stride();
            let img_data = image.data();
            let mut guard = shared.begin_write();
            for rect in &updated_regions {
                let x_offset = usize::from(rect.left) * 4;
                let y_start = usize::from(rect.top);
                let row_start = y_start * stride + x_offset;
                if row_start < img_data.len() {
                    guard.update_rect(
                        rect.left,
                        rect.top,
                        rect.width(),
                        rect.height(),
                        &img_data[row_start..],
                        stride,
                    );
                }
            }
            drop(guard);
            shared.mark_dirty();
        }
        let t2 = std::time::Instant::now();
        let packet = crate::rdp::frame_transport::encode_image_update_packet(
            &image,
            &updated_regions,
            &mut scratch,
        );
        let t3 = std::time::Instant::now();
        let _ = packet.len();

        result.frames += 1;
        result.full_frames += usize::from(is_full);
        if in_sample {
            let decode = (t1 - t0).as_secs_f64() * 1e6;
            let shared_write = (t2 - t1).as_secs_f64() * 1e6;
            let encode = (t3 - t2).as_secs_f64() * 1e6;
            result.latencies_us.push(decode + shared_write + encode);
            result.decode_us_total += decode;
            result.shared_frame_us_total += shared_write;
            result.packet_encode_us_total += encode;
        }

        // --- 30 Hz pacing (measurement code: real wall-clock by design)
        next_deadline += period;
        let now = std::time::Instant::now();
        if now > next_deadline {
            result.deadline_overruns += 1;
        } else {
            let remaining = next_deadline - now;
            if remaining > Duration::from_millis(2) {
                std::thread::sleep(remaining - Duration::from_millis(2));
            }
            while std::time::Instant::now() < next_deadline {
                std::hint::spin_loop();
            }
        }
        frame_idx += 1;
    }
    frame_offset.store(frame_idx, std::sync::atomic::Ordering::Relaxed);
    result
}

/// Design L481 workload matrix on the old stack (see module docs). Run via
/// `scripts/Invoke-Baseline.ps1 -Mode Workload`, which also samples process
/// CPU/memory; set `DRIFT_BENCH_QUICK=1` for a fast harness self-check.
#[test]
#[ignore = "perf baseline (~12 min wall at design durations): run explicitly via scripts/Invoke-Baseline.ps1 -Mode Workload (task 3.5)"]
fn workload_matrix_baseline() {
    let (warmup, sample, repeats) = bench_durations();
    let out_dir = baseline_out_dir();
    std::fs::create_dir_all(&out_dir).expect("create baseline output dir");
    eprintln!(
        "BENCH pid={} out_dir={}",
        std::process::id(),
        out_dir.display()
    );
    eprintln!(
        "BENCH config warmup_s={} sample_s={} repeats={} route=uncompressed-16bpp-fastpath-bitmap (headless old-stack pipeline; see module docs)",
        warmup.as_secs(),
        sample.as_secs(),
        repeats
    );

    let build_id = std::env::var("DRIFT_BENCH_BUILD_ID").unwrap_or_else(|_| "unset".into());
    let mut all_stats: Vec<WorkloadStat> = Vec::new();
    let epoch_ms = |t: std::time::SystemTime| {
        t.duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    };

    for (name, w, h, sessions) in WORKLOAD_MATRIX {
        for repeat in 1..=repeats {
            eprintln!(
                "BENCH window=start workload={name} repeat={repeat} phase=warmup epoch_ms={}",
                epoch_ms(std::time::SystemTime::now())
            );
            let frame_offset = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let handles: Vec<_> = (0..*sessions)
                .map(|_| {
                    std::thread::spawn({
                        let (w, h, warmup, sample) = (*w, *h, warmup, sample);
                        let offset = std::sync::Arc::clone(&frame_offset);
                        move || run_session_pipeline(w, h, warmup, sample, &offset)
                    })
                })
                .collect();
            let results: Vec<SessionBenchResult> = handles
                .into_iter()
                .map(|h| h.join().expect("session pipeline thread"))
                .collect();
            eprintln!(
                "BENCH window=end workload={name} repeat={repeat} phase=sample epoch_ms={}",
                epoch_ms(std::time::SystemTime::now())
            );

            let mut latencies: Vec<f64> = results
                .iter()
                .flat_map(|r| r.latencies_us.iter().copied())
                .collect();
            latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let frames: usize = results.iter().map(|r| r.frames).sum();
            let full_frames: usize = results.iter().map(|r| r.full_frames).sum();
            let overruns: usize = results.iter().map(|r| r.deadline_overruns).sum();
            let sampled: usize = results.iter().map(|r| r.latencies_us.len()).sum();
            let decode_total: f64 = results.iter().map(|r| r.decode_us_total).sum();
            let shared_total: f64 = results.iter().map(|r| r.shared_frame_us_total).sum();
            let encode_total: f64 = results.iter().map(|r| r.packet_encode_us_total).sum();

            let stat = WorkloadStat {
                workload: (*name).to_string(),
                repeat,
                frames,
                full_frames,
                delivered_fps: frames as f64 / (warmup.as_secs_f64() + sample.as_secs_f64()),
                p50_latency_us: percentile_us(&latencies, 50.0),
                p95_latency_us: percentile_us(&latencies, 95.0),
                avg_decode_us: if sampled > 0 {
                    decode_total / sampled as f64
                } else {
                    0.0
                },
                avg_shared_frame_write_us: if sampled > 0 {
                    shared_total / sampled as f64
                } else {
                    0.0
                },
                avg_packet_encode_us: if sampled > 0 {
                    encode_total / sampled as f64
                } else {
                    0.0
                },
                deadline_overruns: overruns,
            };
            eprintln!(
                "BENCH result workload={name} repeat={repeat} frames={frames} full_frames={full_frames} \
                 delivered_fps={:.1} p50_us={:.1} p95_us={:.1} avg_decode_us={:.1} \
                 avg_shared_frame_write_us={:.1} avg_packet_encode_us={:.1} deadline_overruns={overruns}",
                stat.delivered_fps,
                stat.p50_latency_us,
                stat.p95_latency_us,
                stat.avg_decode_us,
                stat.avg_shared_frame_write_us,
                stat.avg_packet_encode_us,
            );
            all_stats.push(stat);
        }
    }

    // medians across repeats per workload
    let mut medians: Vec<serde_json::Value> = Vec::new();
    for (name, _w, _h, _sessions) in WORKLOAD_MATRIX {
        let rows: Vec<&WorkloadStat> = all_stats.iter().filter(|s| s.workload == *name).collect();
        let pick = |get: fn(&WorkloadStat) -> f64| -> f64 {
            let mut vals: Vec<f64> = rows.iter().map(|s| get(s)).collect();
            median_of(&mut vals)
        };
        medians.push(serde_json::json!({
            "workload": name,
            "median_p50_latency_us": pick(|s| s.p50_latency_us),
            "median_p95_latency_us": pick(|s| s.p95_latency_us),
            "median_delivered_fps": pick(|s| s.delivered_fps),
            "median_avg_decode_us": pick(|s| s.avg_decode_us),
            "median_avg_shared_frame_write_us": pick(|s| s.avg_shared_frame_write_us),
            "median_avg_packet_encode_us": pick(|s| s.avg_packet_encode_us),
            "total_deadline_overruns": rows.iter().map(|s| s.deadline_overruns).sum::<usize>(),
        }));
    }

    let report = serde_json::json!({
        "build_id": build_id,
        "profile": if cfg!(debug_assertions) { "dev" } else { "release" },
        "fixture": "text-scroll: 32 dirty 64x32 rects @30 Hz + 1 full-frame/s (design.md L481)",
        "route": "uncompressed 16-bpp FastPath TS_UPDATE_BITMAP (headless old-stack decode->SharedFrame->IPC packet prefix; webview canvas suffix is a recorded gap)",
        "durations": { "warmup_s": warmup.as_secs(), "sample_s": sample.as_secs(), "repeats": repeats },
        "sessions_note": "10x1920x1080 runs 10 identical pipelines headlessly (no visible/background tab distinction without the UI)",
        "queue_high_water_mark_note": "not measurable on the old stack: frame delivery is unbounded fire-and-forget via tauri ipc::Channel; no bounded queue exists yet",
        "per_repeat": all_stats,
        "medians": medians,
    });
    let path = out_dir.join("bench-report.json");
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&report).expect("serialize bench report"),
    )
    .expect("write bench report");
    eprintln!("BENCH report written: {}", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_pdu::Action;
    use ironrdp_session::ActiveStageOutput;
    use ironrdp_session::image::DecodedImage;

    // --- fixture geometry -------------------------------------------------

    #[test]
    fn text_scroll_rects_shape_matches_design_workload() {
        let rects = text_scroll_rects(0, 1920, 1080);
        assert_eq!(rects.len(), TEXT_RECTS_PER_FRAME, "exactly 32 dirty rects");
        for (idx, r) in rects.iter().enumerate() {
            assert_eq!(r.width(), TEXT_RECT_WIDTH, "rect {idx} width");
            assert_eq!(r.height(), TEXT_RECT_HEIGHT, "rect {idx} height");
            assert!(r.right < 1920 && r.bottom < 1080, "rect {idx} in bounds");
        }
        let union = union_rects(&rects);
        assert_eq!(union.width(), 1024, "band is 16 columns wide");
        assert_eq!(union.height(), 64, "band is 2 tile rows tall");
    }

    #[test]
    fn text_scroll_rects_scroll_deterministically_and_wrap() {
        let f0 = text_scroll_rects(0, 1920, 1080);
        let f1 = text_scroll_rects(1, 1920, 1080);
        assert_eq!(f0[0].top, 0);
        assert_eq!(
            f1[0].top, TEXT_RECT_HEIGHT,
            "band moves down one tile per frame"
        );
        assert_eq!(f0, text_scroll_rects(0, 1920, 1080), "deterministic");
        // scroll span = 1080-64 = 1016 px; frames advance 32 px: frame 31 is
        // the last fully-visible band position, frame 32 wraps past the edge.
        assert_eq!(text_scroll_rects(31, 1920, 1080)[0].top, 992);
        assert_eq!(text_scroll_rects(32, 1920, 1080)[0].top, 8, "band wraps");
        // full period: 127 frames * 32 px = 4064 = 4 * 1016 -> identical layout
        assert_eq!(f0, text_scroll_rects(127, 1920, 1080));
        // stays inside a 4K surface too
        let uhd = text_scroll_rects(999, 3840, 2160);
        assert!(uhd.iter().all(|r| r.bottom < 2160));
    }

    #[test]
    fn full_frame_rect_covers_surface_exactly() {
        let r = full_frame_rect(1920, 1080);
        assert_eq!((r.left, r.top, r.right, r.bottom), (0, 0, 1919, 1079));
    }

    #[test]
    fn full_frame_bands_tile_the_surface_within_the_u16_bitmap_budget() {
        for (w, h) in [(1920u16, 1080u16), (3840, 2160)] {
            let bands = full_frame_band_rects(w, h);
            assert_eq!(
                union_rects(&bands),
                full_frame_rect(w, h),
                "union covers the surface"
            );
            // contiguous, non-overlapping strips starting at 0
            let mut expected_top = 0u32;
            for band in &bands {
                assert_eq!(
                    band.top as u32, expected_top,
                    "band starts where the previous ended"
                );
                assert_eq!(band.left, 0);
                assert_eq!(band.right, w - 1);
                let bytes = band.width() as usize * band.height() as usize * 2;
                assert!(
                    bytes <= BITMAP_DATA_MAX_BYTES,
                    "band of {bytes} bytes exceeds the TS_BITMAP_DATA u16 budget"
                );
                expected_top += band.height() as u32;
            }
            assert_eq!(expected_top, h as u32, "bands cover every row exactly once");
        }
        assert_eq!(full_frame_band_height(1920), 17, "1920px band height");
        assert_eq!(full_frame_band_height(3840), 8, "3840px band height");
    }

    #[test]
    fn full_frame_band_rows_slice_the_bottom_up_buffer_correctly() {
        let (w, h) = (64u16, 40u16);
        let mut full_rows = vec![0u8; w as usize * h as usize * 2];
        fill_full_frame_rows_16bpp(&mut full_rows, w as usize, h as usize);
        let bands = full_frame_band_rects(w, h);
        // surface rows y map to full_rows chunk (h-1-y); the first band's
        // chunk 0 must be surface row (band.bottom), i.e. chunk 0 of the
        // whole buffer when band.bottom == h-1.
        let first = &bands[0];
        let rows = full_frame_band_rows(&full_rows, h, first);
        assert_eq!(
            &rows[0..2],
            &fixture_pixel_565(usize::MAX, usize::MAX, 0, first.bottom as usize).to_le_bytes(),
            "band chunk 0 is the band's bottom row"
        );
        // total sliced bytes across bands == whole buffer
        let total: usize = bands
            .iter()
            .map(|b| full_frame_band_rows(&full_rows, h, b).len())
            .sum();
        assert_eq!(total, full_rows.len());
    }

    // --- fixture content --------------------------------------------------

    #[test]
    fn fixture_pixel_is_deterministic_and_varies() {
        assert_eq!(
            fixture_pixel_565(7, 3, 10, 20),
            fixture_pixel_565(7, 3, 10, 20)
        );
        assert_ne!(
            fixture_pixel_565(7, 3, 10, 20),
            fixture_pixel_565(8, 3, 10, 20)
        );
        assert_ne!(
            fixture_pixel_565(7, 3, 10, 20),
            fixture_pixel_565(7, 4, 10, 20)
        );
    }

    #[test]
    fn text_rect_rows_fill_exact_bottom_up_layout() {
        let w = TEXT_RECT_WIDTH as usize;
        let h = TEXT_RECT_HEIGHT as usize;
        let mut rows = vec![0u8; w * h * 2];
        fill_text_rect_rows_16bpp(&mut rows, 5, 0, w, h);
        // chunk row 0 is the BOTTOM source row (y = h-1)
        let expected_first_px = fixture_pixel_565(5, 0, 0, h - 1);
        assert_eq!(&rows[0..2], &expected_first_px.to_le_bytes());
        // last chunk row is the TOP source row (y = 0) at x = 0
        let last_row_off = (h - 1) * w * 2;
        let expected_last_px = fixture_pixel_565(5, 0, 0, 0);
        assert_eq!(
            &rows[last_row_off..last_row_off + 2],
            &expected_last_px.to_le_bytes()
        );
    }

    // --- PDU encoding + real ActiveStage roundtrip ------------------------

    #[test]
    fn encoded_pdu_decodes_through_real_active_stage_to_expected_union_region() {
        let rects = text_scroll_rects(3, 1920, 1080);
        let w = TEXT_RECT_WIDTH as usize;
        let h = TEXT_RECT_HEIGHT as usize;
        let mut rows = vec![0u8; w * h * 2];
        fill_text_rect_rows_16bpp(&mut rows, 3, 0, w, h);
        // all rects share deterministic per-rect content; rect 0 rows suffice
        // for region-shape verification (content verified separately below).
        let row_slices: Vec<&[u8]> = rects.iter().map(|_| rows.as_slice()).collect();
        let fragments = encode_bitmap_update_pdu_16bpp(&rects, &row_slices);
        assert!(
            fragments.len() > 1,
            "a 32-rect uncompressed update (~131 KiB) must fragment past the 16-bit FP length limit"
        );

        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1920, 1080);
        let mut stage = crate::rdp::client::active_stage_from_connection_result(
            synthetic_connection_result(1920, 1080),
        );
        let mut graphics = Vec::new();
        for fragment in &fragments {
            let outputs = stage
                .process(&mut image, Action::FastPath, fragment, None)
                .expect("fixture fragment decodes through the real ActiveStage");
            for out in outputs {
                if let ActiveStageOutput::GraphicsUpdate(r) = out {
                    graphics.push(r);
                }
            }
        }

        let expected_union = union_rects(&rects);
        assert_eq!(
            graphics,
            vec![expected_union],
            "one coalesced union region after reassembly"
        );
    }

    #[test]
    fn decoded_pixels_match_the_565_conversion_and_content_changes_per_frame() {
        // one 64x32 rect, decode frame 4 and frame 5, verify a known pixel
        // against ironrdp's own rdp_16bit_to_rgb formula and that frames differ.
        let w = TEXT_RECT_WIDTH as usize;
        let h = TEXT_RECT_HEIGHT as usize;
        let decode_pixel_at = |frame: usize, x: usize, y: usize| {
            let rect = InclusiveRectangle {
                left: 64,
                top: 32,
                right: 64 + w as u16 - 1,
                bottom: 32 + h as u16 - 1,
            };
            let mut rows = vec![0u8; w * h * 2];
            fill_text_rect_rows_16bpp(&mut rows, frame, 0, w, h);
            let fragments =
                encode_bitmap_update_pdu_16bpp(std::slice::from_ref(&rect), &[rows.as_slice()]);
            assert_eq!(fragments.len(), 1, "single small rect fits one fragment");
            let mut image = DecodedImage::new(PixelFormat::RgbA32, 1920, 1080);
            let mut stage = crate::rdp::client::active_stage_from_connection_result(
                synthetic_connection_result(1920, 1080),
            );
            stage
                .process(&mut image, Action::FastPath, &fragments[0], None)
                .expect("decodes");
            let stride = image.stride();
            let off = ((32 + y) * stride) + (64 + x) * 4;
            let px = &image.data()[off..off + 4];
            (px[0], px[1], px[2], px[3])
        };

        let src = fixture_pixel_565(4, 0, 7, 11);
        let [r, g, b] = {
            let r = (((((src >> 11) & 0x1f) as u32 * 527) + 23) >> 6) as u8;
            let g = (((((src >> 5) & 0x3f) as u32 * 259) + 33) >> 6) as u8;
            let bl = ((((src & 0x1f) as u32 * 527) + 23) >> 6) as u8;
            [r, g, bl]
        };
        assert_eq!(decode_pixel_at(4, 7, 11), (r, g, b, 0xff));
        assert_ne!(decode_pixel_at(4, 7, 11), decode_pixel_at(5, 7, 11));
    }

    // --- schedule, stats, redaction ---------------------------------------

    #[test]
    fn frame_schedule_marks_exactly_one_full_frame_per_second() {
        assert!(frame_is_full_frame(0));
        assert!(frame_is_full_frame(30));
        assert!(frame_is_full_frame(60));
        assert!(!frame_is_full_frame(1));
        assert!(!frame_is_full_frame(29));
        assert!(!frame_is_full_frame(31));
    }

    #[test]
    fn percentile_helper_matches_nearest_rank_semantics() {
        let mut v = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(percentile_us(&v, 50.0), 3.0);
        assert_eq!(percentile_us(&v, 0.0), 1.0);
        assert_eq!(percentile_us(&v, 100.0), 5.0);
        assert_eq!(percentile_us(&[], 50.0), 0.0);
    }

    #[test]
    fn median_of_is_nearest_rank_and_order_independent() {
        assert_eq!(median_of(&mut [5.0, 1.0, 3.0]), 3.0);
        assert_eq!(
            median_of(&mut [4.0, 1.0, 2.0, 3.0]),
            3.0,
            "nearest-rank p50"
        );
        assert_eq!(median_of(&mut []), 0.0);
    }

    #[test]
    fn mask_user_never_echoes_input_content() {
        // The evidence label is the ONLY thing that reaches artifacts; the
        // username value from the environment never becomes a function
        // argument in harness code paths that write evidence.
        assert_eq!(mask_user("user1"), "<user1>");
        assert_eq!(mask_user("user2"), "<user2>");
        let adversarial = "leak-me-PASSW0RD";
        assert!(!mask_user("user1").contains(adversarial));
    }

    #[test]
    fn scrub_is_case_insensitive_for_values_and_components() {
        // sspi logs the NTLM domain UPPERCASED regardless of the staged
        // form's casing — the exact-case scrub leaked it (reviewer finding).
        let secrets = vec!["analysisdesk\\mariu".to_string(), "s3cret-pw".to_string()];
        let line = r#"negotiate_protocol; username="mariu" domain="ANALYSISDESK" protocol="NTLM""#;
        let scrubbed = scrub_secrets(line, &secrets);
        assert!(
            !scrubbed.to_lowercase().contains("analysisdesk"),
            "uppercase domain leaked: {scrubbed}"
        );
        assert!(
            !scrubbed.contains("mariu"),
            "username component leaked: {scrubbed}"
        );
        assert!(scrubbed.contains("domain=\"<redacted>\""));

        // mixed and original casing of both components, plus the full value
        for variant in [
            "analysisdesk\\mariu",
            "ANALYSISDESK\\mariu",
            "AnalysisDesk\\MARIU",
        ] {
            let out = scrub_secrets(&format!("cookie Cookie(\"{variant}\")"), &secrets);
            assert!(
                !out.to_lowercase().contains("analysisdesk"),
                "variant {variant} leaked: {out}"
            );
            assert!(
                !out.to_lowercase().contains("mariu"),
                "variant {variant} leaked: {out}"
            );
        }

        // password value, any casing
        let out = scrub_secrets("auth password=S3CRET-PW ok", &secrets);
        assert!(
            !out.to_lowercase().contains("s3cret-pw"),
            "password leaked: {out}"
        );

        // <4-char components are intentionally not scrubbed (substring-noise guard)
        let short = scrub_secrets("domain=\"ab\" user=\"cd\"", &["ab\\cd".to_string()]);
        assert_eq!(short, "domain=\"ab\" user=\"cd\"");

        // <4-char FULL values scrub only as standalone words: an unbounded
        // 2-char needle mangled "NtlmConfig" (found in remediation round 1)
        let with_short = vec!["mc".to_string(), "analysisdesk\\mariu".to_string()];
        let line = "credssp_config=NtlmConfig { client_computer_name: None } cookie \"mc\"";
        let out = scrub_secrets(line, &with_short);
        assert!(
            out.contains("NtlmConfig"),
            "short needle must not mangle words: {out}"
        );
        assert!(
            !out.contains("\"mc\""),
            "standalone short value must still be scrubbed: {out}"
        );
        // the 2 sspi casing variants of the short value
        let out = scrub_secrets("user=MC user=Mc user=mC", &with_short);
        assert!(
            !out.to_lowercase().contains("mc"),
            "casing variants leaked: {out}"
        );
    }

    #[test]
    fn scrub_unicode_length_changes_do_not_panic() {
        // 'İ' lowercases to two bytes — the case-insensitive path must fall
        // back rather than slice misaligned indices.
        let secrets = vec!["analysisdesk\\mariu".to_string()];
        let line = "İstanbul analysisdesk ok";
        let out = scrub_secrets(line, &secrets);
        assert!(!out.to_lowercase().contains("analysisdesk"));
    }

    #[test]
    fn scrub_removes_encoded_credential_forms_and_keeps_marker_text() {
        // Debug-round root cause 1: sspi dumps credential buffers as
        // UTF-16LE hex (AuthIdentityBuffers) and decimal byte lists — the
        // text-only scrub passed those recoverable forms through. The
        // value net here covers the AS-STAGED encodings (the record-level
        // wholesale redaction covers anything else, incl. cased variants
        // like an uppercased NTLM domain whose encoded bytes differ).
        let secrets = vec!["analysisdesk\\mariu".to_string(), "hunter2-pw".to_string()];
        let aib_line = format!(
            "AuthIdentityBuffers {{ user: 0x{}, domain: 0x{}, password: 0x{} }}",
            utf16le_hex("mariu"),
            utf16le_hex("analysisdesk"),
            utf16le_hex("hunter2-pw"),
        );
        let scrubbed = scrub_secrets(&aib_line, &secrets);
        for needle in [
            utf16le_hex("mariu"),
            utf16le_hex("mariu").to_uppercase(),
            utf16le_hex("analysisdesk"),
            utf16le_hex("hunter2-pw"),
        ] {
            assert!(
                !scrubbed.to_lowercase().contains(&needle.to_lowercase()),
                "encoded form survived scrub: {scrubbed}"
            );
        }
        // structural marker text survives — only the payloads die
        assert!(scrubbed.contains("AuthIdentityBuffers { user:"));
        assert!(scrubbed.contains("<redacted>"));

        let enc_line = format!(
            "encrypt_message; buffers=[SecurityBufferRef {{ data: 0x{} }}]",
            utf16le_hex("hunter2-pw")
        );
        let out = scrub_secrets(&enc_line, &secrets);
        assert!(out.contains("encrypt_message"), "marker lost: {out}");
        assert!(out.contains("SecurityBufferRef"), "marker lost: {out}");
        assert!(
            !out.to_lowercase().contains(&utf16le_hex("hunter2-pw")),
            "password utf16le-hex leaked: {out}"
        );

        // decimal-list form inside a write_password_credentials-shaped line
        let dec_line = format!(
            "write_password_credentials; creds=[{}]",
            utf16le_decimal_list("analysisdesk\\mariu")
        );
        let out = scrub_secrets(&dec_line, &secrets);
        assert!(
            !out.contains(&utf16le_decimal_list("analysisdesk\\mariu")),
            "decimal utf16le list leaked: {out}"
        );
        assert!(out.contains("write_password_credentials"));

        // ASCII-hex form (hex-digit case both ways)
        let cookie_line = format!("cookie Cookie(\"{}\")", ascii_hex("mariu"));
        let out = scrub_secrets(&cookie_line, &secrets);
        assert!(
            !out.to_lowercase()
                .contains(&ascii_hex("mariu").to_lowercase()),
            "ascii-hex form leaked: {out}"
        );

        // case-VARIANT encodings (NTLM uppercases the domain inside its
        // messages — the encoded bytes differ from the staged form)
        let ntlm_line = format!(
            "nego_tokens: Some([{}, 0, 2, 0, 0, 0, 24, 0])",
            utf16le_decimal_list("ANALYSISDESK")
        );
        let out = scrub_secrets(&ntlm_line, &secrets);
        assert!(
            !out.contains(&utf16le_decimal_list("ANALYSISDESK")),
            "uppercase-domain decimal encoding leaked: {out}"
        );
        let hex_upper_domain = format!(
            "AuthIdentityBuffers {{ domain: 0x{} }}",
            utf16le_hex("MARIU")
        );
        let out = scrub_secrets(&hex_upper_domain, &secrets);
        assert!(
            !out.to_lowercase()
                .contains(&utf16le_hex("MARIU").to_lowercase()),
            "uppercase-component utf16le-hex leaked: {out}"
        );
    }

    #[test]
    fn credential_bearing_sspi_records_are_redacted_wholesale() {
        // Record-level redaction is the PRIMARY control and is
        // secret-agnostic: the record's bytes never reach the transcript,
        // whatever staged or unstaged value they embed.
        let secrets = vec!["analysisdesk\\mariu".to_string()];
        let prefix_for = |target: &str| format!("[     1.234s DEBUG {target}]");
        for (target, args) in [
            (
                "sspi::negotiate",
                "AuthIdentityBuffers { user: 0x6d006100, domain: 0x0, password: 0x6d00 }",
            ),
            (
                "sspi::credssp::client_ts_requests",
                "ts_request=TsRequest { nego_tokens: [SecurityBufferRef { .. }] }",
            ),
            ("sspi::kerberos", "write_password_credentials; length=42"),
            (
                "sspi::credssp::client",
                "encrypt_message(fulfill_context=..)",
            ),
            (
                "sspi::credssp::client",
                "  decrypt_message — leading whitespace is trimmed before the match",
            ),
            (
                "sspi::credssp::server",
                "process_ts_request; return=Ok([48, 130, 0, 0, 181, 226])",
            ),
        ] {
            let prefix = prefix_for(target);
            let line = transcript_record_line(&prefix, target, args, &secrets);
            assert_eq!(
                line,
                format!("{prefix} <credential-bearing sspi record redacted>"),
                "record not wholesale-redacted for target {target}"
            );
        }
    }

    #[test]
    fn benign_sspi_records_and_ironrdp_notables_pass_through_redaction() {
        // Over-redaction boundary (regression risk): NTLM wire-level
        // analysis records and ironrdp negotiation markers must keep
        // flowing — only credential-buffer-bearing sspi records die.
        let secrets = vec!["analysisdesk\\mariu".to_string()];
        let prefix_for = |target: &str| format!("[     2.000s DEBUG {target}]");
        for (target, args) in [
            (
                "sspi::negotiate",
                "negotiate_protocol; username=\"mariu\" domain=\"ANALYSISDESK\" protocol=\"NTLM\"",
            ),
            (
                "sspi::credssp::client_ts_requests",
                "tscredentials has been written",
            ),
            (
                "sspi::credssp::client_ts_requests",
                "CredSSp finished NLA stage.",
            ),
            // empty decimal array is NOT credential-bearing
            ("sspi::credssp::server", "process_ts_request; return=Ok([])"),
            (
                "ironrdp_connector::connection_finalize",
                "Begin NLA using CredSSP",
            ),
            (
                "ironrdp_connector::connection",
                "Server confirmed connection selected_protocol=SecurityProtocol(HYBRID_EX)",
            ),
        ] {
            let prefix = prefix_for(target);
            let line = transcript_record_line(&prefix, target, args, &secrets);
            assert_ne!(
                line,
                format!("{prefix} <credential-bearing sspi record redacted>"),
                "benign record over-redacted: {line}"
            );
            assert!(!line.contains("record redacted"), "over-redacted: {line}");
        }
        // negotiate line passes through SCRUBBED (value net still applies)
        let line = transcript_record_line(
            "[     2.000s DEBUG sspi::negotiate]",
            "sspi::negotiate",
            "negotiate_protocol; username=\"mariu\" domain=\"ANALYSISDESK\" protocol=\"NTLM\"",
            &secrets,
        );
        assert!(line.contains("negotiate_protocol"));
        assert!(line.contains("domain=\"<redacted>\""));

        // boundary: the signature check is scoped to sspi targets — a
        // non-sspi record naming the symbol is NOT wholesale-redacted
        let line = transcript_record_line(
            "[     2.000s DEBUG other_crate::x]",
            "other_crate::x",
            "talks about AuthIdentityBuffers in prose",
            &secrets,
        );
        assert!(line.contains("AuthIdentityBuffers"));
    }

    #[test]
    fn classify_nla_route_marks_credssp_nla_from_cycle1_lines() {
        let cycle1 = vec![
            "[ 0.123s DEBUG ironrdp_connector::connection_finalize] Begin NLA using CredSSP"
                .to_string(),
        ];
        let (route, credssp) = classify_nla_route(&cycle1, Some("1.3x"));
        assert!(
            route.contains("TLS rustls 1.3x -> CredSSP/NLA (connect_finalize)"),
            "{route}"
        );
        assert!(credssp.starts_with("ran:"), "{credssp}");

        // nothing captured: TLS unknown, no markers
        let (route, credssp) = classify_nla_route(&[], None);
        assert!(route.contains("TLS rustls ?"), "{route}");
        assert!(route.contains("CredSSP routing not captured"), "{route}");
        assert_eq!(credssp, "outcome not captured in transcript");
    }

    #[test]
    fn classify_nla_route_ssl_and_the_shared_bucket_hazard() {
        // host2-shaped cycle-1 evidence: server selected SSL, CredSSP off
        let host2_cycle1 = vec![
            "[ 14.454s DEBUG ironrdp_connector::connection] CredSSP is disabled".to_string(),
            "[ 14.456s INFO ironrdp_connector::connection] Server confirmed connection selected_protocol=SecurityProtocol(SSL)".to_string(),
        ];
        let (route, credssp) = classify_nla_route(&host2_cycle1, Some("1.3"));
        assert!(
            route.contains("server selected SSL; CredSSP skipped, no NLA"),
            "{route}"
        );
        assert!(credssp.starts_with("skipped by server policy"), "{credssp}");

        // HAZARD DOCUMENTATION (why the harness drains per cycle): a
        // foreign `Begin NLA` line — host1's cycle-2 reconnect lines, the
        // round-3 accumulator bug — in the SAME bucket flips the
        // classification to CredSSP. The per-cycle drain keeps foreign
        // lines out; if it regresses, this is the misclassification you
        // will see (and host1's precedence over SSL markers pins the
        // if/else order).
        let mut shared_bucket = host2_cycle1.clone();
        shared_bucket.push(
            "[ 13.910s DEBUG ironrdp_connector::connection_finalize] Begin NLA using CredSSP"
                .to_string(),
        );
        let (route, _) = classify_nla_route(&shared_bucket, Some("1.3"));
        assert!(
            route.contains("CredSSP/NLA (connect_finalize)"),
            "shared-bucket hazard changed shape: {route}"
        );
    }

    #[test]
    fn extract_bitmap_codecs_fragment_slices_the_balanced_capability() {
        let demand_active = concat!(
            "Received message=ServerDemandActive(ServerDemandActive { pdu: DemandActive ",
            "{ capability_sets: [BitmapCodecs(BitmapCodecs([Codec { id: 0, property: ",
            "NsCodec(NsCodec { color_loss_level: 3 }) }, Codec { id: 0, property: None }])), ",
            "Order(Order { desktop_save_size: 1000000 })] } })"
        );
        assert_eq!(
            extract_bitmap_codecs_fragment(demand_active).as_deref(),
            Some(concat!(
                "BitmapCodecs(BitmapCodecs([Codec { id: 0, property: ",
                "NsCodec(NsCodec { color_loss_level: 3 }) }, Codec { id: 0, property: None }]))"
            )),
            "the fragment must end at the balanced close, not swallow Order(...)"
        );
        assert_eq!(extract_bitmap_codecs_fragment("no codecs here"), None);
        // unbalanced input (truncated record) yields None, not a panic
        assert_eq!(
            extract_bitmap_codecs_fragment("BitmapCodecs(BitmapCodecs([Codec"),
            None
        );
    }

    #[test]
    fn host_smoke_report_serializes_without_any_credential_field() {
        let report = HostSmokeReport {
            host_label: "host1".into(),
            host: "10.0.0.51".into(),
            port: 3389,
            user_label: mask_user("user1"),
            auth_variant: Some("as-staged".into()),
            outcome: "connected".into(),
            connect_ms: Some(432.1),
            tls_protocol: Some("1.3".into()),
            tls_cipher_suite: Some("TLS13_AES_128_GCM_SHA256".into()),
            peer_cert_count: Some(1),
            negotiated_route: Some(
                "X.224 (connect_begin) -> TLS rustls 1.3 -> CredSSP/NLA (connect_finalize)".into(),
            ),
            negotiation_lines: vec![
                "[..] Server confirmed connection selected_protocol=SecurityProtocol(HYBRID_EX)"
                    .into(),
            ],
            codec_negotiation_lines: vec![
                "server DemandActive: BitmapCodecs(BitmapCodecs([Codec { id: 0, property: NsCodec(NsCodec { is_dynamic_fidelity_allowed: true }) }]))".into(),
                "client ConfirmActive: BitmapCodecs(BitmapCodecs([Codec { id: 0, property: None }]))".into(),
            ],
            reconnect_negotiation_lines: vec![
                "[..] Server confirmed connection selected_protocol=SecurityProtocol(HYBRID_EX)"
                    .into(),
            ],
            credssp_nla: Some("finalized-ok".into()),
            desktop_size: Some((1920, 1080)),
            io_channel_id: Some(1003),
            user_channel_id: Some(1004),
            cliprdr_channel_joined: Some(true),
            first_frame_ms: Some(210.0),
            frames_observed: Some(42),
            frames_window_ms: Some(5000.0),
            session_errors: Some(0),
            session_error_first: None,
            fastpath_mouse_move: Some("sent-ok".into()),
            display_control_resize: Some("encode_resize-returned-None".into()),
            graceful_shutdown: Some("clean".into()),
            teardown_observation: Some(
                "server closed the connection after the shutdown request (3 pdus drained)".into(),
            ),
            teardown_ms: Some(88.0),
            reconnect_cycle: None,
            error: None,
        };
        let json = serde_json::to_string(&report).expect("serializes");
        assert!(json.contains("<user1>"));
        assert!(
            json.contains("10.0.0.51"),
            "hosts+ports are allowed in evidence"
        );
        assert!(
            json.contains("codec_negotiation_lines")
                && json.contains("server DemandActive: BitmapCodecs"),
            "codec observation field rides the report"
        );
        assert!(
            json.contains("reconnect_negotiation_lines"),
            "reconnect lines ride the report in their own additive field"
        );
        assert!(!json.to_lowercase().contains("password"));
        assert!(!json.contains("PASS"));
    }
}
