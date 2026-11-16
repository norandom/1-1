//! Task 9.2 (Req 4.1, 4.2, 9.2, 9.4): EGFX adapter.
//!
//! Registers the upstream [`GraphicsPipelineClient`] on the DRDYNVC dynamic
//! virtual channel — with NO H.264 decoder (`GraphicsPipelineClient::new(
//! handler, None)`, Req 9.1/9.5 invariant) — so a real server's EGFX channel
//! creation resolves to a registered processor (the task 9.1 review
//! checkpoint: 9.1 advertises `support_dyn_vc_gfx_protocol` but nothing was
//! listening on the DVC).
//!
//! Pixel adaptation deliberately does NOT happen in a hand-rolled drain: the
//! pinned `ironrdp-session` 0.13 `ActiveStage::process` itself drains the
//! EGFX compositor after each X224 PDU (`composite_graphics_pipeline`),
//! composites decoded frames into the session `DecodedImage` (resizing it on
//! `ResetGraphics`) and emits `ActiveStageOutput::GraphicsUpdate` regions.
//! The session loop routes those regions into the SAME SharedFrame + kind-2
//! packet paths as fast-path bitmaps (Req 4.1/4.2: one compositing truth, GPU
//! surface and webview canvas both fed from SharedFrame). Frame
//! acknowledgement is likewise upstream's: `GraphicsPipelineClient` emits the
//! `FrameAcknowledge` PDU automatically after each `EndFrame`, which the
//! DRDYNVC SVC routing returns to the server as a response frame.
//!
//! Design note (shape resolution, not a fork): design.md's illustrative
//! `attach(active_stage, handler)` resolves to a CONNECTOR-time attachment —
//! the DRDYNVC static channel must be attached to the `ClientConnector`
//! before finalize so the client joins it at MCS and the server-assigned
//! channel id is bound; the stage then receives it through
//! `ConnectionResult::static_channels` (`ActiveStageBuilder`), which is the
//! #1435/#1377 shape the 4.1 probe recorded.

use ironrdp_dvc::DrdynvcClient;
use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_egfx::pdu::CapabilitySet;

/// Per-session EGFX handler (design "EGFX Session Handler": one instance per
/// SessionActor, created at connection time).
///
/// The handler owns the codec POLICY surface only: the advertised capability
/// sets come from [`crate::rdp::codec_policy::CodecPolicy::egfx_caps`] (9.1's
/// proven AVC-free set). Decoded pixels bypass the handler entirely — they
/// surface through the upstream compositor drain inside
/// `ActiveStage::process` — so this type intentionally carries no framebuffer
/// state; the "SharedFrame + channel sender" fields of the design's
/// illustrative contract live in the session loop that consumes the drain.
///
/// Must never construct or reference an H.264 decoder (Req 9.1/9.5).
#[derive(Debug, Default)]
pub struct DriftGfxHandler;

impl GraphicsPipelineHandler for DriftGfxHandler {
    /// Capability sets advertised in RDPGFX_CAPS_ADVANCE: the CodecPolicy
    /// set — ClearCodec/Planar/Uncompressed/RFX-Progressive selectable, no
    /// AVC variant (Req 9.1). Upstream additionally filters AVC-implying
    /// sets at advertisement time when no H.264 decoder is configured;
    /// this policy never offers one, so the advertisement is AVC-free by
    /// construction.
    fn capabilities(&self) -> Vec<CapabilitySet> {
        crate::rdp::codec_policy::CodecPolicy::egfx_caps()
    }
}

/// Build the DRDYNVC static-channel client carrying the graphics pipeline DVC
/// (the design's `DrdynvcClient::with_dynamic_channel` attach shape).
fn drdynvc_with_graphics_pipeline() -> DrdynvcClient {
    DrdynvcClient::new().with_dynamic_channel(GraphicsPipelineClient::new(
        Box::new(DriftGfxHandler),
        // None: no H.264 decoder — AVC-free client (Req 9.5).
        None,
    ))
}

/// Attach the graphics pipeline DVC to the connector (production seam).
///
/// Called by `attempt_rdp_connection` next to the CliprdrClient attachment:
/// the connector joins the DRDYNVC static channel at MCS, binds the
/// server-assigned channel id at finalize, and hands the whole set to the
/// session stage through `ConnectionResult::static_channels` — from where
/// `ActiveStage::process` routes EGFX traffic to the graphics pipeline and
/// drains its compositor into the session image.
pub fn attach_to_connector(connector: &mut ironrdp_connector::ClientConnector) {
    connector.attach_static_channel(drdynvc_with_graphics_pipeline());
}

