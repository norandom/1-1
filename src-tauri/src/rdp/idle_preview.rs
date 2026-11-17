//! Per-connection graphics suppression for an unattended desktop.
use std::time::Duration;
use tokio::time::Instant;

const IDLE_DELAY: Duration = Duration::from_secs(300);
const PREVIEW_PERIOD: Duration = Duration::from_secs(60);
const PREVIEW_WINDOW: Duration = Duration::from_secs(2);
const RECENT_GRAPHICS: Duration = Duration::from_secs(10);
const DISTINCT_UPDATES: Duration = Duration::from_secs(1);
const MIN_TRIAL_BYTES: u64 = 4 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IdlePreviewState {
    #[default]
    Off,
    Unsupported,
    Live,
    Suppressed,
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayAction {
    Suppress,
    Allow,
}

pub struct IdlePreview {
    state: IdlePreviewState,
    next_transition: Instant,
    /// A single refresh split into many rectangles should count as one burst.
    recent_updates: [Option<Instant>; 2],
    origin: Instant,
    /// (second since connection, bytes in graphics-bearing PDUs).
    live_traffic: [(u64, u64); RECENT_GRAPHICS.as_secs() as usize],
    projected_minute_bytes: u64,
    cycle_bytes: u64,
}

impl IdlePreview {
    pub fn new(enabled: bool, supported: bool, now: Instant) -> Self {
        let state = match (enabled, supported) {
            (false, _) => IdlePreviewState::Off,
            (true, false) => IdlePreviewState::Unsupported,
            (true, true) => IdlePreviewState::Live,
        };
        Self {
            state,
            next_transition: now + IDLE_DELAY,
            recent_updates: [None; 2],
            origin: now,
            live_traffic: Default::default(),
            projected_minute_bytes: 0,
            cycle_bytes: 0,
        }
    }

    pub fn state(&self) -> IdlePreviewState {
        self.state
    }

    /// While live, count only identifiable desktop bytes. During a trial,
    /// charge every received PDU, including fragments that have yet to paint.
    pub fn on_transfer(&mut self, now: Instant, encoded_bytes: usize, desktop_painted: bool) {
        match self.state {
            IdlePreviewState::Live => {
                let second = now.duration_since(self.origin).as_secs();
                let index = (second % RECENT_GRAPHICS.as_secs()) as usize;
                let slot = &mut self.live_traffic[index];
                if slot.0 != second {
                    *slot = (second, 0);
                }
                slot.1 = slot.1.saturating_add(encoded_bytes as u64);
            }
            IdlePreviewState::Suppressed => {
                self.cycle_bytes = self.cycle_bytes.saturating_add(encoded_bytes as u64);
                return;
            }
            IdlePreviewState::Preview => {
                self.cycle_bytes = self.cycle_bytes.saturating_add(encoded_bytes as u64);
            }
            _ => return,
        }
        if desktop_painted
            && self.recent_updates[1]
                .is_none_or(|last| now.duration_since(last) >= DISTINCT_UPDATES)
        {
            self.recent_updates = [self.recent_updates[1], Some(now)];
        }
    }

    pub fn on_activity(&mut self, now: Instant) -> Option<DisplayAction> {
        if matches!(
            self.state,
            IdlePreviewState::Off | IdlePreviewState::Unsupported
        ) {
            return None;
        }
        let action = (self.state == IdlePreviewState::Suppressed).then_some(DisplayAction::Allow);
        self.state = IdlePreviewState::Live;
        self.next_transition = now + IDLE_DELAY;
        self.recent_updates = [None; 2];
        action
    }

    pub fn on_timer(&mut self, now: Instant) -> Option<DisplayAction> {
        if now < self.next_transition {
            return None;
        }
        let active_desktop = self.recent_updates[0]
            .is_some_and(|older| now.duration_since(older) <= RECENT_GRAPHICS);
        match self.state {
            IdlePreviewState::Live if active_desktop => {
                let second = now.duration_since(self.origin).as_secs();
                let recent_bytes = self
                    .live_traffic
                    .iter()
                    .filter(|(sample, _)| {
                        second.saturating_sub(*sample) < RECENT_GRAPHICS.as_secs()
                    })
                    .map(|(_, bytes)| *bytes)
                    .fold(0u64, u64::saturating_add);
                let projected = recent_bytes
                    .saturating_mul(PREVIEW_PERIOD.as_secs() / RECENT_GRAPHICS.as_secs());
                if recent_bytes < MIN_TRIAL_BYTES || !self.cycle_is_cheaper(projected) {
                    return None;
                }
                self.projected_minute_bytes = projected;
                self.cycle_bytes = 0;
                self.state = IdlePreviewState::Suppressed;
                self.next_transition = now + PREVIEW_PERIOD;
                Some(DisplayAction::Suppress)
            }
            IdlePreviewState::Suppressed => {
                self.state = IdlePreviewState::Preview;
                self.next_transition = now + PREVIEW_WINDOW;
                self.recent_updates = [None; 2];
                Some(DisplayAction::Allow)
            }
            IdlePreviewState::Preview
                if active_desktop && self.cycle_is_cheaper(self.projected_minute_bytes) =>
            {
                self.cycle_bytes = 0;
                self.state = IdlePreviewState::Suppressed;
                self.next_transition = now + (PREVIEW_PERIOD - PREVIEW_WINDOW);
                Some(DisplayAction::Suppress)
            }
            // Once the agent stops producing output, ordinary deltas can
            // cost less than requesting a full desktop on every preview.
            IdlePreviewState::Preview => {
                self.state = IdlePreviewState::Live;
                self.next_transition = now + PREVIEW_PERIOD;
                self.recent_updates = [None; 2];
                None
            }
            _ => None,
        }
    }

    fn cycle_is_cheaper(&self, projected: u64) -> bool {
        // Leave a 25% margin for variability in the resume redraw and the
        // transport overhead excluded from our graphics-byte estimate.
        projected > self.cycle_bytes.saturating_add(self.cycle_bytes / 4)
    }
}

pub fn encode_action(
    stage: &ironrdp_session::ActiveStage,
    action: DisplayAction,
    width: u16,
    height: u16,
) -> ironrdp_session::SessionResult<Vec<u8>> {
    use ironrdp_core::WriteBuf;
    use ironrdp_pdu::geometry::InclusiveRectangle;
    use ironrdp_pdu::rdp::{headers::ShareDataPdu, suppress_output::SuppressOutputPdu};
    use ironrdp_session::{SessionError, SessionErrorExt as _};

    let desktop_rect = match action {
        DisplayAction::Suppress => None,
        DisplayAction::Allow if width == 0 || height == 0 => {
            return Err(SessionError::general("empty display cannot be resumed"));
        }
        DisplayAction::Allow => Some(InclusiveRectangle {
            left: 0,
            top: 0,
            right: width - 1,
            bottom: height - 1,
        }),
    };
    let mut frame = WriteBuf::new();
    stage.encode_static(
        &mut frame,
        ShareDataPdu::SuppressOutput(SuppressOutputPdu { desktop_rect }),
    )?;
    Ok(frame.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use DisplayAction::{Allow, Suppress};
    use IdlePreviewState::{Live, Off, Preview, Suppressed, Unsupported};

    impl IdlePreview {
        fn on_graphics(&mut self, now: Instant, bytes: usize) {
            self.on_transfer(now, bytes, true);
        }
    }

    #[test]
    fn fragments_and_other_channel_traffic_cannot_count_as_desktop_activity() {
        let start = Instant::now();
        let mut p = IdlePreview::new(true, true, start);
        let idle = start + IDLE_DELAY;
        p.on_transfer(idle - DISTINCT_UPDATES, 4096, false);
        p.on_transfer(idle, 4096, false);
        assert_eq!(p.on_timer(idle), None);
        assert_eq!(p.state(), Live);

        let mut p = active_but_unattended(start);
        let preview = idle + PREVIEW_PERIOD;
        assert_eq!(p.on_timer(preview), Some(Allow));
        p.on_transfer(preview, 128, false);
        p.on_transfer(preview + DISTINCT_UPDATES, 128, false);
        p.on_transfer(preview + PREVIEW_WINDOW, 128, true);
        assert_eq!(p.cycle_bytes, 384);
        assert_eq!(p.on_timer(preview + PREVIEW_WINDOW), None);
        assert_eq!(p.state(), Live);
    }

    fn active_but_unattended(start: Instant) -> IdlePreview {
        let mut p = IdlePreview::new(true, true, start);
        p.on_graphics(start + Duration::from_secs(295), 4096);
        p.on_graphics(start + Duration::from_secs(298), 4096);
        assert_eq!(p.on_timer(start + Duration::from_secs(299)), None);
        assert_eq!(p.on_timer(start + IDLE_DELAY), Some(Suppress));
        assert_eq!(p.state(), Suppressed);
        p
    }

    #[test]
    fn disabled_and_unsupported_hosts_stay_live_on_the_wire() {
        let start = Instant::now();
        for (enabled, supported, expected) in [(false, true, Off), (true, false, Unsupported)] {
            let mut p = IdlePreview::new(enabled, supported, start);
            p.on_graphics(start + IDLE_DELAY, 4096);
            p.on_graphics(start + IDLE_DELAY + PREVIEW_WINDOW, 4096);
            assert_eq!(p.on_timer(start + IDLE_DELAY + PREVIEW_WINDOW), None);
            assert_eq!(p.on_activity(start + IDLE_DELAY + PREVIEW_WINDOW), None);
            assert_eq!(p.state(), expected);
        }
    }

    #[test]
    fn static_desktop_or_single_redraw_does_not_enter_suppression() {
        let start = Instant::now();
        let mut p = IdlePreview::new(true, true, start);
        p.on_graphics(start, 4096);
        let idle = start + IDLE_DELAY;
        assert_eq!(p.on_timer(idle), None);
        for millis in 0..10 {
            p.on_graphics(idle + Duration::from_millis(millis), 4096);
        }
        assert_eq!(p.on_timer(idle + Duration::from_secs(1)), None);
        p.on_graphics(idle + Duration::from_secs(3), 4096);
        assert_eq!(p.on_timer(idle + Duration::from_secs(3)), Some(Suppress));
    }

    #[test]
    fn tiny_frequent_deltas_stay_live_after_expensive_samples_age_out() {
        let start = Instant::now();
        let idle = start + IDLE_DELAY;
        let mut p = IdlePreview::new(true, true, start);
        p.on_graphics(idle - Duration::from_secs(11), 200_000);
        for seconds_ago in (0..10).rev() {
            p.on_graphics(idle - Duration::from_secs(seconds_ago), 48);
        }
        assert_eq!(p.on_timer(idle), None);
        assert_eq!(p.state(), Live);
    }

    #[test]
    fn expensive_refresh_returns_to_live_until_recent_workload_can_pay_for_it() {
        let start = Instant::now();
        let mut p = active_but_unattended(start);
        let preview = start + IDLE_DELAY + PREVIEW_PERIOD;
        assert_eq!(p.on_timer(preview), Some(Allow));
        // Live projection was 2 * 4096 * 6 = 49152 bytes/minute.
        p.on_graphics(preview, 40_000);
        p.on_graphics(preview + Duration::from_secs(1), 512);
        assert_eq!(p.on_timer(preview + PREVIEW_WINDOW), None);
        assert_eq!(p.state(), Live);
        let eligible = preview + PREVIEW_WINDOW + PREVIEW_PERIOD;
        p.on_graphics(eligible - Duration::from_secs(1), 4096);
        p.on_graphics(eligible, 4096);
        assert_eq!(p.on_timer(eligible), None);
        p.on_graphics(eligible + Duration::from_secs(1), 65536);
        assert_eq!(
            p.on_timer(eligible + Duration::from_secs(1)),
            Some(Suppress)
        );
    }

    #[test]
    fn servers_that_keep_sending_updates_during_suppression_fail_the_cost_check() {
        let start = Instant::now();
        let mut p = active_but_unattended(start);
        p.on_graphics(start + IDLE_DELAY + Duration::from_secs(10), 50_000);
        let preview = start + IDLE_DELAY + PREVIEW_PERIOD;
        assert_eq!(p.on_timer(preview), Some(Allow));
        p.on_graphics(preview, 128);
        p.on_graphics(preview + Duration::from_secs(1), 128);
        assert_eq!(p.on_timer(preview + PREVIEW_WINDOW), None);
        assert_eq!(p.state(), Live);
    }

    #[test]
    fn preview_intervals_allow_multiple_refinements_and_do_not_accumulate_drift() {
        let start = Instant::now();
        let mut p = active_but_unattended(start);
        let preview = start + IDLE_DELAY + PREVIEW_PERIOD;
        assert_eq!(p.on_timer(preview - Duration::from_millis(1)), None);
        assert_eq!(p.on_timer(preview), Some(Allow));
        p.on_graphics(preview, 4096);
        p.on_graphics(preview + Duration::from_secs(1), 4096);
        assert_eq!(p.on_timer(preview + Duration::from_secs(1)), None);
        assert_eq!(p.state(), Preview);
        assert_eq!(p.on_timer(preview + PREVIEW_WINDOW), Some(Suppress));
        assert_eq!(p.on_timer(preview + PREVIEW_PERIOD), Some(Allow));
    }

    #[test]
    fn quiet_preview_returns_to_live_without_another_resume_or_refresh() {
        let start = Instant::now();
        for receives_initial_redraw in [true, false] {
            let mut p = active_but_unattended(start);
            let preview = start + IDLE_DELAY + PREVIEW_PERIOD;
            assert_eq!(p.on_timer(preview), Some(Allow));
            if receives_initial_redraw {
                p.on_graphics(preview, 4096);
            }
            assert_eq!(p.on_timer(preview + PREVIEW_WINDOW), None);
            assert_eq!(p.state(), Live);
            assert_eq!(p.on_timer(preview + PREVIEW_PERIOD * 5), None);
        }
    }

    #[test]
    fn first_input_resumes_exactly_once_and_cancels_stale_preview_timers() {
        let start = Instant::now();
        let preview = start + IDLE_DELAY + PREVIEW_PERIOD;
        for wake_during_preview in [true, false] {
            let mut p = active_but_unattended(start);
            if wake_during_preview {
                assert_eq!(p.on_timer(preview), Some(Allow));
            }
            assert_eq!(
                p.on_activity(preview),
                (!wake_during_preview).then_some(Allow)
            );
            assert_eq!(p.state(), Live);
            assert_eq!(p.on_activity(preview), None);
            p.on_graphics(preview, 4096);
            p.on_graphics(preview + Duration::from_secs(1), 4096);
            assert_eq!(p.on_timer(preview + PREVIEW_WINDOW), None);
        }
    }

    #[test]
    fn encoder_matches_upstream_toggle_and_uses_current_dimensions() {
        let stage = crate::rdp::client::active_stage_from_connection_result(
            crate::rdp::baseline::synthetic_connection_result(640, 480),
        );
        for (width, height) in [(640, 480), (1280, 720), (1, 1)] {
            let pair = stage
                .request_full_redraw(width, height, false, true)
                .unwrap();
            assert_eq!(
                encode_action(&stage, Suppress, width, height).unwrap(),
                pair[0]
            );
            assert_eq!(
                encode_action(&stage, Allow, width, height).unwrap(),
                pair[1]
            );
        }
        assert!(encode_action(&stage, Allow, 0, 480).is_err());
    }
}
