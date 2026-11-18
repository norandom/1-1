use super::*;

#[tokio::test(start_paused = true)]
async fn manual_redraw_retains_image_age_and_intent_on_the_replacement_share() {
    use crate::rdp::baseline::{full_frame_rect, synthetic_connection_result};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{Duration, advance};
    for (open, supported, repaint) in [
        (false, true, true),
        (true, true, false),
        (false, false, false),
        (true, false, false),
    ] {
        let manager = SessionManager::new();
        let (id, mut peers) = start_preview_session(&manager, "manual-redraw", false, false).await;
        let mut peer = peers.recv().await.unwrap();
        for frame in bitmap_wire_frames(full_frame_rect(64, 64), &[0xf8; 8192]) {
            peer.write_all(&frame).await.unwrap();
        }
        settle_actor().await;
        manager.set_display_paused(&id, true).await.unwrap();
        assert_wire(&mut peer, &display_frame(DisplayAction::Suppress)).await;
        let paused = manager.get_session_info(&id).await.unwrap();
        let previous = paused
            .idle_preview_updated_at
            .expect("auto off still records a completed bitmap");
        if open {
            advance(Duration::from_secs(60)).await;
            settle_actor().await;
            assert_wire(&mut peer, &display_frame(DisplayAction::Allow)).await;
        }
        let mut script = reactivation_script_frames();
        script[1] = demand_active_with_suppression(96, 80, supported);
        for frame in script {
            peer.write_all(&frame).await.unwrap();
        }
        settle_actor().await;
        let info = manager.get_session_info(&id).await.unwrap();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["displayPaused"], supported);
        assert_eq!(json["displayPauseAvailable"], supported);
        assert_eq!(
            json["displayPauseUnavailableReason"].is_string(),
            !supported
        );
        assert_eq!(info.idle_preview_updated_at, Some(previous));
        assert_eq!((info.width, info.height), (96, 80));
        let mut replacement = synthetic_connection_result(96, 80);
        replacement.share_id = REACTIVATION_SHARE_ID;
        let stage = crate::rdp::client::active_stage_from_connection_result(replacement);
        let allow = encode_action(&stage, DisplayAction::Allow, 96, 80).unwrap();
        let mut wire = [0; 8192];
        let len = peer.read(&mut wire).await.unwrap();
        assert_eq!(wire[..len].ends_with(&allow), supported);
        if supported {
            assert_eq!(info.idle_preview_state, IdlePreviewState::Preview);
            if repaint {
                let rows = [0xe0, 0x07].repeat(96 * 80);
                for frame in bitmap_wire_frames(full_frame_rect(96, 80), &rows) {
                    peer.write_all(&frame).await.unwrap();
                }
            } else {
                advance(Duration::from_secs(2)).await;
            }
            settle_actor().await;
            assert_wire(
                &mut peer,
                &encode_action(&stage, DisplayAction::Suppress, 96, 80).unwrap(),
            )
            .await;
            let info = manager.get_session_info(&id).await.unwrap();
            assert_eq!(info.idle_preview_state, IdlePreviewState::Suppressed);
            assert_eq!(info.idle_preview_updated_at != Some(previous), repaint);
        } else {
            assert!(manager.set_display_paused(&id, true).await.is_err());
            send_first_key_pair(&manager.get_cmd_sender(&id).await.unwrap()).await;
            settle_actor().await;
            assert_wire(&mut peer, &expected_first_key_pair()).await;
        }
        manager.remove_session(&id).await.unwrap();
    }
}

