//! Codec policy for the non-AVC graphics pipeline (task 9.1, Req 9.1/9.3/9.5).
//!
//! Single source of truth for every graphics capability Drift advertises:
//!
//! * [`CodecPolicy::egfx_caps`] — the RDPGFX capability sets advertised when
//!   the EGFX dynamic virtual channel comes up: V10_7/V10_6/V10_4, each with
//!   `AVC_DISABLED | SMALL_CACHE`, plus the AVC-free V8 fallback (the exact
//!   set proven against the pinned crate by the task 4.1 probe, checks
//!   C1/C2/C3). ClearCodec, Planar, Uncompressed and RFX Progressive remain
//!   server-selectable; no AVC (H.264) variant is ever implied (Req 9.1).
//! * [`CodecPolicy::early_capability_flags`] — the codec-policy bits of the
//!   GCC client early capability flags: `SUPPORT_DYN_VC_GFX_PROTOCOL |
//!   WANT_32_BPP_SESSION` (Req 9.1).
//! * [`CodecPolicy::fastpath_bitmap`] — the fast-path bitmap configuration
//!   for sessions without EGFX: 32 bpp, lossless, no surface-command bitmap
//!   codec advertisement, so the session stays on standard/uncompressed
//!   bitmap updates (Req 9.3).
//!
//! Design-contract type resolution (design.md CodecPolicy L282-292 names were
//! illustrative; resolved against the pinned crate, nothing invented):
//!
//! * `CapsAdvertise` -> `Vec<CapabilitySet>` — the return type upstream's
//!   `GraphicsPipelineHandler::capabilities()` advertises (ironrdp-egfx 0.5).
//! * `EarlyCapabilityFlags` -> `ClientEarlyCapabilityFlags` (ironrdp-pdu 0.11,
//!   `ironrdp_pdu::gcc`).
//! * `BitmapCapabilitySet` -> `ironrdp_connector::BitmapConfig` — the type the
//!   connector's `Config.bitmap` field consumes; the wire `Bitmap` capability
//!   set is derived from it internally by the connector
//!   (connection_activation.rs, `CapabilitySet::Bitmap` construction).
//!
//! Channel selection and observed update codec are DISTINCT observations
//! (design.md "Codec routing": log channel and actual codec separately; EGFX
//! activity alone is not ClearCodec evidence). [`GraphicsChannel`] types the
//! negotiation-time observation, [`ObservedCodec`] the per-update one; the
//! EGFX adapter (task 9.2) and the error policy (task 9.3) consume both.
//!
//! This module contains NO decode logic — it only builds and classifies
//! capability/codec values using the pinned upstream types.

use std::fmt;

use ironrdp_connector::BitmapConfig;
use ironrdp_egfx::pdu::{
    CapabilitiesV8Flags, CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitySet, Codec1Type,
    Codec2Type,
};
use ironrdp_pdu::gcc::ClientEarlyCapabilityFlags;
use ironrdp_pdu::rdp::capability_sets::BitmapCodecs;

/// The graphics codec advertisement policy (task 9.1, Req 9.1/9.3/9.5).
///
/// Stateless: every associated function builds its answer from the pinned
/// upstream types, so the connector config builder
/// (`rdp::client::build_connector_config`) and the EGFX handler (task 9.2)
/// share ONE advertisement source.
pub struct CodecPolicy;

impl CodecPolicy {
    /// Capability sets advertised on the EGFX channel (RDPGFX_CAPS_ADVANCE).
    ///
    /// V10_7, V10_6 and V10_4 — each `AVC_DISABLED | SMALL_CACHE` — plus the
    /// AVC-free V8 fallback, most capable first: the server picks the most
    /// capable set it knows, and every modern set here disables AVC, so
    /// ClearCodec, Planar, Uncompressed and RFX Progressive stay selectable
    /// while no H.264/AVC variant is advertised (Req 9.1). Upstream
    /// additionally filters AVC-implying sets at advertisement time when no
    /// H.264 decoder is configured (`GraphicsPipelineClient::start`); the
    /// policy never offers one, so the advertisement is AVC-free by
    /// construction, decoder or not (Req 9.5).
    ///
    /// Production consumer is the EGFX adapter's `capabilities()` override
    /// (task 9.2); until then only the policy tests call it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn egfx_caps() -> Vec<CapabilitySet> {
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

