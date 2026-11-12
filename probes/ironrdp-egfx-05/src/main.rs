//! Task 4.1 compile probe (windows-clearcodec-royalts-e2e): proves the upstream
//! IronRDP EGFX-generation API surface required by Requirements 9.1/9.2/9.3/9.5/9.6/9.7
//! against the EXACT pinned set in this package's Cargo.toml/lockfile.
//!
//! NOT part of any CI tier (all tiers pin `--manifest-path src-tauri/Cargo.toml`).
//! Run: `cargo run --locked --manifest-path probes/ironrdp-egfx-05/Cargo.toml`.
//!
//! Checks (each maps to a requirement surface; failures exit non-zero):
//! - C1 (9.1): CodecPolicy-shaped `GraphicsPipelineHandler::capabilities()` override
//!   advertises non-AVC sets only, and the advertisement decodes back off the wire.
//! - C2 (9.1): upstream start() filters AVC-implying sets when no H.264 decoder is
//!   configured, even if the handler offers them (the default trait caps do).
//! - C3 (9.1/9.3): CapabilitiesConfirm negotiation yields `negotiated_capabilities`
//!   and `codec_capabilities()` with avc420/avc444 == false (AVC_DISABLED honored).
//! - C4 (9.2/compositor output): a Planar frame decodes to RGBA on the handler AND the
//!   compositor `drain_output()` surface (region + pixels); frame completes.
//! - C5 (9.6): a per-frame decode failure is OBSERVABLE (`dropped_updates` delta, no
//!   bitmap delivery) while the channel CONTINUES; a later good frame has a zero
//!   dropped-delta — the reset signal Drift's three-consecutive-strikes policy consumes.
//! - C6 (9.7): unsupported-codec arrival — AVC444 without a decoder is forwarded to
//!   `on_unhandled_pdu` with the codec id readable (Drift can stop with an actionable
//!   error); AVC420 without a decoder is silently skipped upstream (recorded gap).
//! - C7 (compositor reset): ResetGraphics surfaces via `take_output_reset()` (the
//!   full-frame-invalidation signal for kind-1 packets).
//! - C8 (4.2/4.3 shapes): connector `support_dyn_vc_gfx_protocol` early-cap switch,
//!   `ClientEarlyCapabilityFlags` GFX + 32bpp bits, `ActiveStageBuilder` plus the typed
//!   DVC accessor `get_dvc_mut::<GraphicsPipelineClient>()`, and the ironrdp-tokio /
//!   ironrdp-tls transport names the 4.2 adapter reuses.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use ironrdp_core::{WriteBuf, encode_buf};
use ironrdp_dvc::DvcProcessor;
use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_egfx::pdu::{
    CapabilitiesConfirmPdu, CapabilitiesV8Flags, CapabilitiesV10Flags, CapabilitiesV81Flags,
    CapabilitiesV103Flags, CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitySet, Codec1Type,
    CreateSurfacePdu, EndFramePdu, GfxPdu, MapSurfaceToOutputPdu, PixelFormat, RawCapabilitySet,
    ResetGraphicsPdu, StartFramePdu, Timestamp, WireToSurface1Pdu,
};
use ironrdp_graphics::rdp6::{BitmapStreamEncoder, RgbChannels};
use ironrdp_graphics::zgfx::{CompressionMode, Compressor, compress_and_wrap_egfx};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::decode_cursor;
use ironrdp_pdu::gcc::ClientEarlyCapabilityFlags;
use ironrdp_pdu::geometry::ExclusiveRectangle;

const CHANNEL_ID: u32 = 7;
const SURFACE_W: u16 = 64;
const SURFACE_H: u16 = 64;

static CHECKS_FAILED: AtomicUsize = AtomicUsize::new(0);

fn check(id: &str, ok: bool, what: &str) {
    if ok {
        println!("PASS {id}: {what}");
    } else {
        CHECKS_FAILED.fetch_add(1, Ordering::SeqCst);
        println!("FAIL {id}: {what}");
    }
}

/// What Drift's future `rdp/egfx.rs` handler would record, shared out of the handler.
#[derive(Default)]
struct ProbeLog {
    /// (surface_id, codec_id, width, height, rect, data_len, first RGBA pixel)
    bitmaps: Mutex<
        Vec<(
            u16,
            Codec1Type,
            u16,
            u16,
            ExclusiveRectangle,
            usize,
            [u8; 4],
        )>,
    >,
    unhandled: Mutex<Vec<&'static str>>,
    frames_completed: AtomicUsize,
    caps_confirmed: AtomicUsize,
}

