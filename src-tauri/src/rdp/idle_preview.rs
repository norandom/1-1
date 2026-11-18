//! Per-connection graphics suppression for an unattended desktop.
use std::time::Duration;
use tokio::time::Instant;

const IDLE_DELAY: Duration = Duration::from_secs(300);
const PREVIEW_PERIOD: Duration = Duration::from_secs(60);
const PREVIEW_WINDOW: Duration = Duration::from_secs(2);
const RECENT_GRAPHICS: Duration = Duration::from_secs(10);
const DISTINCT_UPDATES: Duration = Duration::from_secs(1);
const MIN_TRIAL_BYTES: u64 = 4 * 1024;
const SUPPRESSION_GRACE: Duration = Duration::from_secs(2);
const OBSERVATION_INTERVAL: Duration = Duration::from_secs(10);
const REFUSAL_INTERVALS: u8 = 3;

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

struct SuppressedPaints {
    observe_from: Instant,
    last_interval: Option<u64>,
    consecutive: u8,
}

impl SuppressedPaints {
    fn new(now: Instant) -> Self {
        Self {
            observe_from: now + SUPPRESSION_GRACE,
            last_interval: None,
            consecutive: 0,
        }
    }

    fn interval(&self, now: Instant) -> Option<u64> {
        now.checked_duration_since(self.observe_from)
            .map(|elapsed| elapsed.as_secs() / OBSERVATION_INTERVAL.as_secs())
    }

    fn record(&mut self, now: Instant) {
        let Some(interval) = self.interval(now) else {
            return;
        };
        if self.last_interval == Some(interval) || self.consecutive == REFUSAL_INTERVALS {
            return;
        }
        self.consecutive = if self.last_interval.is_some_and(|last| last + 1 == interval) {
            self.consecutive + 1
        } else {
            1
        };
        self.last_interval = Some(interval);
    }

    fn confirmed(&self, now: Instant) -> bool {
        self.consecutive == REFUSAL_INTERVALS && self.interval(now) > self.last_interval
    }
}

struct ManualPause {
    next_preview: Instant,
    redraw_pending: bool,
}

pub struct IdlePreview {
    state: IdlePreviewState,
    automatic_enabled: bool,
    manual: Option<ManualPause>,
    suppressed_paints: SuppressedPaints,
    pause_unavailable_reason: Option<&'static str>,
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
    pub fn pause(&mut self, now: Instant) -> Option<DisplayAction> {
        if self.manual.is_some() || self.pause_unavailable_reason.is_some() {
            return None;
        }
        let action = if self.state == IdlePreviewState::Suppressed {
            None
        } else {
            Some(self.suppress_until(now, now + PREVIEW_PERIOD))
        };
        self.manual = Some(ManualPause {
            next_preview: self.next_transition,
            redraw_pending: false,
        });
        action
    }

    pub fn is_manually_paused(&self) -> bool {
        self.manual.is_some()
    }

    pub fn pause_unavailable_reason(&self) -> Option<&'static str> {
        self.pause_unavailable_reason
    }
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "Task 1.3 wires resize redraws to the actor")
)]
impl IdlePreview {
    pub fn begin_manual_redraw(&mut self, now: Instant) -> Option<DisplayAction> {
        self.manual.as_mut()?.redraw_pending = true;
        (self.state == IdlePreviewState::Suppressed).then(|| self.open_preview(now))
    }

    pub fn finish_manual_redraw(&mut self, now: Instant) -> Option<DisplayAction> {
        if self.manual.as_ref()?.redraw_pending {
            self.close_manual_preview(now)
        } else {
            None
        }
    }
}

impl IdlePreview {
    fn close_manual_preview(&mut self, now: Instant) -> Option<DisplayAction> {
        let manual = self.manual.as_mut()?;
        manual.redraw_pending = false;
        if manual.next_preview <= now {
            manual.next_preview = now + PREVIEW_PERIOD;
        }
        let deadline = manual.next_preview;
        Some(self.suppress_until(now, deadline))
    }

    fn suppress_until(&mut self, now: Instant, deadline: Instant) -> DisplayAction {
        self.state = IdlePreviewState::Suppressed;
        self.suppressed_paints = SuppressedPaints::new(now);
        self.next_transition = deadline;
        DisplayAction::Suppress
    }

    fn open_preview(&mut self, now: Instant) -> DisplayAction {
        self.state = IdlePreviewState::Preview;
        self.next_transition = now + PREVIEW_WINDOW;
        self.recent_updates = [None; 2];
        DisplayAction::Allow
    }

