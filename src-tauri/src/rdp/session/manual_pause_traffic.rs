use super::*;
use crate::rdp::clipboard_payload::ClipboardPayload;
use ironrdp_cliprdr::{
    CliprdrClient,
    pdu::{
        Capabilities, ClipboardFormatId, ClipboardGeneralCapabilityFlags, ClipboardPdu,
        ClipboardProtocolVersion, FormatDataRequest, FormatListResponse,
    },
};
use ironrdp_core::{Decode as _, ReadCursor, decode};
use ironrdp_dvc::pdu::DrdynvcClientPdu;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_tokio::{FramedWrite as _, TokioFramed};
use tokio::io::DuplexStream;
use tokio::sync::oneshot;
use tokio::time::{Duration, timeout};

const CLIPBOARD_CHANNEL_ID: u16 = 1006;

async fn start_session(
    manager: &SessionManager,
    tag: &str,
) -> (String, TokioFramed<DuplexStream>, Arc<SharedFrame>) {
    let shared = SharedFrame::new(u32::from(W), u32::from(H));
    let (client, peer) = tokio::io::duplex(EGFX_DUPLEX_CAPACITY);
    let client = StdMutex::new(Some(client));
    let connect: ConnectFn = Arc::new(move |_, _, _, _| {
        let client = client.lock().unwrap().take().unwrap();
        Box::pin(async move {
            let mut result = crate::rdp::baseline::synthetic_connection_result(W, H);
            result.suppress_output_support = true;
            let policy = Arc::new(CodecErrorPolicy::new());
            crate::rdp::egfx::attach_to_stage_channels(
                &mut result.static_channels,
                DRDYNVC_CHANNEL_ID,
                &policy,
            );
            result
                .static_channels
                .insert(CliprdrClient::new(Box::new(SessionClipboardBackend::new())));
            result.static_channels.attach_channel_id(
                std::any::TypeId::of::<CliprdrClient>(),
                CLIPBOARD_CHANNEL_ID,
            );
            ConnectionOutcome::RealSessionLocal {
                connection_result: Box::new(result),
                framed: Box::new(TokioFramed::new(client)),
                codec_policy: policy,
            }
        })
    });
    let (events, mut rx) = RecordedEvents::new();
    let id = manager
        .create_session_with_transport(
            test_config(tag),
            None,
            scripted_frame_channel(manager, FrameScript::Ok, Default::default()),
            SessionEventSink::recorded(events.clone()),
            false,
            Some(Arc::clone(&shared)),
            connect,
        )
        .await
        .unwrap();
    wait_for(&id, &events, &mut rx, |events| {
        events
            .last()
            .is_some_and(|(_, v)| status_string(v) == "connected")
    })
    .await;
    (id, TokioFramed::new(peer), shared)
}

async fn send_clipboard(peer: &mut TokioFramed<DuplexStream>, pdu: ClipboardPdu<'_>) {
    let bytes = encode_vec(&pdu).unwrap();
    let mut user_data = encode_vec(&ChannelPduHeader {
        length: bytes.len() as u32,
        flags: ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST,
    })
    .unwrap();
    user_data.extend(bytes);
    let wire = encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: SERVER_CHANNEL_ID,
        channel_id: CLIPBOARD_CHANNEL_ID,
        user_data: user_data.into(),
    })))
    .unwrap();
    peer.write_all(&wire).await.unwrap();
}

async fn next_frame(peer: &mut TokioFramed<DuplexStream>) -> Vec<u8> {
    timeout(Duration::from_secs(1), peer.read_pdu())
        .await
        .expect("expected an immediate response from the original socket")
        .unwrap()
        .1
        .to_vec()
}

