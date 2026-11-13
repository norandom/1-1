//! Test-support-only E2E commands (task 5.3, Req 12.2/12.7/12.10).
//!
//! Compiled ONLY in `test-support` builds. Production builds do not register
//! these commands (cfg-gated match arms in `lib.rs`), so they cannot be
//! invoked there (Req 12.10).
//!
//! - Seeding creates synthetic saved connections (mock-flagged, reserved
//!   `.invalid` hosts, file-only): no network anywhere on the path.
//! - The loss control lets the E2E harness drive the shared reconnect loop of
//!   a synthetic session; cancellation reuses the production
//!   `cancel_reconnect` command.
//! - Credential cleanup is scoped to the harness namespace and refuses to run
//!   against a production store.

use tauri::State;

use crate::rdp::client::SessionCommand;
use crate::rdp::frame_spy;
use crate::rdp::session::SessionManager;
use crate::store::connections::{ConnectionConfig, ConnectionInput, ConnectionStore};
use crate::store::credentials::CredentialStore;

/// Seed `count` synthetic saved connections named `{prefix} 1..=count`,
/// persisted through `store`. Reserved `.invalid` hosts keep the fixtures
/// off the network even if a future bug routed one to the real connector.
pub async fn seed_mock_connections(
    store: &ConnectionStore,
    count: u32,
    name_prefix: Option<&str>,
) -> Result<Vec<ConnectionConfig>, String> {
    if count == 0 {
        return Err("Seed count must be at least 1".to_string());
    }
    if count > 100 {
        return Err("Seed count must be at most 100".to_string());
    }
    let prefix = name_prefix.unwrap_or("Drift E2E Mock");
    let mut seeded = Vec::with_capacity(count as usize);
    for i in 1..=count {
        let input = ConnectionInput {
            name: format!("{prefix} {i}"),
            // RFC 2606 reserved TLD: no DNS resolver will ever resolve this,
            // so seeding (and any accidental real-path dialing) stays offline.
            host: "drift-e2e-mock.invalid".to_string(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: vec!["e2e-mock".to_string()],
            display_width: Some(64),
            display_height: Some(64),
            mock: true,
            password: None,
        };
        seeded.push(store.create(input).await?);
    }
    log::info!("Seeded {} synthetic mock connections", seeded.len());
    Ok(seeded)
}

/// E2E fixture seeding: create synthetic saved connections (mock-flagged).
#[tauri::command]
pub async fn debug_seed_mock_connections(
    count: Option<u32>,
    name_prefix: Option<String>,
    conn_store: State<'_, ConnectionStore>,
) -> Result<Vec<ConnectionConfig>, String> {
    seed_mock_connections(&conn_store, count.unwrap_or(1), name_prefix.as_deref()).await
}

/// E2E loss control: force a synthetic session's connection to drop so the
/// shared reconnect loop runs. No-op (logged) on real sessions.
#[tauri::command]
pub async fn debug_drop_mock_connection(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let cmd_tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;

    cmd_tx
        .send(SessionCommand::DropConnection)
        .await
        .map_err(|_| format!("Session '{}' actor is no longer running", session_id))?;

    log::info!("Loss control requested for session {}", session_id);
    Ok(())
}

/// E2E scoped cleanup: delete only harness-namespaced credentials. The
/// command goes through the managed store, which the harness wired to its
/// namespace (see `crate::harness`); it refuses on a production store.
#[tauri::command]
pub fn debug_cleanup_test_credentials(
    connection_ids: Vec<String>,
    cred_store: State<'_, CredentialStore>,
) -> Result<usize, String> {
    cred_store.delete_test_credentials(&connection_ids)
}

/// E2E frame spy (task 5.4, Req 12.4): cumulative counts of actually
/// emitted packet kinds per session. `None` snapshots every counted
/// session; a specific id scopes to it. Empty while the spy is inactive or
/// nothing was recorded yet — the spy itself is activated by the
/// `frame_spy::FRAME_SPY_ENV` env var inside test-support builds only
/// (Req 12.10).
#[tauri::command]
pub fn debug_frame_spy_snapshot(session_id: Option<String>) -> Vec<frame_spy::FrameSpySnapshot> {
    match session_id.as_deref() {
        Some(id) => frame_spy::snapshot(id).into_iter().collect(),
        None => frame_spy::snapshot_all(),
    }
}

/// E2E frame spy reset (task 5.4): clear every session's spy counters so
/// test phases start from a known tally.
#[tauri::command]
pub fn debug_frame_spy_reset() {
    frame_spy::reset();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_seed_persists_synthetic_mock_saved_connections() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());

        let seeded = seed_mock_connections(&store, 2, Some("E2E")).await.unwrap();
        assert_eq!(seeded.len(), 2);
        assert!(
            seeded.iter().all(|c| c.mock),
            "seeded connections are synthetic (mock) saved connections"
        );
        assert!(
            seeded.iter().all(|c| c.host.ends_with(".invalid")),
            "hosts are reserved .invalid names: no network even if misrouted"
        );

        let names: std::collections::HashSet<&str> =
            seeded.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names.len(), 2, "deterministic unique names");
        let ids: std::collections::HashSet<&str> = seeded.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids.len(), 2, "fresh unique ids");

        // Persisted for the harness's isolated namespace (file-only, no network).
        let reloaded = ConnectionStore::new(dir.path().to_path_buf());
        for conn in &seeded {
            assert!(
                reloaded.get(&conn.id).await.is_some(),
                "seeded connection {} persisted",
                conn.id
            );
        }
    }

    #[tokio::test]
    async fn test_seed_defaults_prefix_and_rejects_runaway_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());

        let one = seed_mock_connections(&store, 1, None).await.unwrap();
        assert!(
            one[0].name.starts_with("Drift E2E Mock"),
            "deterministic default prefix, got {}",
            one[0].name
        );

        assert!(
            seed_mock_connections(&store, 0, None).await.is_err(),
            "zero-count seeding is a harness bug"
        );
        assert!(
            seed_mock_connections(&store, 101, None).await.is_err(),
            "runaway seeding is rejected"
        );
    }

    // ---- Task 5.4 (Req 12.4/12.10): frame-spy command surface. ----

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
    fn test_frame_spy_snapshot_command_scopes_and_resets() {
        let _g = EnvVarGuard::set(frame_spy::FRAME_SPY_ENV, "1");
        frame_spy::reset();
        frame_spy::record_emission("cmd-a", 1);
        frame_spy::record_emission("cmd-a", 2);
        frame_spy::record_emission("cmd-b", 1);

        let all = debug_frame_spy_snapshot(None);
        assert_eq!(all.len(), 2, "no scope: every counted session");
        assert_eq!(all[0].session_id, "cmd-a");

        let scoped = debug_frame_spy_snapshot(Some("cmd-a".to_string()));
        assert_eq!(scoped.len(), 1, "scoped snapshot returns one session");
        assert_eq!(scoped[0].total_packets, 2);
        let mut kinds = std::collections::BTreeMap::new();
        kinds.insert(1u8, 1u64);
        kinds.insert(2u8, 1u64);
        assert_eq!(scoped[0].counts_by_kind, kinds);

        assert!(
            debug_frame_spy_snapshot(Some("missing".to_string())).is_empty(),
            "unknown sessions yield an empty snapshot, not an error"
        );

        debug_frame_spy_reset();
        assert!(
            debug_frame_spy_snapshot(None).is_empty(),
            "reset clears every session's counters"
        );
    }

    #[test]
    fn test_frame_spy_commands_inert_when_spy_inactive() {
        let _g = EnvVarGuard::unset(frame_spy::FRAME_SPY_ENV);
        frame_spy::reset();
        assert!(
            frame_spy::record_emission("cmd-off", 1).is_none(),
            "inactive spy records nothing"
        );
        assert!(
            debug_frame_spy_snapshot(Some("cmd-off".to_string())).is_empty()
                && debug_frame_spy_snapshot(None).is_empty(),
            "snapshot commands report nothing while the spy is inactive"
        );
    }
}