/// Actor-test seam twin of [`attach_to_connector`] (task 9.2): bind the same
/// DRDYNVC + graphics pipeline wiring to a scripted `ConnectionResult`'s
/// static channels with an explicit server-assigned channel id — mirroring
/// what the connector does at finalize (`attach_channel_id_by_key`) and
/// upstream's own session fixtures — so scripted-server tests exercise the
/// production DVC registration shape end-to-end.
#[cfg(test)]
pub(crate) fn attach_to_stage_channels(
    static_channels: &mut ironrdp_svc::StaticChannelSet,
    drdynvc_channel_id: u16,
) {
    let existing = static_channels.insert(drdynvc_with_graphics_pipeline());
    debug_assert!(
        existing.is_none(),
        "task 9.2: the EGFX wiring attaches the DRDYNVC channel exactly once"
    );
    static_channels.attach_channel_id(std::any::TypeId::of::<DrdynvcClient>(), drdynvc_channel_id);
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{WriteBuf, encode_buf};
    use ironrdp_dvc::pdu::{CreateRequestPdu, DataPdu, DrdynvcDataPdu, DrdynvcServerPdu};
    use ironrdp_egfx::CHANNEL_NAME;
    use ironrdp_egfx::client::GraphicsPipelineClient;
    use ironrdp_egfx::pdu::{
        CapabilitiesConfirmPdu, Codec1Type, CreateSurfacePdu, EndFramePdu, GfxPdu,
        MapSurfaceToOutputPdu, PixelFormat, RawCapabilitySet, ResetGraphicsPdu, StartFramePdu,
        Timestamp, WireToSurface1Pdu,
    };
    use ironrdp_graphics::image_processing::PixelFormat as SessionPixelFormat;
    use ironrdp_graphics::zgfx::{CompressionMode, Compressor, compress_and_wrap_egfx};
    use ironrdp_pdu::geometry::InclusiveRectangle;
    use ironrdp_pdu::x224::X224;
    use ironrdp_pdu::{Action, encode_vec};
    use ironrdp_session::ActiveStage;
    use ironrdp_session::image::DecodedImage;

    use crate::rdp::codec_policy::CodecPolicy;
    use crate::rdp::frame_transport::{
        PACKET_KIND_DIRTY_RECTS, PACKET_KIND_FULL_FRAME, encode_image_update_packet,
    };
    use crate::renderer::shared_frame::SharedFrame;

    /// Server-assigned static channel id for DRDYNVC in the fixture
    /// (io=1003 / user=1004 mirror `baseline::synthetic_connection_result`).
    const DRDYNVC_CHANNEL_ID: u16 = 1005;
    /// Dynamic channel id the scripted server picks for the graphics channel.
    const GFX_DVC_CHANNEL_ID: u32 = 7;
    const W: u16 = 64;
    const H: u16 = 64;

    // ------------------------------------------------------------------
    // Fixture payload builders (deterministic patterns)
    // ------------------------------------------------------------------

    /// Deterministic RGBA pattern: pixel n has
    /// (n % 256, 100 + n % 100, 200 + n % 55, 255) — every byte differs per
    /// pixel so any channel swap, row reversal or wrong-stride copy fails.
    fn expected_rgba(width: u16, height: u16) -> Vec<u8> {
        let pixels = usize::from(width) * usize::from(height);
        let mut rgba = Vec::with_capacity(pixels * 4);
        for n in 0..pixels {
            rgba.extend_from_slice(&[
                (n % 256) as u8,
                100 + (n % 100) as u8,
                200 + (n % 55) as u8,
                255,
            ]);
        }
        rgba
    }

    /// Second pattern (used after reset/resize) with different values.
    fn expected_rgba_after_resize(width: u16, height: u16) -> Vec<u8> {
        let pixels = usize::from(width) * usize::from(height);
        let mut rgba = Vec::with_capacity(pixels * 4);
        for n in 0..pixels {
            rgba.extend_from_slice(&[
                230 - (n % 200) as u8,
                5 + (n % 250) as u8,
                60 + (n % 177) as u8,
                255,
            ]);
        }
        rgba
    }

    /// ClearCodec takes BGRA on the wire side of the encoder (alpha is not
    /// transported; the decoder refills 0xFF).
    fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
        let (pixels, _) = rgba.as_chunks::<4>();
        pixels
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], p[3]])
            .collect()
    }

    /// Solid-color RGBA block for the glyph-cache fixture (ClearCodec's glyph
    /// path engages at <= 1024 pixels).
    fn solid_rgba(width: u16, height: u16, value: [u8; 3]) -> Vec<u8> {
        let pixels = usize::from(width) * usize::from(height);
        let mut rgba = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            rgba.extend_from_slice(&[value[0], value[1], value[2], 255]);
        }
        rgba
    }

    fn clearcodec_payload(bgra: &[u8], width: u16, height: u16) -> Vec<u8> {
        ironrdp_graphics::clearcodec::ClearCodecEncoder::new().encode(bgra, width, height)
    }

    /// Planar (RDP6) payload from expected RGBA. `BitmapStreamEncoder` takes
    /// RGB rows; both RLE modes are exercised across tests.
    fn planar_payload(rgba: &[u8], width: u16, height: u16, rle: bool) -> Vec<u8> {
        use ironrdp_graphics::rdp6::{BitmapStreamEncoder, RgbChannels};

        let (pixels, _) = rgba.as_chunks::<4>();
        let rgb: Vec<u8> = pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        let mut encoded = vec![0u8; rgb.len() * 4 + 64];
        let len = BitmapStreamEncoder::new(usize::from(width), usize::from(height))
            .encode_bitmap::<RgbChannels>(&rgb, &mut encoded, rle)
            .expect("planar encode");
        encoded.truncate(len);
        encoded
    }

    /// Uncompressed wire format is 32-bit LE [B, G, R, A]; the client decodes
    /// to RGBA with alpha forced opaque.
    fn uncompressed_payload(rgba: &[u8]) -> Vec<u8> {
        rgba_to_bgra(rgba)
    }

    // ------------------------------------------------------------------
    // Scripted-server plumbing (real ironrdp_pdu encodings, no network)
    // ------------------------------------------------------------------

    /// Encode a GfxPdu exactly like the probe: PDU bytes -> zgfx wrap. This is
    /// the on-the-wire form inside a DVC Data PDU.
    fn zgfx_wrap(pdu: &GfxPdu, compressor: &mut Compressor) -> Vec<u8> {
        let mut buf = WriteBuf::new();
        encode_buf(pdu, &mut buf).expect("GfxPdu encode");
        compress_and_wrap_egfx(buf.filled(), compressor, CompressionMode::Never).expect("zgfx wrap")
    }

    /// A server→client MCS SendDataIndication on the DRDYNVC static channel
    /// carrying a DRDYNVC PDU — byte-identical in shape to what `read_pdu`
    /// delivers from a real transport. The PDU is wrapped in a
    /// single-fragment Channel PDU Header exactly like the SVC layer
    /// (`ChunkProcessor::dechunkify`) expects.
    fn dvc_frame(pdu: &DrdynvcServerPdu) -> Vec<u8> {
        use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
        use ironrdp_pdu::rdp::capability_sets::SERVER_CHANNEL_ID;
        use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};

        let pdu_bytes = encode_vec(pdu).expect("scripted drdynvc PDU encodes");
        let header = encode_vec(&ChannelPduHeader {
            length: pdu_bytes.len() as u32,
            flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
        })
        .expect("channel pdu header encodes");
        let mut user_data = Vec::with_capacity(header.len() + pdu_bytes.len());
        user_data.extend_from_slice(&header);
        user_data.extend_from_slice(&pdu_bytes);

        encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
            initiator_id: SERVER_CHANNEL_ID,
            channel_id: DRDYNVC_CHANNEL_ID,
            user_data: std::borrow::Cow::Owned(user_data),
        })))
        .expect("scripted x224 frame encodes")
    }

    fn gfx_data(pdu: &GfxPdu, compressor: &mut Compressor) -> DrdynvcServerPdu {
        DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
            GFX_DVC_CHANNEL_ID,
            zgfx_wrap(pdu, compressor),
        )))
    }

    fn full_rect(width: u16, height: u16) -> ironrdp_pdu::geometry::ExclusiveRectangle {
        ironrdp_pdu::geometry::ExclusiveRectangle {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        }
    }

    fn wire_pdu(codec: Codec1Type, bitmap_data: Vec<u8>) -> WireToSurface1Pdu {
        WireToSurface1Pdu {
            surface_id: 1,
            codec_id: codec,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: full_rect(W, H),
            bitmap_data,
        }
    }

    fn sub_rect_wire_pdu(
        codec: Codec1Type,
        bitmap_data: Vec<u8>,
        left: u16,
        top: u16,
        width: u16,
        height: u16,
    ) -> WireToSurface1Pdu {
        WireToSurface1Pdu {
            surface_id: 1,
            codec_id: codec,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: ironrdp_pdu::geometry::ExclusiveRectangle {
                left,
                top,
                right: left + width,
                bottom: top + height,
            },
            bitmap_data,
        }
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

    fn reset_pdu(width: u16, height: u16) -> GfxPdu {
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(width),
            height: u32::from(height),
            monitors: Vec::new(),
        })
    }

    fn create_surface_pdu(width: u16, height: u16) -> GfxPdu {
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width,
            height,
            pixel_format: PixelFormat::XRgb,
        })
    }

    fn map_surface_pdu() -> GfxPdu {
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
        })
    }

    /// Feed one server DRDYNVC PDU through the stage; collect outputs.
    fn feed(
        stage: &mut ActiveStage,
        image: &mut DecodedImage,
        pdu: &DrdynvcServerPdu,
    ) -> Vec<ironrdp_session::ActiveStageOutput> {
        stage
            .process(image, Action::X224, &dvc_frame(pdu), None)
            .expect("scripted drdynvc frame processes")
    }

    /// Build the production-shaped wired stage (the actor's seam twin).
    fn egfx_stage() -> (ActiveStage, DecodedImage) {
        let mut connection_result = crate::rdp::baseline::synthetic_connection_result(W, H);
        super::attach_to_stage_channels(&mut connection_result.static_channels, DRDYNVC_CHANNEL_ID);
        let stage = crate::rdp::client::active_stage_from_connection_result(connection_result);
        let image = DecodedImage::new(SessionPixelFormat::RgbA32, W, H);
        (stage, image)
    }

    /// Unwired control: the pre-task-9.2 stage (no DRDYNVC anywhere).
    fn bare_stage() -> (ActiveStage, DecodedImage) {
        let stage = crate::rdp::client::active_stage_from_connection_result(
            crate::rdp::baseline::synthetic_connection_result(W, H),
        );
        let image = DecodedImage::new(SessionPixelFormat::RgbA32, W, H);
        (stage, image)
    }

    /// Server channel-creation handshake + compositor surface setup. Mirrors
    /// the probe's proven sequence: Create, capabilities confirm, ResetGraphics,
    /// CreateSurface, MapSurfaceToOutput.
    fn open_graphics_channel(
        stage: &mut ActiveStage,
        image: &mut DecodedImage,
        compressor: &mut Compressor,
    ) {
        let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(
            GFX_DVC_CHANNEL_ID,
            CHANNEL_NAME.to_owned(),
        ));
        let outputs = feed(stage, image, &create);
        assert!(
            outputs
                .iter()
                .any(|o| matches!(o, ironrdp_session::ActiveStageOutput::ResponseFrame(f) if !f.is_empty())),
            "channel creation must produce a response frame (create response + caps advertise)"
        );

        let confirm = ironrdp_egfx::pdu::CapabilitySet::V10_7 {
            flags: ironrdp_egfx::pdu::CapabilitiesV107Flags::AVC_DISABLED
                | ironrdp_egfx::pdu::CapabilitiesV107Flags::SMALL_CACHE,
        };
        feed(
            stage,
            image,
            &gfx_data(
                &GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(RawCapabilitySet::from(
                    &confirm,
                ))),
                compressor,
            ),
        );
        feed(stage, image, &gfx_data(&reset_pdu(W, H), compressor));
        feed(
            stage,
            image,
            &gfx_data(&create_surface_pdu(W, H), compressor),
        );
        feed(stage, image, &gfx_data(&map_surface_pdu(), compressor));
    }

    /// Drive one complete EGFX frame (StartFrame/WireToSurface1/EndFrame) and
    /// return the accumulated GraphicsUpdate regions.
    fn feed_frame(
        stage: &mut ActiveStage,
        image: &mut DecodedImage,
        compressor: &mut Compressor,
        frame_id: u32,
        wire: WireToSurface1Pdu,
    ) -> Vec<InclusiveRectangle> {
        let mut regions = Vec::new();
        for pdu in frame_pdus(frame_id, wire) {
            for out in feed(stage, image, &gfx_data(&pdu, compressor)) {
                if let ironrdp_session::ActiveStageOutput::GraphicsUpdate(region) = out {
                    regions.push(region);
                }
            }
        }
        regions
    }

    /// Mirror of the session loop's SharedFrame sync (session.rs), including
    /// the task 9.2 reset/resize branch: when a graphics reset resized the
    /// decoded image, the SharedFrame is resized and fully repainted.
    fn sync_shared_frame(sf: &SharedFrame, image: &DecodedImage, regions: &[InclusiveRectangle]) {
        let stride = image.stride();
        let img_data = image.data();
        let mut guard = sf.begin_write();
        if guard.width() != u32::from(image.width()) || guard.height() != u32::from(image.height())
        {
            guard.update_full(
                u32::from(image.width()),
                u32::from(image.height()),
                img_data,
            );
        }
        for rect in regions {
            use ironrdp_pdu::geometry::Rectangle as _;
            let x_offset = usize::from(rect.left) * 4;
            let row_start = usize::from(rect.top) * stride + x_offset;
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
        sf.mark_dirty();
    }

    /// One parsed frame packet: kind byte, surface dims, dirty rects
    /// (left, top, width, height) and the packed RGBA payload.
    struct ParsedPacket {
        kind: u8,
        width: u16,
        height: u16,
        rects: Vec<(u16, u16, u16, u16)>,
        pixels: Vec<u8>,
    }

    /// Parse a kind-1/2 frame packet back. Layouts are frozen wire constants
    /// (`frame_transport.rs`).
    fn parse_packet(packet: &[u8]) -> ParsedPacket {
        assert!(packet.len() >= 7, "packet header present");
        let kind = packet[0];
        let width = u16::from_le_bytes([packet[1], packet[2]]);
        let height = u16::from_le_bytes([packet[3], packet[4]]);
        let count = u16::from_le_bytes([packet[5], packet[6]]) as usize;
        let mut rects = Vec::with_capacity(count);
        let mut offset = 7;
        for _ in 0..count {
            rects.push((
                u16::from_le_bytes([packet[offset], packet[offset + 1]]),
                u16::from_le_bytes([packet[offset + 2], packet[offset + 3]]),
                u16::from_le_bytes([packet[offset + 4], packet[offset + 5]]),
                u16::from_le_bytes([packet[offset + 6], packet[offset + 7]]),
            ));
            offset += 8;
        }
        ParsedPacket {
            kind,
            width,
            height,
            rects,
            pixels: packet[offset..].to_vec(),
        }
    }

    fn full_inclusive_rect(w: u16, h: u16) -> InclusiveRectangle {
        InclusiveRectangle {
            left: 0,
            top: 0,
            right: w - 1,
            bottom: h - 1,
        }
    }

    // ------------------------------------------------------------------
    // (a) DVC registration — the task 9.1 review checkpoint
    // ------------------------------------------------------------------

    #[test]
    fn attach_to_connector_registers_the_drdynvc_static_channel() {
        // The production seam (REQUIRED CHECKPOINT from the 9.1 review):
        // attempt_rdp_connection's connector carries the DRDYNVC static
        // channel after attach_to_connector, so the server's EGFX DVC
        // creation meets a registered processor.
        let config = crate::store::connections::ConnectionConfig {
            id: "egfx-attach".to_string(),
            name: "egfx-attach".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: Some(64),
            display_height: Some(64),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_connected_at: None,
            mock: false,
            library_id: String::new(),
            folder_id: None,
        };
        let connector_config = crate::rdp::client::build_connector_config(&config, "", 64, 64);
        let mut connector = ironrdp_connector::ClientConnector::new(
            connector_config,
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        );

        assert!(
            connector
                .static_channels
                .get_by_type::<ironrdp_dvc::DrdynvcClient>()
                .is_none(),
            "pre-attach control: no DRDYNVC channel on the connector"
        );
        super::attach_to_connector(&mut connector);
        assert!(
            connector
                .static_channels
                .get_by_type::<ironrdp_dvc::DrdynvcClient>()
                .is_some(),
            "attach_to_connector lands the DRDYNVC static channel"
        );
    }

    #[test]
    fn egfx_dvc_creation_resolves_to_registered_graphics_pipeline() {
        // Unwired control (pre-9.2 shape): no DRDYNVC SVC, no graphics DVC.
        let (mut bare, _bare_image) = bare_stage();
        assert!(
            bare.get_svc_processor::<ironrdp_dvc::DrdynvcClient>()
                .is_none(),
            "without the 9.2 wiring the stage carries no DRDYNVC static channel"
        );

        // Wired stage: the server's EGFX channel creation resolves to the
        // registered GraphicsPipelineClient processor (typed accessor #1377).
        let (mut stage, mut image) = egfx_stage();
        assert!(
            stage
                .get_svc_processor::<ironrdp_dvc::DrdynvcClient>()
                .is_some(),
            "the wired stage carries the DRDYNVC static channel"
        );
        {
            let drdynvc = stage
                .get_svc_processor::<ironrdp_dvc::DrdynvcClient>()
                .expect("drdynvc registered");
            assert!(
                drdynvc.has_registered_dvc::<GraphicsPipelineClient>(),
                "the graphics pipeline DVC is pre-registered"
            );
            assert!(
                drdynvc.get_dvc::<GraphicsPipelineClient>().is_none(),
                "before the server creates the channel there is no active processor"
            );
        }

        let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(
            GFX_DVC_CHANNEL_ID,
            CHANNEL_NAME.to_owned(),
        ));
        feed(&mut stage, &mut image, &create);

        let drdynvc = stage
            .get_svc_processor::<ironrdp_dvc::DrdynvcClient>()
            .expect("drdynvc registered");
        let gfx = drdynvc
            .get_dvc::<GraphicsPipelineClient>()
            .expect("server EGFX channel creation resolves to the registered processor");
        assert_eq!(gfx.channel_id(), GFX_DVC_CHANNEL_ID);
    }

    #[test]
    fn advertised_capabilities_are_the_codec_policy_set_without_avc() {
        // The handler's advertisement IS the 9.1 codec policy — the exact
        // AVC-free set CodecPolicy pins (ClearCodec/Planar/Uncompressed/
        // RFX-Progressive selectable, no AVC variant).
        let handler = super::DriftGfxHandler;
        assert_eq!(
            ironrdp_egfx::client::GraphicsPipelineHandler::capabilities(&handler),
            CodecPolicy::egfx_caps()
        );

        // Through the real client: channel creation on the wired stage
        // responds with the create response + caps advertise payload.
        let (mut stage, mut image) = egfx_stage();
        let create = DrdynvcServerPdu::Create(CreateRequestPdu::new(
            GFX_DVC_CHANNEL_ID,
            CHANNEL_NAME.to_owned(),
        ));
        let outputs = feed(&mut stage, &mut image, &create);
        let frames: Vec<&[u8]> = outputs
            .iter()
            .filter_map(|o| match o {
                ironrdp_session::ActiveStageOutput::ResponseFrame(f) => Some(f.as_slice()),
                _ => None,
            })
            .collect();
        assert!(
            frames.iter().any(|f| f.len() > 16),
            "channel creation responded with the caps advertise payload"
        );
    }

    // ------------------------------------------------------------------
    // (b) ClearCodec fixture -> handler drain -> SharedFrame + packets
    // ------------------------------------------------------------------

    #[test]
    fn clearcodec_frames_reach_shared_frame_and_packets_with_matching_pixels() {
        let (mut stage, mut image) = egfx_stage();
        let mut compressor = Compressor::new();
        open_graphics_channel(&mut stage, &mut image, &mut compressor);

        // Frame 1: full-surface ClearCodec gradient. The initial
        // ResetGraphics widened this to a full refresh.
        let expected = expected_rgba(W, H);
        let wire = wire_pdu(
            Codec1Type::ClearCodec,
            clearcodec_payload(&rgba_to_bgra(&expected), W, H),
        );
        let regions = feed_frame(&mut stage, &mut image, &mut compressor, 1, wire);
        assert_eq!(
            regions,
            vec![full_inclusive_rect(W, H)],
            "first frame after the initial reset is a full-surface region"
        );
        assert_eq!(image.data(), expected.as_slice());

        // The existing frame paths: SharedFrame (GPU feed) + kind-2 packet.
        let sf = SharedFrame::new(u32::from(W), u32::from(H));
        sync_shared_frame(&sf, &image, &regions);
        let mut scratch = Vec::new();
        let packet = encode_image_update_packet(&image, &regions, &mut scratch);
        let parsed = parse_packet(&packet);
        assert_eq!(parsed.kind, PACKET_KIND_DIRTY_RECTS);
        assert_eq!((parsed.width, parsed.height), (W, H));
        assert_eq!(parsed.rects, vec![(0, 0, W, H)]);
        assert_eq!(parsed.pixels, expected);

        // Frame 2: 32x32 ClearCodec sub-rect (glyph-cache path: <= 1024 px).
        let sub = solid_rgba(32, 32, [9, 130, 240]);
        let sub_wire = sub_rect_wire_pdu(
            Codec1Type::ClearCodec,
            clearcodec_payload(&rgba_to_bgra(&sub), 32, 32),
            16,
            16,
            32,
            32,
        );
        let regions2 = feed_frame(&mut stage, &mut image, &mut compressor, 2, sub_wire);
        assert_eq!(
            regions2,
            vec![InclusiveRectangle {
                left: 16,
                top: 16,
                right: 47,
                bottom: 47
            }]
        );
        let mut expected2 = expected.clone();
        for y in 16..48 {
            for x in 16..48 {
                let idx = ((y as usize) * W as usize + x as usize) * 4;
                expected2[idx..idx + 4].copy_from_slice(&[9, 130, 240, 255]);
            }
        }
        assert_eq!(image.data(), expected2.as_slice());

        // Frame 3: identical sub-rect — a glyph-cache HIT must decode to the
        // same pixels (no dropped/corrupted update).
        let sub_wire_repeat = sub_rect_wire_pdu(
            Codec1Type::ClearCodec,
            clearcodec_payload(&rgba_to_bgra(&sub), 32, 32),
            16,
            16,
            32,
            32,
        );
        let regions3 = feed_frame(&mut stage, &mut image, &mut compressor, 3, sub_wire_repeat);
        assert_eq!(regions3.len(), 1);
        assert_eq!(image.data(), expected2.as_slice());
    }

    // ------------------------------------------------------------------
    // (c) Allowed alternate codecs
    // ------------------------------------------------------------------

    #[test]
    fn planar_frames_decode_to_matching_pixels_rle_and_non_rle() {
        let (mut stage, mut image) = egfx_stage();
        let mut compressor = Compressor::new();
        open_graphics_channel(&mut stage, &mut image, &mut compressor);

        // Non-RLE planar.
        let expected = expected_rgba(W, H);
        let wire = wire_pdu(Codec1Type::Planar, planar_payload(&expected, W, H, false));
        let regions = feed_frame(&mut stage, &mut image, &mut compressor, 1, wire);
        assert_eq!(regions, vec![full_inclusive_rect(W, H)]);
        assert_eq!(image.data(), expected.as_slice());

        // RLE planar with the second pattern.
        let expected2 = expected_rgba_after_resize(W, H);
        let wire2 = wire_pdu(Codec1Type::Planar, planar_payload(&expected2, W, H, true));
        let regions2 = feed_frame(&mut stage, &mut image, &mut compressor, 2, wire2);
        assert_eq!(regions2.len(), 1);
        assert_eq!(image.data(), expected2.as_slice());
    }

    #[test]
    fn uncompressed_frames_decode_to_matching_pixels() {
        let (mut stage, mut image) = egfx_stage();
        let mut compressor = Compressor::new();
        open_graphics_channel(&mut stage, &mut image, &mut compressor);

        let expected = expected_rgba(W, H);
        let wire = wire_pdu(Codec1Type::Uncompressed, uncompressed_payload(&expected));
        let regions = feed_frame(&mut stage, &mut image, &mut compressor, 1, wire);
        assert_eq!(regions, vec![full_inclusive_rect(W, H)]);
        assert_eq!(image.data(), expected.as_slice());
    }

    // ------------------------------------------------------------------
    // (d) Surface reset / resize
    // ------------------------------------------------------------------

    #[test]
    fn reset_graphics_resizes_image_and_full_refreshes_shared_frame() {
        let (mut stage, mut image) = egfx_stage();
        let mut compressor = Compressor::new();
        open_graphics_channel(&mut stage, &mut image, &mut compressor);

        // Initial full frame at 64x64.
        let expected = expected_rgba(W, H);
        let wire = wire_pdu(
            Codec1Type::ClearCodec,
            clearcodec_payload(&rgba_to_bgra(&expected), W, H),
        );
        let regions = feed_frame(&mut stage, &mut image, &mut compressor, 1, wire);
        let sf = SharedFrame::new(u32::from(W), u32::from(H));
        sync_shared_frame(&sf, &image, &regions);
        assert_eq!(image.width(), W);
        assert_eq!(image.height(), H);

        // Server resets the graphics output to 96x64 (e.g. after a resize).
        feed(
            &mut stage,
            &mut image,
            &gfx_data(&reset_pdu(96, 64), &mut compressor),
        );
        feed(
            &mut stage,
            &mut image,
            &gfx_data(&create_surface_pdu(96, 64), &mut compressor),
        );
        feed(
            &mut stage,
            &mut image,
            &gfx_data(&map_surface_pdu(), &mut compressor),
        );
        assert_eq!(image.width(), 96, "ResetGraphics resized the decoded image");
        assert_eq!(image.height(), 64);

        // Next frame at the new size is widened to a full-surface refresh.
        let expected2 = expected_rgba_after_resize(96, 64);
        let wire2 = WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::ClearCodec,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: full_rect(96, 64),
            bitmap_data: clearcodec_payload(&rgba_to_bgra(&expected2), 96, 64),
        };
        let regions2 = feed_frame(&mut stage, &mut image, &mut compressor, 2, wire2);
        assert_eq!(regions2, vec![full_inclusive_rect(96, 64)]);
        assert_eq!(image.data(), expected2.as_slice());

        // The 9.2 loop adaptation resizes the SharedFrame and repaints in
        // full; the packet carries the new surface dimensions.
        sync_shared_frame(&sf, &image, &regions2);
        let snapshot = sf.publish().expect("dirty after sync");
        assert_eq!(snapshot.width, 96);
        assert_eq!(snapshot.height, 64);
        assert_eq!(snapshot.data, expected2);

        let mut scratch = Vec::new();
        let packet = encode_image_update_packet(&image, &regions2, &mut scratch);
        let parsed = parse_packet(&packet);
        assert_eq!(parsed.kind, PACKET_KIND_DIRTY_RECTS);
        assert_eq!((parsed.width, parsed.height), (96, 64));
        assert_eq!(parsed.rects, vec![(0, 0, 96, 64)]);
        assert_eq!(parsed.pixels, expected2);
    }

    // ------------------------------------------------------------------
    // (e) GPU feed — SharedFrame publish() is exactly what GpuRenderer
    //     uploads to the wgpu texture (renderer/gpu.rs render()).
    // ------------------------------------------------------------------

    #[test]
    fn composited_output_publishes_to_shared_frame_snapshot_for_gpu_feed() {
        let (mut stage, mut image) = egfx_stage();
        let mut compressor = Compressor::new();
        open_graphics_channel(&mut stage, &mut image, &mut compressor);

        let expected = expected_rgba(W, H);
        let wire = wire_pdu(
            Codec1Type::ClearCodec,
            clearcodec_payload(&rgba_to_bgra(&expected), W, H),
        );
        let regions = feed_frame(&mut stage, &mut image, &mut compressor, 1, wire);

        let sf = SharedFrame::new(u32::from(W), u32::from(H));
        sync_shared_frame(&sf, &image, &regions);
        let snapshot = sf.publish().expect("dirty after the EGFX frame sync");
        assert_eq!(snapshot.width, u32::from(W));
        assert_eq!(snapshot.height, u32::from(H));
        assert_eq!(
            snapshot.data, expected,
            "the GPU texture upload source matches the decoded ClearCodec pixels"
        );
    }

    // ------------------------------------------------------------------
    // (f) Kinds: EGFX-sourced packets are exclusively kind 1/2 (no H.264
    //     transport — Req 9.4). Existing pins freeze kind values; this test
    //     pins the EGFX route's emission set.
    // ------------------------------------------------------------------

    #[test]
    fn egfx_route_emits_only_bitmap_packet_kinds() {
        let (mut stage, mut image) = egfx_stage();
        let mut compressor = Compressor::new();
        open_graphics_channel(&mut stage, &mut image, &mut compressor);

        let expected = expected_rgba(W, H);
        let wire = wire_pdu(
            Codec1Type::ClearCodec,
            clearcodec_payload(&rgba_to_bgra(&expected), W, H),
        );
        let regions = feed_frame(&mut stage, &mut image, &mut compressor, 1, wire);

        let mut scratch = Vec::new();
        let packet = encode_image_update_packet(&image, &regions, &mut scratch);
        let kind = packet[0];
        assert!(
            kind == PACKET_KIND_FULL_FRAME || kind == PACKET_KIND_DIRTY_RECTS,
            "EGFX-sourced packets must be bitmap packets (kind 1/2), got {kind}"
        );
        assert_ne!(kind, 3, "no H.264 transport packet may be produced");
    }
}
