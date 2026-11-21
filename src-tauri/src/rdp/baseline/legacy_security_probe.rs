//! Optional on-host check of legacy RDP security (`rdp/legacy_security.rs`)
//! against a host that refuses TLS and NLA. It sends no password: without
//! NLA the host draws its sign-in screen in the session, which is enough to
//! prove the key exchange and both directions of encryption.
use super::{smoke_config, smoke_consume_outputs, smoke_stage_from_result};
use crate::rdp::client::{ConnectionOutcome, attempt_rdp_connection};
use crate::store::connections::SecurityLayer;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use std::time::Duration;
use tokio::time::{Instant, timeout, timeout_at};

#[tokio::test]
#[ignore = "requires a host limited to Standard RDP Security, run explicitly with RDP_LEGACY_HOST"]
async fn legacy_security_reaches_the_sign_in_screen() {
    let endpoint = std::env::var("RDP_LEGACY_HOST").expect("missing host");
    let user = std::env::var("RDP_LEGACY_USER").unwrap_or_default();
    let (host, port) = endpoint
        .rsplit_once(':')
        .and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?)))
        .unwrap_or((&endpoint, 3389));
    let mut config = smoke_config("legacy", host, port, &user);
    config.host_profile.security = SecurityLayer::Legacy;
    let outcome = timeout(
        Duration::from_secs(35),
        attempt_rdp_connection(&config, None, 1280, 720),
    )
    .await
    .expect("connection timed out");
    let (connection_result, framed, network_counters) = match outcome {
        ConnectionOutcome::RealSessionLegacy {
            connection_result,
            framed,
            network_counters,
            ..
        } => (connection_result, framed, network_counters),
        ConnectionOutcome::Failed(reason) => panic!("connection failed: {reason}"),
        _ => panic!("not a legacy session"),
    };
    let size = connection_result.desktop_size;
    let mut stage = smoke_stage_from_result(*connection_result);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
    let (mut read, mut write) = ironrdp_tokio::split_tokio_framed(*framed);
    eprintln!("negotiated {}x{}", size.width, size.height);

    let seconds = std::env::var("RDP_LEGACY_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    // More packets than one key lasts (4096), so the client's key update is
    // checked by the server: it drops the connection on a bad signature.
    let moves: u16 = std::env::var("RDP_LEGACY_MOVES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    for step in 0..moves {
        use ironrdp_pdu::input::fast_path::FastPathInputEvent;
        use ironrdp_pdu::input::mouse::{MousePdu, PointerFlags};
        let event = FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: 600 + step % 100,
            y_position: 600 + (step / 100) % 100,
        });
        let outputs = stage
            .process_fastpath_input(&mut image, &[event])
            .expect("input encoded");
        smoke_consume_outputs(&mut write, outputs).await.unwrap();
    }
    let end = Instant::now() + Duration::from_secs(seconds);
    let (mut pdus, mut updates) = (0u32, 0u32);
    while let Ok(pdu) = timeout_at(end, read.read_pdu()).await {
        let (kind, bytes) = pdu.expect("lost transport");
        pdus += 1;
        let outputs = stage
            .process(&mut image, kind, &bytes, None)
            .expect("RDP decode failed");
        let effects = smoke_consume_outputs(&mut write, outputs).await.unwrap();
        if effects.deactivate {
            eprintln!("server deactivated the share; stopping");
            break;
        }
        updates += u32::from(effects.graphics);
    }
    let (rx, tx) = network_counters.totals();
    eprintln!("{pdus} PDUs, {updates} graphics batches, rx={rx} tx={tx}");
    if let Ok(path) = std::env::var("RDP_LEGACY_PNG") {
        image::RgbaImage::from_raw(
            u32::from(image.width()),
            u32::from(image.height()),
            image.data().to_vec(),
        )
        .expect("image size")
        .save(path)
        .expect("PNG written");
    }
    assert!(updates > 0, "no graphics arrived");
    if let Ok(shutdown) = stage.graceful_shutdown() {
        let _ = smoke_consume_outputs(&mut write, shutdown).await;
    }
}
