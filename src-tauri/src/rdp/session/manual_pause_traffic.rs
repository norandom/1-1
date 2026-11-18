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

async fn start_session(manager: &SessionManager, tag: &str) -> (String, TokioFramed<DuplexStream>) {
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
            None,
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
    (id, TokioFramed::new(peer))
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
async fn manual_pause_services_clipboard_resync_and_frame_ack_then_wakes_for_paste_once() {
    let manager = SessionManager::new();
    let (id, mut peer) = start_session(&manager, "paused-channels").await;
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
    let (other, mut other_peer) = start_session(&manager, "independent-channels").await;
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