    pub fn new(enabled: bool, supported: bool, now: Instant) -> Self {
        let state = match (enabled, supported) {
            (false, _) => IdlePreviewState::Off,
            (true, false) => IdlePreviewState::Unsupported,
            (true, true) => IdlePreviewState::Live,
        };
        Self {
            state,
            automatic_enabled: enabled,
            manual: None,
            suppressed_paints: SuppressedPaints::new(now),
            pause_unavailable_reason: (!supported)
                .then_some("Host does not support display pause."),
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
        if desktop_painted && self.state == IdlePreviewState::Suppressed {
            self.suppressed_paints.record(now);
        }
        if self.manual.is_some() {
            return;
        }
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
        self.manual = None;
        self.state = if self.automatic_enabled {
            IdlePreviewState::Live
        } else {
            IdlePreviewState::Off
        };
        self.next_transition = now + IDLE_DELAY;
        self.recent_updates = [None; 2];
        action
    }

    pub fn on_timer(&mut self, now: Instant) -> Option<DisplayAction> {
        if let Some(manual) = &mut self.manual {
            if self.state == IdlePreviewState::Suppressed && self.suppressed_paints.confirmed(now) {
                self.manual = None;
                self.pause_unavailable_reason = Some("Host continues sending display updates.");
                self.state = if self.automatic_enabled {
                    IdlePreviewState::Unsupported
                } else {
                    IdlePreviewState::Off
                };
                return Some(DisplayAction::Allow);
            }
            if now < self.next_transition {
                return None;
            }
            if self.state == IdlePreviewState::Preview {
                return self.close_manual_preview(now);
            }
            manual.next_preview = now + PREVIEW_PERIOD;
            return Some(self.open_preview(now));
        }
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
                Some(self.suppress_until(now, now + PREVIEW_PERIOD))
            }
            IdlePreviewState::Suppressed => Some(self.open_preview(now)),
            IdlePreviewState::Preview
                if active_desktop && self.cycle_is_cheaper(self.projected_minute_bytes) =>
            {
                self.cycle_bytes = 0;
                Some(self.suppress_until(now, now + (PREVIEW_PERIOD - PREVIEW_WINDOW)))
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
    fn manual_minute_previews_survive_quiet_and_costly_windows_with_auto_off_or_on() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        for auto in [false, true] {
            let mut p = IdlePreview::new(auto, true, start);
            assert_eq!(p.pause_unavailable_reason(), None);
            assert_eq!(p.pause(start), Some(Suppress));
            assert!(p.is_manually_paused());
            assert_eq!(p.state(), Suppressed);
            assert_eq!(p.pause(at(35)), None);
            for minute in [60, 120] {
                assert_eq!(p.on_timer(at(minute - 1)), None);
                assert_eq!(p.on_timer(at(minute)), Some(Allow));
                assert_eq!(p.state(), Preview);
                assert_eq!(p.pause(at(minute)), None);
                if minute == 120 {
                    p.on_graphics(at(minute), 1_000_000);
                    p.on_graphics(at(minute + 1), 1_000_000);
                }
                assert_eq!(p.on_timer(at(minute + 1)), None);
                assert_eq!(p.on_timer(at(minute + 2)), Some(Suppress));
            }
            assert_eq!(p.on_activity(at(123)), Some(Allow));
            assert_eq!(p.on_activity(at(123)), None);
            assert!(!p.is_manually_paused());
            assert_eq!(p.state(), if auto { Live } else { Off });
            p.on_graphics(at(421), 4096);
            p.on_graphics(at(422), 4096);
            assert_eq!(p.on_timer(at(422)), None);
            assert_eq!(p.on_timer(at(423)), auto.then_some(Suppress));
        }
    }

    #[test]
    fn manual_pause_adopts_auto_suppression_or_closes_an_open_auto_preview() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut suppressed = active_but_unattended(start);
        assert_eq!(suppressed.pause(at(320)), None);
        assert!(suppressed.is_manually_paused());
        assert_eq!(suppressed.on_timer(at(360)), Some(Allow));
        assert_eq!(suppressed.on_activity(at(361)), None);
        assert_eq!(suppressed.state(), Live);

        let mut preview = active_but_unattended(start);
        assert_eq!(preview.on_timer(at(360)), Some(Allow));
        assert_eq!(preview.pause(at(361)), Some(Suppress));
        assert_eq!(preview.on_timer(at(362)), None);
        assert_eq!(preview.on_timer(at(420)), None);
        assert_eq!(preview.on_timer(at(421)), Some(Allow));
    }