async fn next_channel_payload(peer: &mut TokioFramed<DuplexStream>, channel: u16) -> Vec<u8> {
    let wire = next_frame(peer).await;
    let X224(McsMessage::SendDataRequest(request)) = decode(&wire).unwrap() else {
        panic!("expected virtual channel response");
    };
    assert_eq!(request.channel_id, channel, "unexpected write while paused");
    let mut cursor = ReadCursor::new(&request.user_data);
    let header = ChannelPduHeader::decode(&mut cursor).unwrap();
    assert_eq!(header.length as usize, cursor.len());
    cursor.remaining().to_vec()
}

async fn expect_display(peer: &mut TokioFramed<DuplexStream>, action: DisplayAction) {
    let stage = crate::rdp::client::active_stage_from_connection_result(
        crate::rdp::baseline::synthetic_connection_result(W, H),
    );
    assert_eq!(
        next_frame(peer).await,
        encode_action(&stage, action, W, H).unwrap()
    );
}

#[tokio::test(start_paused = true)]
async fn manual_redraw_egfx_reset_keeps_valid_pixels_until_a_completed_new_size_paint() {
    let manager = SessionManager::new();
    let (id, mut peer, shared) = start_session(&manager, "egfx-redraw").await;
    let rgba = expected_rgba(W, H);
    for frame in egfx_server_frames(&rgba) {
        peer.write_all(&frame).await.unwrap();
    }
    // The acknowledgment orders all commands and paints before the pause.
    loop {
        let payload = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
        if let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(data)) = decode(&payload).unwrap()
            && matches!(decode(data.data()), Ok(GfxPdu::FrameAcknowledge(_)))
        {
            break;
        }
    }
    manager.set_display_paused(&id, true).await.unwrap();
    expect_display(&mut peer, DisplayAction::Suppress).await;
    assert_eq!(shared.publish().unwrap().data.as_slice(), rgba);
    let previous = manager
        .get_session_info(&id)
        .await
        .unwrap()
        .idle_preview_updated_at
        .unwrap();
    let mut compressor = Compressor::new();
    let start_frame = |frame_id| {
        GfxPdu::StartFrame(StartFramePdu {
            timestamp: Timestamp {
                milliseconds: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
            },
            frame_id,
        })
    };
    let (w, h) = (96, 80);
    for pdu in [
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(w),
            height: u32::from(h),
            monitors: vec![],
        }),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 2,
            width: w,
            height: h,
            pixel_format: PixelFormat::XRgb,
        }),
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 2,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
    ] {
        peer.write_all(&gfx_data(&pdu, &mut compressor))
            .await
            .unwrap();
    }
    let stage = crate::rdp::client::active_stage_from_connection_result(
        crate::rdp::baseline::synthetic_connection_result(W, H),
    );
    assert_eq!(
        next_frame(&mut peer).await,
        encode_action(&stage, DisplayAction::Allow, w, h).unwrap()
    );
    use ironrdp_egfx::pdu::{
        CacheToSurfacePdu, Color, Point, SolidFillPdu, SurfaceToCachePdu, SurfaceToSurfacePdu,
    };
    use ironrdp_pdu::geometry::ExclusiveRectangle;
    // These callbacks also run when IronRDP cannot draw any pixels.
    let misses = [
        GfxPdu::CacheToSurface(CacheToSurfacePdu {
            cache_slot: 17,
            surface_id: 2,
            destination_points: vec![Point { x: 0, y: 0 }],
        }),
        GfxPdu::SurfaceToSurface(SurfaceToSurfacePdu {
            source_surface_id: 99,
            destination_surface_id: 2,
            source_rectangle: ExclusiveRectangle {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            },
            destination_points: vec![Point { x: 0, y: 0 }],
        }),
        GfxPdu::SolidFill(SolidFillPdu {
            surface_id: 2,
            fill_pixel: Color {
                r: 1,
                g: 2,
                b: 3,
                xa: 0,
            },
            rectangles: vec![ExclusiveRectangle {
                left: w,
                top: h,
                right: w + 1,
                bottom: h + 1,
            }],
        }),
    ];
    for pdu in misses {
        peer.write_all(&gfx_data(&pdu, &mut compressor))
            .await
            .unwrap();
    }
    let mut bad = clearcodec_wire(&rgba, W, H);
    bad.surface_id = 2;
    bad.bitmap_data = vec![]; // EndFrame may still expose the freshly mapped blank surface.
    for frame in egfx_frame_frames(2, bad, &mut compressor) {
        peer.write_all(&frame).await.unwrap();
    }
    let _ack = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
    let info = manager.get_session_info(&id).await.unwrap();
    assert_eq!((info.width, info.height), (u32::from(w), u32::from(h)));
    assert_eq!(info.idle_preview_state, IdlePreviewState::Preview);
    assert_eq!(info.idle_preview_updated_at, Some(previous));
    assert!(
        shared.publish().is_none(),
        "blank map and dropped tile replaced the last valid image"
    );
    // A real copy from transparent allocation has compositor damage of its
    // own. It still provides no desktop pixels on the freshly reset output.
    for pdu in [
        start_frame(3),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 9,
            width: w,
            height: h,
            pixel_format: PixelFormat::XRgb,
        }),
        GfxPdu::SurfaceToSurface(SurfaceToSurfacePdu {
            source_surface_id: 9,
            destination_surface_id: 2,
            source_rectangle: ExclusiveRectangle {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            },
            destination_points: vec![Point { x: 0, y: 0 }],
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 3 }),
    ] {
        peer.write_all(&gfx_data(&pdu, &mut compressor))
            .await
            .unwrap();
    }
    let _ack = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
    assert_eq!(
        manager
            .get_session_info(&id)
            .await
            .unwrap()
            .idle_preview_updated_at,
        Some(previous)
    );
    assert!(shared.publish().is_none());

    let resized = expected_rgba_after_resize(w, h);
    // Surface 3 is decoded while offscreen. A fresh blank mapping for surface 2
    // in the same frame produces compositor damage without presenting those pixels.
    for pdu in [
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 3,
            width: w,
            height: h,
            pixel_format: PixelFormat::XRgb,
        }),
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 2,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
    ] {
        peer.write_all(&gfx_data(&pdu, &mut compressor))
            .await
            .unwrap();
    }
    let mut good = clearcodec_wire(&resized, w, h);
    good.surface_id = 3;
    for frame in egfx_frame_frames(4, good, &mut compressor) {
        peer.write_all(&frame).await.unwrap();
    }
    let _ack = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
    assert!(
        shared.publish().is_none(),
        "offscreen pixels gave the blank mapping paint credit"
    );
    assert_eq!(
        manager
            .get_session_info(&id)
            .await
            .unwrap()
            .idle_preview_updated_at,
        Some(previous)
    );

    // EndFrame 5 has no bitmap. Mapping the populated surface makes frame 4's
    // pixels visible on the output for the first time.
    for pdu in [
        start_frame(5),
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 3,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 5 }),
    ] {
        peer.write_all(&gfx_data(&pdu, &mut compressor))
            .await
            .unwrap();
    }
    let _ack = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
    expect_display(&mut peer, DisplayAction::Suppress).await;
    let info = manager.get_session_info(&id).await.unwrap();
    assert!(info.display_paused);
    assert_eq!(info.idle_preview_state, IdlePreviewState::Suppressed);
    assert!(info.idle_preview_updated_at > Some(previous));
    assert_eq!(shared.publish().unwrap().data.as_slice(), resized);
    let before_copy = info.idle_preview_updated_at;
    // A genuine cache hit can populate an unmapped surface. End its frame,
    // then map it to the reset output, exactly like the staged bitmap above.
    for pdu in [
        GfxPdu::SurfaceToCache(SurfaceToCachePdu {
            surface_id: 3,
            cache_key: 1,
            cache_slot: 17,
            source_rectangle: ExclusiveRectangle {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            },
        }),
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(w),
            height: u32::from(h),
            monitors: vec![],
        }),
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 5,
            width: w,
            height: h,
            pixel_format: PixelFormat::XRgb,
        }),
        start_frame(6),
        GfxPdu::CacheToSurface(CacheToSurfacePdu {
            cache_slot: 17,
            surface_id: 5,
            destination_points: vec![Point { x: 0, y: 0 }],
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 6 }),
        start_frame(7),
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 5,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
        GfxPdu::EndFrame(EndFramePdu { frame_id: 7 }),
    ] {
        peer.write_all(&gfx_data(&pdu, &mut compressor))
            .await
            .unwrap();
    }
    assert_eq!(
        next_frame(&mut peer).await,
        encode_action(&stage, DisplayAction::Allow, w, h).unwrap()
    );
    for _ in 0..2 {
        let _ack = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
    }
    expect_display(&mut peer, DisplayAction::Suppress).await;
    assert!(
        manager
            .get_session_info(&id)
            .await
            .unwrap()
            .idle_preview_updated_at
            > before_copy
    );
    assert_eq!(shared.publish().unwrap().data.as_slice(), resized);
    manager.remove_session(&id).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn manual_pause_services_clipboard_resync_and_frame_ack_then_wakes_for_paste_once() {
    let manager = SessionManager::new();
    let (id, mut peer, _) = start_session(&manager, "paused-channels").await;
    let sender = manager.get_cmd_sender(&id).await.unwrap();
    manager.set_display_paused(&id, true).await.unwrap();
    expect_display(&mut peer, DisplayAction::Suppress).await;

    send_clipboard(
        &mut peer,
        ClipboardPdu::Capabilities(Capabilities::new(
            ClipboardProtocolVersion::V2,
            ClipboardGeneralCapabilityFlags::empty(),
        )),
    )
    .await;
    send_clipboard(&mut peer, ClipboardPdu::MonitorReady).await;
    for expected in [
        "CLIPRDR_CAPABILITIES",
        "CLIPRDR_TEMP_DIRECTORY",
        "CLIPRDR_FORMAT_LIST",
    ] {
        let payload = next_channel_payload(&mut peer, CLIPBOARD_CHANNEL_ID).await;
        assert_eq!(
            decode::<ClipboardPdu<'_>>(&payload).unwrap().message_name(),
            expected
        );
    }
    let ok = ClipboardPdu::FormatListResponse(FormatListResponse::Ok);
    send_clipboard(&mut peer, ok.clone()).await;
    // The ACK and the server request share this ordered stream. Receiving the
    // data response guarantees the actor has processed the bootstrap ACK.
    let request = ClipboardPdu::FormatDataRequest(FormatDataRequest {
        format: ClipboardFormatId::CF_UNICODETEXT,
    });
    send_clipboard(&mut peer, request.clone()).await;
    let payload = next_channel_payload(&mut peer, CLIPBOARD_CHANNEL_ID).await;
    assert!(matches!(
        decode::<ClipboardPdu<'_>>(&payload).unwrap(),
        ClipboardPdu::FormatDataResponse(_)
    ));

    let (reply, context) = oneshot::channel();
    sender
        .send(SessionCommand::ClipboardContext { reply })
        .await
        .unwrap();
    let epoch = context.await.unwrap().epoch;
    let (reply, mut sync) = oneshot::channel();
    sender
        .send(SessionCommand::ClipboardSync {
            epoch: epoch.clone(),
            payload: Some(ClipboardPayload::Text("resync during pause".into())),
            reply,
        })
        .await
        .unwrap();
    let payload = next_channel_payload(&mut peer, CLIPBOARD_CHANNEL_ID).await;
    let ClipboardPdu::FormatList(formats) = decode(&payload).unwrap() else {
        panic!("expected Resync to advertise new formats");
    };
    assert!(
        formats
            .get_formats(false)
            .unwrap()
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT)
    );

    // The first session's pending Resync must not block pause or resume on a
    // second socket, or be consumed by that session's operations.
    let (other, mut other_peer, _) = start_session(&manager, "independent-channels").await;
    manager.set_display_paused(&other, true).await.unwrap();
    expect_display(&mut other_peer, DisplayAction::Suppress).await;
    manager.set_display_paused(&other, false).await.unwrap();
    expect_display(&mut other_peer, DisplayAction::Allow).await;
    manager.remove_session(&other).await.unwrap();

    // An in-flight graphics frame arrives while the Resync ACK is outstanding.
    for frame in egfx_server_frames(&expected_rgba(W, H)) {
        peer.write_all(&frame).await.unwrap();
    }
    loop {
        let payload = next_channel_payload(&mut peer, DRDYNVC_CHANNEL_ID).await;
        if let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(data)) = decode(&payload).unwrap()
            && let GfxPdu::FrameAcknowledge(ack) = decode(data.data()).unwrap()
        {
            assert_eq!(ack.frame_id, 1);
            assert_eq!(ack.total_frames_decoded, 1);
            break;
        }
    }
    assert_eq!(sync.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    send_clipboard(&mut peer, ok).await;
    assert_eq!(
        timeout(Duration::from_secs(1), sync)
            .await
            .unwrap()
            .unwrap(),
        Ok(())
    );
    send_clipboard(&mut peer, request).await;
    let payload = next_channel_payload(&mut peer, CLIPBOARD_CHANNEL_ID).await;
    let ClipboardPdu::FormatDataResponse(response) = decode(&payload).unwrap() else {
        panic!("expected the requested clipboard contents");
    };
    assert_eq!(response.to_unicode_string().unwrap(), "resync during pause");
    let (reply, expired) = oneshot::channel();
    drop(expired);
    sender
        .send(SessionCommand::ClipboardPaste {
            epoch: epoch.clone(),
            reply,
        })
        .await
        .unwrap();
    // The context reply follows the expired paste in the actor's FIFO.
    let (reply, context) = oneshot::channel();
    sender
        .send(SessionCommand::ClipboardContext { reply })
        .await
        .unwrap();
    assert_eq!(context.await.unwrap().epoch, epoch);
    assert!(
        timeout(Duration::from_millis(5), peer.read_pdu())
            .await
            .is_err(),
        "expired paste wrote a display or input frame"
    );
    assert_eq!(
        manager
            .get_session_info(&id)
            .await
            .unwrap()
            .idle_preview_state,
        IdlePreviewState::Suppressed
    );

    for open_preview in [false, true] {
        if open_preview {
            manager.set_display_paused(&id, true).await.unwrap();
            expect_display(&mut peer, DisplayAction::Suppress).await;
            tokio::time::advance(Duration::from_secs(60)).await;
            expect_display(&mut peer, DisplayAction::Allow).await;
        }
        let (reply, pasted) = oneshot::channel();
        sender
            .send(SessionCommand::ClipboardPaste {
                epoch: epoch.clone(),
                reply,
            })
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(1), pasted)
                .await
                .unwrap()
                .unwrap(),
            Ok(())
        );
        if !open_preview {
            expect_display(&mut peer, DisplayAction::Allow).await;
        }
        let input = decode::<FastPathInput>(&next_frame(&mut peer).await).unwrap();
        assert_eq!(
            input.input_events(),
            &[
                FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1d),
                FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2f),
                FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2f),
                FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1d),
            ]
        );
        assert!(
            timeout(Duration::from_millis(5), peer.read_pdu())
                .await
                .is_err(),
            "paste wrote another frame"
        );
        assert_eq!(
            manager
                .get_session_info(&id)
                .await
                .unwrap()
                .idle_preview_state,
            IdlePreviewState::Off
        );
    }
    manager.remove_session(&id).await.unwrap();
}
