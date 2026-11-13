//! Build-gated frame-kind spy (task 5.4, Req 12.4/12.10).
//!
//! Compiled ONLY in `test-support` builds (see `mod.rs`): production builds
//! do not contain this module, its environment variable, its events or its
//! commands at all (Req 12.10).
//!
//! The spy counts packet kinds at the actor's single frame-emission funnel
//! AFTER the frame channel accepts the packet — i.e. what actually crossed
//! toward the renderer, not what a frame loop intended to send. The E2E
//! harness activates it with [`FRAME_SPY_ENV`] `=1` (exact value) and
//! observes cumulative per-session counts through the `frame-spy-{session}`
//! Tauri event and the `debug_frame_spy_snapshot`/`debug_frame_spy_reset`
//! test commands, asserting the Requirement 9 bitmap-only wire policy
//! (kinds 1/2 exclusively; any stray kind is counted and surfaced, never
//! hidden). Every payload carries a computed `onlyBitmapKinds` boolean so
//! the policy assertion is self-contained for the harness.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::rdp::frame_transport::{PACKET_KIND_DIRTY_RECTS, PACKET_KIND_FULL_FRAME};

/// Environment variable activating the spy in test-support builds. Must be
/// set to exactly `1`. Consulted only by code in this module, which is
/// compiled out of production builds entirely (Req 12.10).
pub const FRAME_SPY_ENV: &str = "DRIFT_TEST_FRAME_SPY";

/// Prefix of the per-session spy event (`frame-spy-{session_id}`), following
/// the existing `session-info-{id}` / `clipboard-{id}` event naming.
pub const FRAME_SPY_EVENT_PREFIX: &str = "frame-spy-";

/// The bitmap packet kinds allowed on the UI frame transport (Req 9.4/12.4).
/// Pinned to the frame-transport wire constants; AVC or any other kind is a
/// policy violation the spy must surface.
pub const BITMAP_PACKET_KINDS: [u8; 2] = [PACKET_KIND_FULL_FRAME, PACKET_KIND_DIRTY_RECTS];

/// Cumulative per-session spy counts (the payload of every spy event and the
/// return shape of the snapshot command).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameSpySnapshot {
    pub session_id: String,
    /// Count per actually-emitted packet kind. Every observed kind appears,
    /// so a stray non-bitmap kind is visible, not filtered out.
    pub counts_by_kind: BTreeMap<u8, u64>,
    pub total_packets: u64,
    /// True when every observed kind is a bitmap kind (1/2) — the Req
    /// 9/12.4 assertion the mock E2E makes, computed at construction so
    /// every payload self-asserts the wire policy for the harness.
    pub only_bitmap_kinds: bool,
}

/// Spy event name for a session (`frame-spy-{session_id}`).
pub fn spy_event_name(session_id: &str) -> String {
    format!("{FRAME_SPY_EVENT_PREFIX}{session_id}")
}

/// Spy activation: exactly `1`. Re-read per call so harness runs control it
/// without restarts; the read only exists in test-support builds.
fn activated() -> bool {
    std::env::var(FRAME_SPY_ENV).as_deref() == Ok("1")
}

/// Per-session kind → count registry (BTreeMaps for deterministic order).
type Registry = BTreeMap<String, BTreeMap<u8, u64>>;

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn snapshot_of(session_id: &str, counts: &BTreeMap<u8, u64>) -> FrameSpySnapshot {
    FrameSpySnapshot {
        session_id: session_id.to_string(),
        // Req 9/12.4: computed from the observed kinds at construction —
        // every event/command payload self-asserts the bitmap-only policy.
        only_bitmap_kinds: counts.keys().all(|kind| BITMAP_PACKET_KINDS.contains(kind)),
        total_packets: counts.values().sum(),
        counts_by_kind: counts.clone(),
    }
}

/// Record one actually-emitted packet kind for a session. Returns the
/// cumulative snapshot (for the spy event) when the spy is active, `None`
/// when inactive — the actor hook then does nothing.
pub fn record_emission(session_id: &str, kind: u8) -> Option<FrameSpySnapshot> {
    if !activated() {
        return None;
    }
    let mut registry = registry().lock().unwrap();
    let counts = registry.entry(session_id.to_string()).or_default();
    *counts.entry(kind).or_insert(0) += 1;
    Some(snapshot_of(session_id, counts))
}

/// Cumulative snapshot for one session; `None` when nothing was recorded
/// (inactive spy or no emitted frames yet).
pub fn snapshot(session_id: &str) -> Option<FrameSpySnapshot> {
    let registry = registry().lock().unwrap();
    registry
        .get(session_id)
        .map(|counts| snapshot_of(session_id, counts))
}

/// Snapshots of every counted session, ordered by session id.
pub fn snapshot_all() -> Vec<FrameSpySnapshot> {
    let registry = registry().lock().unwrap();
    registry
        .iter()
        .map(|(session_id, counts)| snapshot_of(session_id, counts))
        .collect()
}

