//! Attribute decoded damage to the wire update that produced it. A software cursor
//! and even a surface frame marker can produce GraphicsUpdate without desktop pixels.
use ironrdp_core::{ReadCursor, decode_cursor, encode_vec};
use ironrdp_pdu::Action;
use ironrdp_pdu::fast_path::{
    FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode,
};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::surface_commands::SurfaceCommand;
use ironrdp_session::{
    ActiveStage, ActiveStageOutput, SessionError, SessionErrorExt as _, SessionResult,
    image::DecodedImage,
};

/// A new output begins as transparent zero. Check completed copies for actual
/// pixels. A software cursor preserves destination alpha, except its invert
/// mask turns transparent zero into opaque white.
pub(super) fn has_drawn_pixels(
    image: &DecodedImage,
    rect: &InclusiveRectangle,
    cursor_possible: bool,
) -> bool {
    let row_bytes = usize::from(rect.width()) * 4;
    let stride = image.stride();
    // The actor's batch plan has already clipped this rectangle to the image.
    (rect.top..=rect.bottom).any(|y| {
        let offset = usize::from(y) * stride + usize::from(rect.left) * 4;
        image.data()[offset..offset + row_bytes]
            .as_chunks::<4>()
            .0
            .iter()
            .any(|rgba| rgba[3] != 0 && (!cursor_possible || *rgba != [255; 4]))
    })
}

#[derive(Default)]
pub(super) struct DesktopUpdate {
    pub(super) bytes: usize,
    pub(super) painted: bool,
    pub(super) pointer_seen: bool,
}

impl DesktopUpdate {
    fn observe_fast_path(&mut self, update: &FastPathUpdatePdu<'_>, outputs: &[ActiveStageOutput]) {
        self.pointer_seen |= matches!(
            update.update_code,
            UpdateCode::ColorPointer
                | UpdateCode::NewPointer
                | UpdateCode::LargePointer
                | UpdateCode::CachedPointer
        );
        if !matches!(
            update.update_code,
            UpdateCode::Bitmap | UpdateCode::SurfaceCommands
        ) {
            return;
        }
        self.bytes += update.data.len();
        if !matches!(
            update.fragmentation,
            Fragmentation::Last | Fragmentation::Single
        ) {
            return;
        }
        let mut regions = outputs.iter().filter_map(|out| match out {
            ActiveStageOutput::GraphicsUpdate(rect) => Some(rect),
            _ => None,
        });
        if update.update_code == UpdateCode::Bitmap {
            self.painted |= regions.next().is_some();
            return;
        }
        // The pinned surface-command processor uses the origin rectangle as a
        // sentinel for marker-only or unsupported updates. Accept an origin
        // pixel only when its uncompressed command actually supplies one.
        let plain = update.fragmentation == Fragmentation::Single
            && update
                .compression_flags
                .is_none_or(|flags| flags.is_empty());
        let mut origin_pixel = false;
        if plain {
            let Ok(FastPathUpdate::SurfaceCommands(commands)) =
                FastPathUpdate::decode_with_code(update.data, update.update_code)
            else {
                return;
            };
            let mut has_bits = false;
            for command in commands {
                if let SurfaceCommand::SetSurfaceBits(bits)
                | SurfaceCommand::StreamSurfaceBits(bits) = command
                {
                    has_bits = true;
                    origin_pixel |= bits.destination.left == 0
                        && bits.destination.top == 0
                        && bits.destination.right == 1
                        && bits.destination.bottom == 1
                        && bits.extended_bitmap_data.codec_id == 0
                        && bits.extended_bitmap_data.data.len()
                            >= usize::from(bits.extended_bitmap_data.bpp).div_ceil(8)
                        && matches!(bits.extended_bitmap_data.bpp, 15 | 16 | 24 | 32);
                }
            }
            if !has_bits {
                return;
            }
        }
        self.painted |= regions.any(|rect| origin_pixel || *rect != InclusiveRectangle::empty());
    }
}

/// Log the first data on each static channel, so the app log shows which
/// channels the server actually uses (folder sharing diagnosis).
fn first_on_channel(channel_id: u16, bytes: usize) {
    use std::sync::{Mutex, OnceLock};
    static SEEN: OnceLock<Mutex<std::collections::HashSet<u16>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if seen.insert(channel_id) {
        log::info!("First data on static channel {channel_id} ({bytes} bytes)");
    }
}