#[test]
fn manual_redraw_counts_decoded_bitmaps_and_excludes_cursor_marker_and_dropped_updates() {
    use ironrdp_core::{ReadCursor, decode_cursor, encode_vec};
    use ironrdp_graphics::image_processing::PixelFormat;
    use ironrdp_pdu::fast_path::{
        EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode,
    };
    use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute};
    use ironrdp_pdu::rdp::headers::ShareDataPdu;
    use ironrdp_pdu::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
    use ironrdp_session::image::DecodedImage;
    let mut connection = crate::rdp::baseline::synthetic_connection_result(64, 64);
    connection.enable_server_pointer = true;
    connection.pointer_software_rendering = true;
    let mut stage = crate::rdp::client::active_stage_from_connection_result(connection);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, 64, 64);
    let pointer = encode_vec(&PointerAttribute {
        xor_bpp: 24,
        color_pointer: ColorPointerAttribute {
            cache_index: 0,
            hot_spot: Point16 { x: 0, y: 0 },
            width: 2,
            height: 2,
            xor_mask: &[0xff; 12],
            and_mask: &[0; 4],
        },
    })
    .unwrap();
    let marker = encode_vec(&SurfaceCommand::FrameMarker(FrameMarkerPdu {
        frame_action: FrameAction::Begin,
        frame_id: Some(7),
    }))
    .unwrap();
    let encoded = |kind, data: &[u8]| {
        encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: kind,
            compression_flags: None,
            compression_type: None,
            data,
        })
        .unwrap()
    };
    let mut bundle = encoded(UpdateCode::NewPointer, &pointer);
    bundle.extend(encoded(UpdateCode::SurfaceCommands, &marker));
    bundle.extend(encoded(UpdateCode::Bitmap, &[0]));
    let wire = |body: &[u8]| {
        let mut bytes =
            encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), body.len())).unwrap();
        bytes.extend(body);
        bytes
    };
    let (out, desktop) = desktop_updates::process(
        &mut stage,
        &mut image,
        ironrdp_pdu::Action::FastPath,
        &wire(&bundle),
        IO_CHANNEL_ID,
    )
    .unwrap();
    assert!(
        out.iter()
            .any(|out| matches!(out, ironrdp_session::ActiveStageOutput::GraphicsUpdate(_)))
    );
    assert!(!desktop.painted);
    assert!(desktop.pointer_seen);
    assert!(!desktop_updates::has_drawn_pixels(
        &image,
        &crate::rdp::baseline::full_frame_rect(64, 64),
        true
    ));
    let bitmap = bitmap_wire_frames(crate::rdp::baseline::full_frame_rect(2, 2), &[0xf8; 8])
        .pop()
        .unwrap();
    let mut cursor = ReadCursor::new(&bitmap);
    let _header = decode_cursor::<FastPathHeader>(&mut cursor).unwrap();
    bundle.extend(cursor.remaining());
    assert!(
        desktop_updates::process(
            &mut stage,
            &mut image,
            ironrdp_pdu::Action::FastPath,
            &wire(&bundle),
            IO_CHANNEL_ID
        )
        .unwrap()
        .1
        .painted
    );
    // The Share Data update wraps the same bitmap representation as fast-path.
    let body = decode_cursor::<FastPathUpdatePdu>(&mut cursor)
        .unwrap()
        .data;
    // Upstream decodes server Update and Pointer but only encodes client PDUs.
    // Use its empty-body share header and encode the outer MCS frame normally.
    use ironrdp_pdu::mcs::{McsMessage, SendDataIndication, decode_send_data_indication};
    use ironrdp_pdu::rdp::headers::ShareDataPduType;
    use ironrdp_pdu::x224::X224;
    let template = finalization_frame(ShareDataPdu::ShutdownDenied);
    let ctx = decode_send_data_indication(&template).unwrap();
    let share = |kind: ShareDataPduType, data: &[u8]| {
        let mut header = ctx.user_data.to_vec();
        let size = u16::try_from(header.len() + data.len()).unwrap();
        header[..2].copy_from_slice(&size.to_le_bytes());
        header[12..14].copy_from_slice(&(size - 6).to_le_bytes());
        header[14] = kind as u8;
        header.extend(data);
        header
    };
    let mcs = |user_data| {
        encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
            initiator_id: ctx.initiator_id,
            channel_id: ctx.channel_id,
            user_data: std::borrow::Cow::Owned(user_data),
        })))
        .unwrap()
    };
    let update = mcs(share(ShareDataPduType::Update, body));
    assert!(
        desktop_updates::process(
            &mut stage,
            &mut image,
            ironrdp_pdu::Action::X224,
            &update,
            IO_CHANNEL_ID
        )
        .unwrap()
        .1
        .painted
    );
    let mut slow_pointer = vec![8, 0, 0, 0]; // TS_PTRMSGTYPE_POINTER, padding
    slow_pointer.extend(pointer);
    let pointer = mcs(share(ShareDataPduType::Pointer, &slow_pointer));
    let (out, desktop) = desktop_updates::process(
        &mut stage,
        &mut image,
        ironrdp_pdu::Action::X224,
        &pointer,
        IO_CHANNEL_ID,
    )
    .unwrap();
    assert!(
        out.iter()
            .any(|out| matches!(out, ironrdp_session::ActiveStageOutput::GraphicsUpdate(_)))
    );
    assert!(!desktop.painted);
}