    /// Codec-policy bits of the GCC client early capability flags (Req 9.1):
    /// request the graphics pipeline DVC and a 32bpp session.
    ///
    /// The connector ORs its own base flags and derives the same information
    /// from its config switches (`support_dyn_vc_gfx_protocol` sets
    /// `SUPPORT_DYN_VC_GFX_PROTOCOL`; `bitmap.color_depth == 32` sets
    /// `WANT_32_BPP_SESSION` — ironrdp-connector 0.12 `create_gcc_blocks`).
    pub fn early_capability_flags() -> ClientEarlyCapabilityFlags {
        ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL
            | ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION
    }

    /// Value for the connector's `support_dyn_vc_gfx_protocol` switch — the
    /// advertisement side of [`Self::early_capability_flags`].
    pub fn support_dyn_vc_gfx_protocol() -> bool {
        Self::early_capability_flags()
            .contains(ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL)
    }

    /// Fast-path bitmap configuration for sessions without EGFX (Req 9.3):
    /// 32 bpp, lossless, and NO surface-command bitmap codec advertisement.
    ///
    /// With an empty codec list the connector advertises no
    /// `BitmapCodecs`/`SurfaceCommands` capability sets, keeping the session
    /// on standard bitmap updates — the fallback whose decoded frames already
    /// flow through Drift's existing fast-path bitmap packet pipeline.
    pub fn fastpath_bitmap() -> BitmapConfig {
        BitmapConfig {
            lossy_compression: false,
            color_depth: 32,
            codecs: BitmapCodecs(Vec::new()),
        }
    }
}

/// Which graphics channel a session uses — a NEGOTIATION-time observation
/// (what was advertised and confirmed), independent of the codec any single
/// update carries (design.md "Codec routing": log channel and actual codec
/// observations separately; EGFX activity alone is not ClearCodec evidence).
///
/// Production consumer is the EGFX adapter's observation logging (task 9.2);
/// until then only the policy tests exercise it.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphicsChannel {
    /// EGFX pipeline negotiated: the graphics DVC came up. `confirmed` holds
    /// the server-confirmed capability set once the RDPGFX_CAPS_CONFIRM
    /// exchange completed; `None` while that confirmation is still pending.
    Egfx {
        /// Server-confirmed RDPGFX capability set, once received.
        confirmed: Option<CapabilitySet>,
    },
    /// No EGFX: the session runs on standard/uncompressed fast-path bitmap
    /// updates (the Req 9.3 fallback — [`CodecPolicy::fastpath_bitmap`]).
    FastpathBitmap,
}

impl GraphicsChannel {
    /// Channel selection implied by the negotiation outcome: EGFX is in use
    /// only once the server confirmed a capability set on the graphics
    /// channel; before that — and whenever EGFX never comes up — the session
    /// stays on the fast-path bitmap route.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_confirmed_caps(confirmed: Option<&CapabilitySet>) -> Self {
        match confirmed {
            Some(caps) => Self::Egfx {
                confirmed: Some(caps.clone()),
            },
            None => Self::FastpathBitmap,
        }
    }
}

impl fmt::Display for GraphicsChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Egfx { confirmed } => match confirmed {
                Some(_) => f.write_str("egfx"),
                None => f.write_str("egfx(confirm-pending)"),
            },
            Self::FastpathBitmap => f.write_str("fastpath-bitmap"),
        }
    }
}