/// Clear all recorded counters (E2E phase isolation).
pub fn reset() {
    registry().lock().unwrap().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes the variable on scope exit (including panics) so
    /// shared-process test runs never inherit spy settings.
    struct EnvVarGuard(&'static str);
    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            unsafe { std::env::set_var(key, value) };
            EnvVarGuard(key)
        }
        fn unset(key: &'static str) -> Self {
            unsafe { std::env::remove_var(key) };
            EnvVarGuard(key)
        }
    }
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(self.0) };
        }
    }

    #[test]
    fn test_activation_requires_exact_env_value() {
        let _a = EnvVarGuard::unset(FRAME_SPY_ENV);
        reset();
        assert!(
            record_emission("spy-activation", PACKET_KIND_FULL_FRAME).is_none(),
            "unset env: spy stays inactive"
        );
        assert!(snapshot("spy-activation").is_none());

        for inert in ["0", "true", " 1", "yes", " "] {
            let _g = EnvVarGuard::set(FRAME_SPY_ENV, inert);
            assert!(
                record_emission("spy-activation", PACKET_KIND_FULL_FRAME).is_none(),
                "value {inert:?} must not activate the spy"
            );
        }

        let _g = EnvVarGuard::set(FRAME_SPY_ENV, "1");
        assert!(
            record_emission("spy-activation", PACKET_KIND_FULL_FRAME).is_some(),
            "exactly \"1\" activates the spy"
        );
    }

    #[test]
    fn test_counts_accumulate_per_kind_and_session() {
        let _g = EnvVarGuard::set(FRAME_SPY_ENV, "1");
        reset();

        record_emission("s-a", PACKET_KIND_FULL_FRAME);
        let after_second = record_emission("s-a", PACKET_KIND_FULL_FRAME).unwrap();
        record_emission("s-a", PACKET_KIND_DIRTY_RECTS);

        let got = snapshot("s-a").expect("counts exist for s-a");
        let mut expected = BTreeMap::new();
        expected.insert(PACKET_KIND_FULL_FRAME, 2u64);
        expected.insert(PACKET_KIND_DIRTY_RECTS, 1u64);
        assert_eq!(got.counts_by_kind, expected, "per-kind counts accumulate");
        assert_eq!(got.total_packets, 3);
        assert_eq!(
            after_second.total_packets, 2,
            "returned snapshot is cumulative"
        );

        // Sessions are isolated from each other.
        record_emission("s-b", PACKET_KIND_FULL_FRAME);
        let other = snapshot("s-b").expect("counts exist for s-b");
        assert_eq!(other.total_packets, 1);
        assert_eq!(
            other.counts_by_kind.keys().copied().collect::<Vec<_>>(),
            vec![1]
        );
        assert_eq!(
            got.counts_by_kind, expected,
            "s-b records must not leak into s-a"
        );

        let all = snapshot_all();
        assert_eq!(all.len(), 2, "both sessions listed");
        assert_eq!(all[0].session_id, "s-a", "deterministic session ordering");
    }

    #[test]
    fn test_only_bitmap_kinds_surfaces_stray_non_bitmap_kind() {
        let _g = EnvVarGuard::set(FRAME_SPY_ENV, "1");
        reset();

        record_emission("s-kinds", PACKET_KIND_FULL_FRAME);
        record_emission("s-kinds", PACKET_KIND_FULL_FRAME);
        record_emission("s-kinds", PACKET_KIND_DIRTY_RECTS);
        let bitmap_only = snapshot("s-kinds").unwrap();
        assert!(bitmap_only.only_bitmap_kinds, "kinds 1/2 are bitmap-only");
        assert_eq!(
            bitmap_only
                .counts_by_kind
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![1, 2]
        );

        // A stray non-bitmap kind (the AVC scenario Req 12.4 guards against)
        // must be COUNTED and visible — never hidden — and flip the predicate.
        record_emission("s-kinds", 3);
        let with_stray = snapshot("s-kinds").unwrap();
        assert!(!with_stray.only_bitmap_kinds, "kind 3 is not a bitmap kind");
        assert_eq!(with_stray.counts_by_kind.get(&3), Some(&1));
    }

    /// Task 5.4 remediation: the snapshot payload self-asserts the Req
    /// 9/12.4 wire policy — `onlyBitmapKinds` is computed from the observed
    /// kinds at construction time, so the E2E harness reads one boolean
    /// instead of re-deriving the check from the kind map.
    #[test]
    fn test_snapshot_payload_serializes_only_bitmap_kinds_field() {
        let _g = EnvVarGuard::set(FRAME_SPY_ENV, "1");
        reset();

        record_emission("s-payload", PACKET_KIND_FULL_FRAME);
        record_emission("s-payload", PACKET_KIND_DIRTY_RECTS);
        let payload = serde_json::to_value(snapshot("s-payload").unwrap()).unwrap();
        assert_eq!(
            payload["onlyBitmapKinds"],
            serde_json::json!(true),
            "bitmap kinds serialize a true self-assertion: {payload}"
        );
        assert_eq!(payload["sessionId"], serde_json::json!("s-payload"));
        assert_eq!(payload["totalPackets"], serde_json::json!(2));

        // A stray kind flips the serialized field — the payload can never
        // claim bitmap-only when a non-bitmap kind was observed.
        record_emission("s-payload", 3);
        let payload = serde_json::to_value(snapshot("s-payload").unwrap()).unwrap();
        assert_eq!(
            payload["onlyBitmapKinds"],
            serde_json::json!(false),
            "stray kind flips the serialized self-assertion: {payload}"
        );
    }

    #[test]
    fn test_reset_clears_all_sessions() {
        let _g = EnvVarGuard::set(FRAME_SPY_ENV, "1");
        reset();
        record_emission("s-reset", PACKET_KIND_FULL_FRAME);
        assert!(snapshot("s-reset").is_some());

        reset();
        assert!(snapshot("s-reset").is_none());
        assert!(snapshot_all().is_empty());
    }

    #[test]
    fn test_event_name_uses_design_prefix() {
        assert_eq!(spy_event_name("abc"), "frame-spy-abc");
    }

    #[test]
    fn test_bitmap_kind_set_pins_wire_constants() {
        assert_eq!(
            BITMAP_PACKET_KINDS,
            [PACKET_KIND_FULL_FRAME, PACKET_KIND_DIRTY_RECTS]
        );
        assert_eq!(BITMAP_PACKET_KINDS, [1, 2], "wire constants must not drift");
    }
}