fn process_x224(
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    payload: &[u8],
    io_channel_id: u16,
) -> SessionResult<(Vec<ActiveStageOutput>, DesktopUpdate)> {
    use ironrdp_pdu::mcs::{McsMessage, SendDataIndication, decode_send_data_indication};
    use ironrdp_pdu::rdp::headers::{ShareControlPduType, ShareDataPduType};
    use ironrdp_pdu::x224::X224;
    let mut desktop = DesktopUpdate::default();
    if let Ok(ctx) = decode_send_data_indication(payload)
        && ctx.channel_id != io_channel_id
    {
        first_on_channel(ctx.channel_id, ctx.user_data.len());
    }
    let Some(ctx) = decode_send_data_indication(payload)
        .ok()
        .filter(|ctx| ctx.channel_id == io_channel_id)
    else {
        return Ok((stage.process(image, Action::X224, payload, None)?, desktop));
    };
    // MS-RDPBCGR TS_SHAREDATAHEADER. The uncompressed pduType2 at offset 14
    // distinguishes an Update from a software Pointer even under bulk compression.
    let observe = |part: &[u8], outputs: &[ActiveStageOutput], desktop: &mut DesktopUpdate| {
        desktop.pointer_seen |= part.get(14) == Some(&(ShareDataPduType::Pointer as u8));
        if part
            .get(2)
            .is_some_and(|kind| u16::from(*kind & 0xf) == ShareControlPduType::DataPdu as u16)
            && part.get(14) == Some(&(ShareDataPduType::Update as u8))
            && outputs
                .iter()
                .any(|out| matches!(out, ActiveStageOutput::GraphicsUpdate(_)))
        {
            desktop.bytes += part.len();
            desktop.painted = true;
        }
    };
    let part_len = |bytes: &[u8]| {
        let len = usize::from(u16::from_le_bytes(bytes.first_chunk::<2>().copied()?));
        (len >= 6 && len <= bytes.len()).then_some(len)
    };
    let mut remaining = ctx.user_data;
    if part_len(remaining).is_none_or(|len| len == remaining.len())
        || remaining.get(2..4) == Some(&[0, 0])
    {
        let outputs = stage.process(image, Action::X224, payload, None)?;
        observe(remaining, &outputs, &mut desktop);
        return Ok((outputs, desktop));
    }
    // Concatenated Share Control PDUs can also mix a pointer with a rejected
    // bitmap. Keep each decoded region paired with its own share header.
    let mut outputs = Vec::new();
    while !remaining.is_empty() {
        let len = part_len(remaining).unwrap_or(remaining.len());
        let (part, rest) = remaining.split_at(len);
        let frame = encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
            initiator_id: ctx.initiator_id,
            channel_id: ctx.channel_id,
            user_data: part.into(),
        })))
        .map_err(SessionError::encode)?;
        let part_outputs = stage.process(image, Action::X224, &frame, None)?;
        observe(part, &part_outputs, &mut desktop);
        outputs.extend(part_outputs);
        remaining = rest;
    }
    Ok((outputs, desktop))
}

pub(super) fn process(
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    action: Action,
    payload: &[u8],
    io_channel_id: u16,
) -> SessionResult<(Vec<ActiveStageOutput>, DesktopUpdate)> {
    let mut desktop = DesktopUpdate::default();
    if action != Action::FastPath {
        return process_x224(stage, image, payload, io_channel_id);
    }
    let mut cursor = ReadCursor::new(payload);
    let header = decode_cursor::<FastPathHeader>(&mut cursor).map_err(SessionError::decode)?;
    let first = cursor.remaining();
    let parsed = decode_cursor::<FastPathUpdatePdu>(&mut cursor);
    if cursor.is_empty() || parsed.is_err() {
        let outputs = stage.process(image, action, payload, None)?;
        if let Ok(update) = parsed {
            desktop.observe_fast_path(&update, &outputs);
        }
        return Ok((outputs, desktop));
    }
    // A dropped bitmap and a software cursor in one transport frame otherwise
    // look exactly like a successful bitmap. Process packed updates in order
    // through the same decoder (including fragment and compression history).
    let mut outputs = Vec::new();
    let mut cursor = ReadCursor::new(first);
    while !cursor.is_empty() {
        let start = cursor.remaining();
        let parsed = decode_cursor::<FastPathUpdatePdu>(&mut cursor);
        let len = if parsed.is_ok() {
            start.len() - cursor.len()
        } else {
            start.len()
        };
        let mut frame =
            encode_vec(&FastPathHeader::new(header.flags, len)).map_err(SessionError::encode)?;
        frame.extend_from_slice(&start[..len]);
        let part = stage.process(image, action, &frame, None)?;
        if let Ok(update) = parsed {
            desktop.observe_fast_path(&update, &part);
            outputs.extend(part);
        } else {
            outputs.extend(part);
            break;
        }
    }
    Ok((outputs, desktop))
}