/// Handler whose `capabilities()` is the non-AVC set a `CodecPolicy` would produce:
/// AVC_DISABLED on every modern set, plus the AVC-free V8 fallback.
#[derive(Default)]
struct PolicyHandler {
    log: Arc<ProbeLog>,
}

impl PolicyHandler {
    fn policy_caps() -> Vec<CapabilitySet> {
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::AVC_DISABLED | CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::AVC_DISABLED | CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::AVC_DISABLED | CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ]
    }
}

impl GraphicsPipelineHandler for PolicyHandler {
    fn capabilities(&self) -> Vec<CapabilitySet> {
        Self::policy_caps()
    }

    fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {
        self.log.caps_confirmed.fetch_add(1, Ordering::SeqCst);
    }

    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        let first = if update.data.len() >= 4 {
            [
                update.data[0],
                update.data[1],
                update.data[2],
                update.data[3],
            ]
        } else {
            [0, 0, 0, 0]
        };
        self.log.bitmaps.lock().expect("bitmaps lock").push((
            update.surface_id,
            update.codec_id,
            update.width,
            update.height,
            update.destination_rectangle.clone(),
            update.data.len(),
            first,
        ));
    }

    fn on_frame_complete(&mut self, _frame_id: u32) {
        self.log.frames_completed.fetch_add(1, Ordering::SeqCst);
    }

    fn on_unhandled_pdu(&mut self, pdu: &GfxPdu) {
        let name: &'static str = match pdu {
            GfxPdu::WireToSurface1(p) => match p.codec_id {
                Codec1Type::Avc420 => "WireToSurface1:Avc420",
                Codec1Type::Avc444 => "WireToSurface1:Avc444",
                Codec1Type::Avc444v2 => "WireToSurface1:Avc444v2",
                _ => "WireToSurface1:other",
            },
            _ => "other",
        };
        self.log
            .unhandled
            .lock()
            .expect("unhandled lock")
            .push(name);
    }
}

/// Uses the DEFAULT trait advertisement (which includes AVC-capable sets) — proves
/// upstream's no-decoder filtering in C2.
#[derive(Default)]
struct DefaultCapsHandler;

impl GraphicsPipelineHandler for DefaultCapsHandler {}

fn pdu_to_wire(pdu: &GfxPdu, compressor: &mut Compressor) -> Vec<u8> {
    let mut buf = WriteBuf::new();
    encode_buf(pdu, &mut buf).expect("GfxPdu encode");
    compress_and_wrap_egfx(buf.filled(), compressor, CompressionMode::Never).expect("zgfx wrap")
}

fn feed(
    client: &mut GraphicsPipelineClient,
    compressor: &mut Compressor,
    pdu: GfxPdu,
) -> PduResult<()> {
    let payload = pdu_to_wire(&pdu, compressor);
    client.process(CHANNEL_ID, &payload).map(|_| ())
}

fn decode_gfx(bytes: &[u8]) -> GfxPdu {
    let mut cursor = ironrdp_core::ReadCursor::new(bytes);
    decode_cursor::<GfxPdu>(&mut cursor).expect("GfxPdu decode")
}

/// Would this capability set let the server pick an AVC codec? Mirrors upstream's
/// advertisement-time filter semantics (client.rs `start()`).
fn implies_avc(cap: &CapabilitySet) -> bool {
    match cap {
        CapabilitySet::V8 { .. } => false,
        CapabilitySet::V8_1 { flags } => flags.contains(CapabilitiesV81Flags::AVC420_ENABLED),
        CapabilitySet::V10_1 => true,
        CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => {
            !flags.contains(CapabilitiesV10Flags::AVC_DISABLED)
        }
        CapabilitySet::V10_3 { flags } => !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
        CapabilitySet::V10_4 { flags }
        | CapabilitySet::V10_5 { flags }
        | CapabilitySet::V10_6 { flags }
        | CapabilitySet::V10_6Err { flags } => !flags.contains(CapabilitiesV104Flags::AVC_DISABLED),
        CapabilitySet::V10_7 { flags } => !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
    }
}

fn advertised_sets(client: &mut GraphicsPipelineClient) -> Vec<CapabilitySet> {
    let messages = client.start(CHANNEL_ID).expect("start");
    let mut sets = Vec::new();
    for message in &messages {
        let mut buf = WriteBuf::new();
        encode_buf(message.as_ref(), &mut buf).expect("advertised PDU encode");
        if let GfxPdu::CapabilitiesAdvertise(advertise) = decode_gfx(buf.filled()) {
            for raw in &advertise.0 {
                if let Some(parsed) = raw.parsed().expect("capability set parse") {
                    sets.push(parsed);
                }
            }
        }
    }
    sets
}