    #[test]
    fn manual_pause_keeps_observations_from_the_same_uninterrupted_auto_suppression() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut p = active_but_unattended(start);
        p.on_graphics(at(302), 4096);
        p.on_graphics(at(312), 4096);
        assert_eq!(p.pause(at(318)), None);
        p.on_graphics(at(322), 4096);
        assert_eq!(p.on_timer(at(331)), None);
        assert_eq!(p.on_timer(at(332)), Some(Allow));
        assert!(!p.is_manually_paused());
    }

    #[test]
    fn manual_late_timers_coalesce_without_replaying_missed_windows() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut p = IdlePreview::new(false, true, start);
        assert_eq!(p.pause(start), Some(Suppress));
        assert_eq!(p.on_timer(at(212)), Some(Allow));
        assert_eq!(p.on_timer(at(212)), None);
        assert_eq!(p.on_timer(at(215)), Some(Suppress));
        assert_eq!(p.on_timer(at(271)), None);
        assert_eq!(p.on_timer(at(272)), Some(Allow));
        assert_eq!(p.on_timer(at(500)), Some(Suppress));
        assert_eq!(p.on_timer(at(500)), None);
    }

    #[test]
    fn manual_pause_requires_negotiated_support_even_when_auto_is_off() {
        let now = Instant::now();
        for auto in [false, true] {
            let mut p = IdlePreview::new(auto, false, now);
            assert!(p.pause_unavailable_reason().is_some());
            assert_eq!(p.pause(now), None);
            assert!(!p.is_manually_paused());
            assert_eq!(p.state(), if auto { Unsupported } else { Off });
        }
    }

    #[test]
    fn manual_suppression_refusal_requires_paints_in_three_consecutive_full_intervals() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        for auto in [false, true] {
            let mut p = IdlePreview::new(auto, true, start);
            assert_eq!(p.pause(start), Some(Suppress));
            p.on_graphics(at(1), 9000); // In flight during grace.
            p.on_transfer(at(4), 4096, false); // Cursor or channel traffic.
            for second in [12, 14, 21, 22, 29] {
                p.on_graphics(at(second), 4096);
            }
            assert_eq!(p.on_timer(at(32)), None);
            p.on_transfer(at(33), 9999, false); // Break the consecutive streak.
            p.on_graphics(at(42), 4096);
            assert_eq!(p.on_timer(at(52)), None);
            assert_eq!(p.on_timer(at(60)), Some(Allow));
            p.on_graphics(at(61), 4096); // Requested preview.
            assert_eq!(p.on_timer(at(62)), Some(Suppress));
            for second in [63, 64, 74, 84, 85] {
                p.on_graphics(at(second), 4096);
            }
            assert_eq!(p.on_timer(at(93)), None);
            assert_eq!(p.on_timer(at(94)), Some(Allow));
            assert_eq!(
                p.pause_unavailable_reason(),
                Some("Host continues sending display updates.")
            );
            assert!(!p.is_manually_paused());
            assert_eq!(p.pause(at(95)), None);
            assert_eq!(p.on_timer(at(122)), None);
            assert_eq!(p.state(), if auto { Unsupported } else { Off });
            let fresh = IdlePreview::new(auto, true, at(124));
            assert_eq!(fresh.pause_unavailable_reason(), None);
            assert!(!fresh.is_manually_paused());
            assert_eq!(fresh.state(), if auto { Live } else { Off });
        }
    }

    #[test]
    fn manual_resize_redraws_reset_observation_and_preserve_the_minute_deadline() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut p = IdlePreview::new(false, true, start);
        assert_eq!(p.pause(start), Some(Suppress));
        p.on_graphics(at(3), 4096);
        p.on_graphics(at(13), 4096);
        assert_eq!(p.begin_manual_redraw(at(22)), Some(Allow));
        assert_eq!(p.begin_manual_redraw(at(23)), None);
        p.on_graphics(at(23), 8000);
        assert_eq!(p.finish_manual_redraw(at(23)), Some(Suppress));
        assert_eq!(p.finish_manual_redraw(at(23)), None);
        p.on_graphics(at(24), 4096); // In flight after the redraw.
        assert_eq!(p.on_timer(at(33)), None);
        assert_eq!(p.on_timer(at(60)), Some(Allow));
        assert_eq!(p.begin_manual_redraw(at(61)), None);
        assert_eq!(p.on_timer(at(62)), Some(Suppress));
        assert!(p.is_manually_paused());
        assert_eq!(p.on_timer(at(120)), Some(Allow));
        assert_eq!(p.finish_manual_redraw(at(120)), None); // Ordinary preview.
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
