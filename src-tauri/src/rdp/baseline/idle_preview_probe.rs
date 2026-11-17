//! Optional on-host experiment. Compare equal windows on the same Windows desktop.
//! Only counts and timings are reported. No keyboard input or screenshots are needed.
use super::{smoke_config, smoke_consume_outputs, smoke_stage_from_result, smoke_write_frame};
use crate::rdp::client::{ConnectionOutcome, attempt_rdp_connection};
use crate::rdp::idle_preview::{DisplayAction, encode_action};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_session::image::DecodedImage;
use std::time::Duration;
use tokio::time::{Instant, timeout, timeout_at};

#[tokio::test]
#[ignore = "requires Windows RDP lab host, run explicitly with RDP_SMOKE_HOST1/USER1/PASS1"]
async fn windows_idle_preview_traffic() {
    let endpoint = std::env::var("RDP_SMOKE_HOST1").expect("missing lab host");
    let user = std::env::var("RDP_SMOKE_USER1").expect("missing lab username");
    let pass = std::env::var("RDP_SMOKE_PASS1").expect("missing lab password");
    let (host, port) = endpoint
        .rsplit_once(':')
        .and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?)))
        .unwrap_or((&endpoint, 3389));
    let config = smoke_config("1", host, port, &user);
    let outcome = timeout(
        Duration::from_secs(35),
        attempt_rdp_connection(&config, Some(&pass), 1280, 720),
    )
    .await
    .expect("lab connection timed out");
    let ConnectionOutcome::RealSession {
        connection_result,
        framed,
        network_counters,
        ..
    } = outcome
    else {
        panic!("lab connection failed");
    };
    let supported = connection_result.suppress_output_support;
    let size = connection_result.desktop_size;
    let mut stage = smoke_stage_from_result(*connection_result);
    let mut image = DecodedImage::new(PixelFormat::RgbA32, size.width, size.height);
    let (mut read, mut write) = ironrdp_tokio::split_tokio_framed(*framed);
    eprintln!(
        "negotiated {}x{}, suppress_output={supported}",
        size.width, size.height
    );

    if supported {
        // Cover 60s live and 60s containing one preview. Count the request and redraw
        // against the preview budget. Wake separately so its cost remains visible.
        let phases = [
            ("warmup", 12, None),
            ("live", 60, None),
            ("suppressed", 58, Some(DisplayAction::Suppress)),
            ("preview", 2, Some(DisplayAction::Allow)),
            ("suppressed-again", 3, Some(DisplayAction::Suppress)),
            ("wake", 8, Some(DisplayAction::Allow)),
        ];
        let mut has_initial_graphics = false;
        for (label, seconds, action) in phases {
            let (rx0, tx0) = network_counters.totals();
            let start = Instant::now();
            if let Some(action) = action {
                let frame = encode_action(&stage, action, image.width(), image.height()).unwrap();
                smoke_write_frame(&mut write, &frame).await.unwrap();
            }
            let mut updates = 0;
            let mut first_ms = None;
            while let Ok(pdu) =
                timeout_at(start + Duration::from_secs(seconds), read.read_pdu()).await
            {
                let (kind, bytes) = pdu.expect("lost transport");
                let outputs = stage
                    .process(&mut image, kind, &bytes, None)
                    .expect("RDP decode failed");
                let effects = smoke_consume_outputs(&mut write, outputs).await.unwrap();
                assert!(
                    !effects.deactivate,
                    "server reactivated during comparison; repeat run"
                );
                if effects.graphics {
                    first_ms.get_or_insert(start.elapsed().as_millis());
                    updates += 1;
                }
            }
            let (rx, tx) = network_counters.totals();
            eprintln!(
                "{label} {seconds}s rx={} tx={} graphics_batches={updates} first_graphics_ms={first_ms:?}",
                rx - rx0,
                tx - tx0
            );
            if matches!(label, "warmup" | "live") {
                has_initial_graphics |= updates != 0;
            }
            if label == "live" {
                assert!(has_initial_graphics, "no initial desktop");
            }
            if label == "wake" {
                assert!(updates > 0, "no graphics after re-enabling output");
            }
        }
    }
    let shutdown = stage.graceful_shutdown().unwrap();
    smoke_consume_outputs(&mut write, shutdown).await.unwrap();
}