fn full_rect() -> ExclusiveRectangle {
    ExclusiveRectangle {
        left: 0,
        top: 0,
        right: SURFACE_W,
        bottom: SURFACE_H,
    }
}

/// Planar payload with distinct per-channel values so a channel swap or row reversal
/// fails (mirrors ironrdp-egfx's own upstream test construction). First pixel is
/// RGB(0, 100, 200) → asserted as RGBA(0, 100, 200, 255) after decode.
fn planar_rgb_payload() -> Vec<u8> {
    let pixels = usize::from(SURFACE_W) * usize::from(SURFACE_H);
    let mut rgb = Vec::with_capacity(pixels * 3);
    for n in 0..pixels {
        rgb.extend_from_slice(&[(n % 256) as u8, 100 + (n % 100) as u8, 200 + (n % 55) as u8]);
    }
    let mut encoded = vec![0u8; rgb.len() * 4 + 64];
    let len = BitmapStreamEncoder::new(usize::from(SURFACE_W), usize::from(SURFACE_H))
        .encode_bitmap::<RgbChannels>(&rgb, &mut encoded, false)
        .expect("planar encode");
    encoded.truncate(len);
    encoded
}

fn frame_pdus(frame_id: u32, wire: WireToSurface1Pdu) -> Vec<GfxPdu> {
    vec![
        GfxPdu::StartFrame(StartFramePdu {
            timestamp: Timestamp {
                milliseconds: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
            },
            frame_id,
        }),
        GfxPdu::WireToSurface1(wire),
        GfxPdu::EndFrame(EndFramePdu { frame_id }),
    ]
}

fn wire_pdu(codec: Codec1Type, bitmap_data: Vec<u8>) -> WireToSurface1Pdu {
    WireToSurface1Pdu {
        surface_id: 1,
        codec_id: codec,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: full_rect(),
        bitmap_data,
    }
}

/// Compile-shape proof for the transport names task 4.2 reuses.
mod transport_shape {
    pub fn names_resolve() -> bool {
        true
    }

    #[allow(dead_code)]
    fn _proof<S>(_framed: ironrdp_tokio::TokioFramed<S>, _stream: ironrdp_tls::TlsStream<S>) {}
}