/// A codec this policy allows on the EGFX channel (design.md "Codec
/// routing"): the supported non-AVC set. "Lack of ClearCodec alone does not
/// imply lack of EGFX" — the server may select any of these per update.
///
/// Production consumer is the EGFX adapter's observation logging (task 9.2);
/// until then only the policy tests exercise it.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedCodec {
    /// Raw/uncompressed bitmap data (RDPGFX WIRE_TO_SURFACE_1 codec 0x0).
    Uncompressed,
    /// ClearCodec bitmap compression (RDPGFX WIRE_TO_SURFACE_1 codec 0x8).
    ClearCodec,
    /// Planar (RDP6) bitmap compression (codec 0xA).
    Planar,
    /// RemoteFX Progressive (RDPGFX WIRE_TO_SURFACE_2, codec 0x9).
    RemoteFxProgressive,
}

impl fmt::Display for SupportedCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SupportedCodec::Uncompressed => "uncompressed",
            SupportedCodec::ClearCodec => "clearcodec",
            SupportedCodec::Planar => "planar",
            SupportedCodec::RemoteFxProgressive => "rfx-progressive",
        })
    }
}

/// An AVC (H.264) codec variant. Never advertised by [`CodecPolicy`];
/// observing one means the server violated the negotiated capabilities and
/// feeds the terminal error policy (task 9.3, Req 9.6/9.7) — never the
/// decode pipeline.
///
/// Production consumer is the EGFX adapter's observation logging (task 9.2);
/// until then only the policy tests exercise it.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvcVariant {
    /// H.264 4:2:0 (codec 0xB).
    Avc420,
    /// H.264 4:4:4 (codec 0xE).
    Avc444,
    /// H.264 4:4:4 v2 (codec 0xF).
    Avc444v2,
}

impl fmt::Display for AvcVariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            AvcVariant::Avc420 => "avc420",
            AvcVariant::Avc444 => "avc444",
            AvcVariant::Avc444v2 => "avc444v2",
        })
    }
}

/// The codec a received EGFX update actually used — a PER-UPDATE observation,
/// distinct from the channel selection ([`GraphicsChannel`]).
///
/// Classified against the policy's allowed set; AVC variants are kept
/// separately identifiable so an unadvertised AVC arrival can be reported
/// and made terminal (Req 9.6/9.7) instead of being conflated with the
/// supported codecs (Req 9.1/9.3).
///
/// Production consumer is the EGFX adapter's observation logging (task 9.2)
/// and the error policy (task 9.3); until then only the policy tests
/// exercise it.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedCodec {
    /// A supported non-AVC codec this policy allows on EGFX.
    Supported(SupportedCodec),
    /// An AVC (H.264) variant — unadvertised by policy; terminal if observed.
    Avc(AvcVariant),
    /// Any other wire codec id outside both sets (e.g. RemoteFx 0x3, Alpha
    /// 0xC), carried with its raw wire value for actionable errors.
    Other(u16),
}

impl ObservedCodec {
    /// Classify a WIRE_TO_SURFACE_1 codec id (the `codec_id` of a
    /// `WireToSurface1Pdu` / decoded `BitmapUpdate`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_wire_codec1(codec: Codec1Type) -> Self {
        match codec {
            Codec1Type::Uncompressed => Self::Supported(SupportedCodec::Uncompressed),
            Codec1Type::ClearCodec => Self::Supported(SupportedCodec::ClearCodec),
            Codec1Type::Planar => Self::Supported(SupportedCodec::Planar),
            Codec1Type::Avc420 => Self::Avc(AvcVariant::Avc420),
            Codec1Type::Avc444 => Self::Avc(AvcVariant::Avc444),
            Codec1Type::Avc444v2 => Self::Avc(AvcVariant::Avc444v2),
            other => Self::Other(u16::from(other)),
        }
    }

    /// Classify a WIRE_TO_SURFACE_2 codec id (RFX Progressive arrives on
    /// `WireToSurface2Pdu`; decoded tiles surface as Uncompressed-tagged
    /// bitmap updates).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_wire_codec2(codec: Codec2Type) -> Self {
        match codec {
            Codec2Type::RemoteFxProgressive => Self::Supported(SupportedCodec::RemoteFxProgressive),
        }
    }

    /// True for an AVC (H.264) variant — never advertised by this policy.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_avc(&self) -> bool {
        matches!(self, Self::Avc(_))
    }

    /// True for a codec in the policy's supported non-AVC set.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_supported_non_avc(&self) -> bool {
        matches!(self, Self::Supported(_))
    }
}