fn main() {
    println!("== task 4.1 probe: ironrdp-egfx 0.5.0 generation ==");

    // ------------------------------------------------------------------
    // C1 + C2: advertisement-time codec policy (Req 9.1)
    // ------------------------------------------------------------------
    let log = Arc::new(ProbeLog::default());
    // AVC-free constructor (Req 9.5): no H.264 decoder exists in this build.
    let mut client =
        GraphicsPipelineClient::new(Box::new(PolicyHandler { log: log.clone() }), None);
    let sets = advertised_sets(&mut client);
    check(
        "C1a",
        !sets.is_empty(),
        "policy handler advertisement produced capability sets",
    );
    check(
        "C1b",
        sets.iter().all(|s| !implies_avc(s)),
        "no advertised policy set implies AVC (ClearCodec/Planar/Uncompressed/RFX-P remain selectable)",
    );
    check(
        "C1c",
        sets.contains(&CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::AVC_DISABLED | CapabilitiesV107Flags::SMALL_CACHE,
        }),
        "highest policy set is V10_7 with AVC_DISABLED|SMALL_CACHE preserved on the wire",
    );

    let mut default_client =
        GraphicsPipelineClient::new(Box::<DefaultCapsHandler>::default(), None);
    let default_sets = advertised_sets(&mut default_client);
    check(
        "C2",
        default_sets.iter().all(|s| !implies_avc(s)),
        "upstream filters AVC-implying sets at advertisement time when no H.264 decoder is configured",
    );

    // ------------------------------------------------------------------
    // C3: negotiation confirm (Req 9.1/9.3)
    // ------------------------------------------------------------------
    let mut compressor = Compressor::new();
    let confirm = CapabilitySet::V10_7 {
        flags: CapabilitiesV107Flags::AVC_DISABLED | CapabilitiesV107Flags::SMALL_CACHE,
    };
    feed(
        &mut client,
        &mut compressor,
        GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(RawCapabilitySet::from(&confirm))),
    )
    .expect("confirm processed");
    let caps = client.codec_capabilities();
    check(
        "C3a",
        client.negotiated_capabilities().is_some(),
        "negotiated_capabilities() is Some after confirm",
    );
    check(
        "C3b",
        !caps.avc420 && !caps.avc444,
        "negotiated caps decode to avc420=false, avc444=false (AVC_DISABLED)",
    );
    check(
        "C3c",
        log.caps_confirmed.load(Ordering::SeqCst) == 1,
        "handler on_capabilities_confirmed fired exactly once",
    );

    // ------------------------------------------------------------------
    // C4: frame decode + compositor output (Req 9.2)
    // ------------------------------------------------------------------
    feed(
        &mut client,
        &mut compressor,
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(SURFACE_W),
            height: u32::from(SURFACE_H),
            monitors: Vec::new(),
        }),
    )
    .expect("initial reset processed");
    feed(
        &mut client,
        &mut compressor,
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width: SURFACE_W,
            height: SURFACE_H,
            pixel_format: PixelFormat::XRgb,
        }),
    )
    .expect("surface created");
    feed(
        &mut client,
        &mut compressor,
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
    )
    .expect("surface mapped");
    let _ = client.take_output_reset(); // consume the initial reset marker

    let planar = planar_rgb_payload();
    for pdu in frame_pdus(1, wire_pdu(Codec1Type::Planar, planar.clone())) {
        feed(&mut client, &mut compressor, pdu).expect("planar frame processed");
    }

    {
        let bitmaps = log.bitmaps.lock().expect("bitmaps lock");
        check(
            "C4a",
            bitmaps.len() == 1,
            "exactly one on_bitmap_updated for the frame",
        );
        let (sid, codec, w, h, rect, len, first) = match bitmaps.last() {
            Some((sid, codec, w, h, rect, len, first)) => {
                (*sid, *codec, *w, *h, rect.clone(), *len, *first)
            }
            None => (0, Codec1Type::Uncompressed, 0, 0, full_rect(), 0, [0; 4]),
        };
        check(
            "C4b",
            codec == Codec1Type::Planar && sid == 1,
            "update identifies surface 1 and the Planar codec",
        );
        check(
            "C4c",
            w == SURFACE_W
                && h == SURFACE_H
                && len == usize::from(SURFACE_W) * usize::from(SURFACE_H) * 4,
            "decoded RGBA payload is width*height*4",
        );
        check(
            "C4d",
            rect == full_rect(),
            "destination rectangle preserved (exclusive right/bottom)",
        );
        check(
            "C4e",
            first == [0, 100, 200, 255],
            "first pixel is RGBA(0,100,200,255) — channel order proven",
        );
    }

    let output = client.drain_output();
    check(
        "C4f",
        output.len() == 1
            && output[0].region == full_rect()
            && output[0].data.len() == usize::from(SURFACE_W) * usize::from(SURFACE_H) * 4,
        "compositor drain_output() yields the region + composited RGBA for the frame",
    );
    check(
        "C4g",
        client.total_frames_decoded() == 1,
        "total_frames_decoded() == 1 after one frame",
    );
    check(
        "C4h",
        client.dropped_updates() == 0,
        "dropped_updates() == 0 for a clean frame",
    );
    check(
        "C4i",
        log.frames_completed.load(Ordering::SeqCst) == 1,
        "on_frame_complete fired for frame 1",
    );

    // ------------------------------------------------------------------
    // C5: per-frame decode failure observability (Req 9.6)
    // ------------------------------------------------------------------
    for pdu in frame_pdus(2, wire_pdu(Codec1Type::ClearCodec, Vec::new())) {
        feed(&mut client, &mut compressor, pdu)
            .expect("corrupt frame processed WITHOUT protocol error");
    }
    check(
        "C5a",
        client.dropped_updates() == 1,
        "dropped_updates() incremented — per-frame failure observable",
    );
    check(
        "C5b",
        log.bitmaps.lock().expect("bitmaps lock").len() == 1,
        "no bitmap delivered for the failed frame",
    );
    check(
        "C5c",
        client.total_frames_decoded() == 2 && log.frames_completed.load(Ordering::SeqCst) == 2,
        "channel CONTINUES through decode failure (frame boundary still reached)",
    );

    // A good frame after the failure: its dropped-delta is zero — the reset signal for
    // Drift's three-consecutive-strikes counter.
    for pdu in frame_pdus(3, wire_pdu(Codec1Type::Planar, planar.clone())) {
        feed(&mut client, &mut compressor, pdu).expect("recovery frame processed");
    }
    check(
        "C5d",
        client.dropped_updates() == 1,
        "dropped delta for the recovery frame is ZERO (counter reset signal)",
    );

    // ------------------------------------------------------------------
    // C6: unsupported-codec arrival (Req 9.7)
    // ------------------------------------------------------------------
    for pdu in frame_pdus(4, wire_pdu(Codec1Type::Avc444, vec![0; 16])) {
        feed(&mut client, &mut compressor, pdu).expect("avc444 frame processed");
    }
    {
        let unhandled = log.unhandled.lock().expect("unhandled lock");
        check(
            "C6a",
            unhandled.last() == Some(&"WireToSurface1:Avc444"),
            "AVC444 arrival is forwarded to on_unhandled_pdu with the codec id readable — stop-with-actionable-error hook",
        );
    }
    check(
        "C6b",
        client.dropped_updates() == 1,
        "forwarded AVC444 is not a decode drop (distinct from undecodable payloads)",
    );

    for pdu in frame_pdus(5, wire_pdu(Codec1Type::Avc420, vec![0; 16])) {
        feed(&mut client, &mut compressor, pdu).expect("avc420 frame processed");
    }
    {
        let unhandled = log.unhandled.lock().expect("unhandled lock");
        check(
            "C6c",
            !unhandled.contains(&"WireToSurface1:Avc420"),
            "GAP (recorded in research.md): AVC420-without-decoder produces NO handler callback (valid streams are silently skipped upstream, client.rs L1236-1239)",
        );
    }
    check(
        "C6d",
        client.dropped_updates() == 1,
        "this parseable AVC420 payload is FULLY invisible: neither a decode drop nor a handler event (silent skip, client.rs L1236-1239)",
    );

    // ------------------------------------------------------------------
    // C7: compositor reset surfacing (kind-1 full-frame signal)
    // ------------------------------------------------------------------
    feed(
        &mut client,
        &mut compressor,
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: 320,
            height: 200,
            monitors: Vec::new(),
        }),
    )
    .expect("reset processed");
    check(
        "C7",
        client.take_output_reset() == Some((320, 200)),
        "take_output_reset() returns the new dimensions after ResetGraphics",
    );

    // ------------------------------------------------------------------
    // C8: connector/session API shapes for tasks 4.2/4.3
    // ------------------------------------------------------------------
    // NOTE (recorded for 4.2): connector 0.12's Config has NO Default impl — all
    // fields are pub and consumers construct it literally (upstream's own pattern).
    let connector_config = ironrdp_connector::Config {
        desktop_size: ironrdp_connector::DesktopSize {
            width: 1024,
            height: 768,
        },
        monitor_layout: None,
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        enable_tls: true,
        enable_credssp: true,
        enable_standard_rdp_security: false,
        credentials: ironrdp_connector::Credentials::UsernamePassword {
            username: "probe".into(),
            password: "probe".into(),
        },
        domain: None,
        client_build: 0,
        client_name: "probe".into(),
        keyboard_type: ironrdp_pdu::gcc::KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: ironrdp_pdu::gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        platform: ironrdp_pdu::rdp::capability_sets::MajorPlatformType::WINDOWS,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        enable_audio_capture: false,
        license_cache: None,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: true,
        performance_flags: Default::default(),
        timezone_info: Default::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::SUPPORTED,
    };
    check(
        "C8a",
        connector_config.support_dyn_vc_gfx_protocol,
        "connector Config exposes support_dyn_vc_gfx_protocol (RNS_UD_CS_SUPPORT_DYNVC_GFX_PROTOCOL, Req 9.1 negotiation switch)",
    );
    let flags = ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL
        | ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION;
    check(
        "C8b",
        flags.contains(ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL)
            && flags.contains(ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION),
        "ClientEarlyCapabilityFlags combines GFX protocol + WANT_32_BPP_SESSION",
    );

    // NOTE (recorded for 4.3): ActiveStageBuilder takes its fields from the
    // connector's ConnectionResult (#1435 shape) — no Default, pub fields only.
    let mut stage = ironrdp_session::ActiveStageBuilder {
        static_channels: ironrdp_svc::StaticChannelSet::new(),
        user_channel_id: 0,
        io_channel_id: 0,
        message_channel_id: None,
        share_id: 0,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();
    check(
        "C8c",
        stage.get_dvc_mut::<GraphicsPipelineClient>().is_none(),
        "ActiveStageBuilder + typed DVC accessor get_dvc_mut::<GraphicsPipelineClient>() resolve (None until registered)",
    );
    check(
        "C8d",
        transport_shape::names_resolve(),
        "ironrdp-tokio TokioFramed and ironrdp-tls TlsStream names resolve on the pinned train",
    );

    let failed = CHECKS_FAILED.load(Ordering::SeqCst);
    if failed == 0 {
        println!("== probe result: ALL CHECKS PASSED ==");
    } else {
        println!("== probe result: {failed} CHECK(S) FAILED ==");
        std::process::exit(1);
    }
}