impl fmt::Display for ObservedCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Supported(codec) => write!(f, "{}", codec),
            Self::Avc(variant) => write!(f, "{}", variant),
            Self::Other(id) => write!(f, "codec-{:#04x}", id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ironrdp_core::{ReadCursor, WriteBuf, encode_buf};
    use ironrdp_dvc::DvcProcessor;
    use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::pdu::{
        CapabilitiesConfirmPdu, CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV103Flags,
        GfxPdu, RawCapabilitySet,
    };
    use ironrdp_graphics::zgfx::{CompressionMode, Compressor, compress_and_wrap_egfx};
    use ironrdp_pdu::decode_cursor;

    const CHANNEL_ID: u32 = 7;

    /// Handler exactly as task 9.2's EGFX adapter will consume the policy:
    /// `capabilities()` returns the CodecPolicy set (the probe's C1 shape).
    struct PolicyCapsHandler;

    impl GraphicsPipelineHandler for PolicyCapsHandler {
        fn capabilities(&self) -> Vec<CapabilitySet> {
            CodecPolicy::egfx_caps()
        }
    }

    /// Would this capability set let the server pick an AVC codec? Mirrors
    /// upstream `CodecCapabilities::from_capability_set` (ironrdp-egfx
    /// client.rs) and the advertisement-time filter in `start()` — the same
    /// semantics the task 4.1 probe used for check C1b.
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
            | CapabilitySet::V10_6Err { flags } => {
                !flags.contains(CapabilitiesV104Flags::AVC_DISABLED)
            }
            CapabilitySet::V10_7 { flags } => !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
        }
    }

    /// Drive the advertisement exchange exactly like the task 4.1 probe: the
    /// client's `start()` messages encode, decode back off the wire, and the
    /// advertised capability sets are returned (probe `advertised_sets`).
    fn advertised_sets(client: &mut GraphicsPipelineClient) -> Vec<CapabilitySet> {
        let messages = client.start(CHANNEL_ID).expect("start");
        let mut sets = Vec::new();
        for message in &messages {
            let mut buf = WriteBuf::new();
            encode_buf(message.as_ref(), &mut buf).expect("advertised PDU encode");
            let mut cursor = ReadCursor::new(buf.filled());
            if let GfxPdu::CapabilitiesAdvertise(advertise) =
                decode_cursor::<GfxPdu>(&mut cursor).expect("advertised PDU decode")
            {
                for raw in &advertise.0 {
                    if let Some(parsed) = raw.parsed().expect("capability set parse") {
                        sets.push(parsed);
                    }
                }
            }
        }
        sets
    }

    /// Wrap a GfxPdu the way the server transport delivers it (zfg framing,
    /// no compression — probe `pdu_to_wire`).
    fn wrap_for_channel(pdu: &GfxPdu, compressor: &mut Compressor) -> Vec<u8> {
        let mut buf = WriteBuf::new();
        encode_buf(pdu, &mut buf).expect("GfxPdu encode");
        compress_and_wrap_egfx(buf.filled(), compressor, CompressionMode::Never).expect("zgfx wrap")
    }

    // Req 9.1: the advertised set is exactly the proven policy set — modern
    // versions with AVC disabled plus the AVC-free V8 fallback — and no set
    // implies any AVC variant.
    #[test]
    fn egfx_caps_advertise_supported_codecs_without_avc() {
        let caps = CodecPolicy::egfx_caps();

        assert_eq!(
            caps.len(),
            4,
            "policy advertises exactly four capability sets"
        );
        assert_eq!(
            caps.first(),
            Some(&CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::AVC_DISABLED | CapabilitiesV107Flags::SMALL_CACHE,
            }),
            "the most capable set leads (the server picks the most capable advertised set)"
        );
        assert!(
            caps.contains(&CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::AVC_DISABLED | CapabilitiesV104Flags::SMALL_CACHE,
            }),
            "V10_6 must be advertised with AVC disabled"
        );
        assert!(
            caps.contains(&CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::AVC_DISABLED | CapabilitiesV104Flags::SMALL_CACHE,
            }),
            "V10_4 must be advertised with AVC disabled"
        );
        assert!(
            caps.contains(&CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            }),
            "the AVC-free V8 fallback must be advertised for older servers"
        );
        assert!(
            caps.iter().all(|cap| !implies_avc(cap)),
            "no advertised set may let the server pick an AVC codec (Req 9.1/9.5)"
        );
    }

    // Req 9.1: through the REAL upstream client, with no H.264 decoder
    // configured, the policy advertisement survives the wire round-trip and
    // a server capability confirm yields non-AVC negotiated codec
    // capabilities (probe checks C1/C3 — ClearCodec, Planar, Uncompressed
    // and RFX Progressive stay selectable with AVC off).
    #[test]
    fn egfx_caps_advertise_and_confirm_without_avc_through_upstream_client() {
        // AVC-free constructor (Req 9.5): no H.264 decoder exists in this build.
        let mut client = GraphicsPipelineClient::new(Box::new(PolicyCapsHandler), None);

        let sets = advertised_sets(&mut client);
        assert_eq!(
            sets,
            CodecPolicy::egfx_caps(),
            "the advertisement must carry the policy sets unchanged over the wire"
        );
        assert!(
            sets.iter().all(|cap| !implies_avc(cap)),
            "upstream advertisement (with its no-decoder AVC filter) must contain no AVC set"
        );

        // Server confirms the leading set; negotiation must resolve with
        // every AVC capability off.
        let confirm = CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::AVC_DISABLED | CapabilitiesV107Flags::SMALL_CACHE,
        };
        let mut compressor = Compressor::new();
        let payload = wrap_for_channel(
            &GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(RawCapabilitySet::from(&confirm))),
            &mut compressor,
        );
        client
            .process(CHANNEL_ID, &payload)
            .expect("capabilities confirm processed");

        assert!(
            client.negotiated_capabilities().is_some(),
            "negotiated_capabilities() must resolve after the server confirm"
        );
        let caps = client.codec_capabilities();
        assert!(
            !caps.avc420 && !caps.avc444,
            "negotiated codec capabilities must have avc420=false and avc444=false (Req 9.1)"
        );
    }

    // Req 9.1: early capability flags request the graphics pipeline and a
    // 32bpp session, and the connector switch mirrors the GFX bit.
    #[test]
    fn early_flags_request_gfx_protocol_and_32bpp() {
        let flags = CodecPolicy::early_capability_flags();
        assert!(
            flags.contains(ClientEarlyCapabilityFlags::SUPPORT_DYN_VC_GFX_PROTOCOL),
            "early flags must request the EGFX dynamic virtual channel (Req 9.1)"
        );
        assert!(
            flags.contains(ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION),
            "early flags must request a 32bpp session"
        );
        assert!(
            CodecPolicy::support_dyn_vc_gfx_protocol(),
            "the connector GFX switch must be derived from (and ON in) the policy flags"
        );
    }

    // Req 9.3: the no-EGFX fallback is a 32bpp lossless fast-path bitmap
    // config with no bitmap-codec advertisement — standard/uncompressed
    // bitmap updates only.
    #[test]
    fn fastpath_bitmap_is_the_no_egfx_fallback() {
        let bitmap = CodecPolicy::fastpath_bitmap();
        assert_eq!(
            bitmap.color_depth, 32,
            "fallback must prefer 32bpp (drives WANT_32_BPP_SESSION in the connector)"
        );
        assert!(
            !bitmap.lossy_compression,
            "fallback must not request lossy compression (no color subsampling)"
        );
        assert!(
            bitmap.codecs.0.is_empty(),
            "fallback must advertise no bitmap codec: no SurfaceCommands/BitmapCodecs caps, standard bitmap updates only (Req 9.3)"
        );
    }

    // Design "Codec routing": channel selection and observed codec are
    // separate observations. The channel is chosen at negotiation; the codec
    // is observed per update. EGFX activity alone is not ClearCodec evidence.
    #[test]
    fn channel_selection_and_observed_codec_are_distinct_observations() {
        // Channel selection: no confirmed EGFX caps -> fast-path bitmap route.
        assert_eq!(
            GraphicsChannel::from_confirmed_caps(None),
            GraphicsChannel::FastpathBitmap,
            "without an EGFX confirmation the session runs on fast-path bitmaps (Req 9.3)"
        );
        let confirmed = CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::AVC_DISABLED | CapabilitiesV107Flags::SMALL_CACHE,
        };
        let egfx = GraphicsChannel::from_confirmed_caps(Some(&confirmed));
        assert_eq!(
            egfx,
            GraphicsChannel::Egfx {
                confirmed: Some(confirmed.clone()),
            },
            "a confirmed capability set selects the EGFX channel and is retained for logging"
        );
        assert_ne!(
            egfx,
            GraphicsChannel::FastpathBitmap,
            "channel states must be distinguishable"
        );

        // Observed codec: per-update classification of the supported non-AVC
        // set (Req 9.3 "server may use another supported non-AVC EGFX codec").
        for (wire, expected) in [
            (Codec1Type::Uncompressed, SupportedCodec::Uncompressed),
            (Codec1Type::ClearCodec, SupportedCodec::ClearCodec),
            (Codec1Type::Planar, SupportedCodec::Planar),
        ] {
            let observed = ObservedCodec::from_wire_codec1(wire);
            assert_eq!(
                observed,
                ObservedCodec::Supported(expected),
                "{wire:?} must classify as a supported non-AVC codec"
            );
            assert!(observed.is_supported_non_avc() && !observed.is_avc());
        }
        assert_eq!(
            ObservedCodec::from_wire_codec2(Codec2Type::RemoteFxProgressive),
            ObservedCodec::Supported(SupportedCodec::RemoteFxProgressive),
            "RFX Progressive (WireToSurface2) is part of the supported set"
        );

        // AVC variants: never advertised; classified distinctly so an
        // unadvertised arrival is actionable (Req 9.1 advertisement, 9.6/9.7
        // error policy), never decoded.
        for wire in [Codec1Type::Avc420, Codec1Type::Avc444, Codec1Type::Avc444v2] {
            let observed = ObservedCodec::from_wire_codec1(wire);
            assert!(observed.is_avc(), "{wire:?} must classify as AVC");
            assert!(!observed.is_supported_non_avc());
        }

        // Codecs outside both sets keep their wire id for actionable errors.
        let other = ObservedCodec::from_wire_codec1(Codec1Type::RemoteFx);
        assert_eq!(other, ObservedCodec::Other(0x3));
        assert!(!other.is_supported_non_avc() && !other.is_avc());

        // The two observations render to separate log fields (design: "log
        // channel/actual codec observations separately").
        assert_ne!(
            format!("{egfx}"),
            format!(
                "{}",
                ObservedCodec::from_wire_codec1(Codec1Type::ClearCodec)
            ),
            "channel selection and observed codec must render distinctly in logs"
        );
    }

    // Req 9.5 pin: the locked build graph carries no H.264 encoder crate.
    // The advertisement-level AVC absence is proven above; this pins the
    // dependency graph itself (deterministic: Cargo.lock is committed).
    #[test]
    fn build_carries_no_h264_encoder_dependency() {
        let lock_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
        let lock = std::fs::read_to_string(&lock_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", lock_path.display()));

        for banned in [
            "openh264",
            "openh264-sys",
            "openh264-bundled",
            "x264",
            "x264-sys",
        ] {
            assert!(
                !lock.contains(&format!("name = \"{banned}\"")),
                "H.264 encoder dependency '{banned}' must never enter the locked build (Req 9.5)"
            );
        }
        assert!(
            lock.contains("name = \"ironrdp-egfx\""),
            "the pinned EGFX crate must stay in the locked build"
        );
    }
}
