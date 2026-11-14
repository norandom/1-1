//! Task 7.3 (Req 1.5, 6.2, 10.2, 10.17, 10.18; design "Destructive
//! actions and identity"): the coordination layer that makes
//! connection/library deletion safe against live session actors.
//!
//! Deletion is a cross-subsystem pipeline — session stop, store
//! transaction, keyring cleanup — and this module owns its exact order:
//!
//! 1. serialize against Connect for the affected connection IDs
//!    ([`SessionManager::stop_sessions_for_connections`] tombstones the
//!    IDs under the task 7.1 admission lock, so a concurrent Connect can
//!    never admit or resurrect an actor for a connection being deleted);
//! 2. cancel reconnect/backoff retries and await actor stop with a
//!    shared five-second bound — a stop timeout ABORTS the deletion,
//!    keeps already-stopped sessions stopped and retains the saved data;
//! 3. atomically persist the deletion in ONE store transaction — a
//!    persist failure also retains the saved data, un-tombstones the
//!    survivors and reports that sessions were stopped;
//! 4. only then clean keyring entries, best-effort: a failed cleanup
//!    still reports a successful deletion and lists the orphaned
//!    credential IDs for explicit retry (never as deleted records).
//!
//! The `#[tauri::command]` marshalers in `commands::connection`
//! (`delete_connection`) and `commands::library` (`delete_library`) stay
//! thin: they forward to [`delete_connection_coordinated`] /
//! [`delete_library_coordinated`] and surface the exact
//! [`DeletionReport`] counts the 10.18 confirmation states.

use crate::rdp::session::SessionManager;
use crate::store::credentials::CredentialStore;
use crate::store::library::{CascadedIds, LibraryStore, LibraryStoreError};

/// Exact outcome of a coordinated deletion — the backend half of the
/// Req 10.18 confirmation counts. `cascaded` states the removed
/// folders/connections (and moved ones, once a destructive operation
/// re-files anything), `stopped_session_ids` lists the sessions this
/// deletion stopped, and `orphaned_credential_ids` lists credentials
/// whose keyring cleanup failed AFTER a successful persist: they are
/// reported for an explicit retry and must never be mistaken for
/// deleted connection records (the store record is already gone).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionReport {
    /// Whether the store transaction removed the target. `false` only
    /// for the v1 connection contract (unknown id — a no-op).
    pub deleted: bool,
    /// Exact folder/connection counts and IDs of the committed
    /// transaction ([`CascadedIds`]).
    pub cascaded: CascadedIds,
    /// Sessions that were live and were stopped by this deletion, in
    /// stop order (Req 10.18 "sessions stopped").
    pub stopped_session_ids: Vec<String>,
    /// Connection IDs whose keyring entry survived a successful
    /// deletion because best-effort cleanup failed (Req 1.5/6.2):
    /// orphaned credentials reported for explicit retry.
    pub orphaned_credential_ids: Vec<String>,
}

impl DeletionReport {
    fn noop() -> Self {
        Self {
            deleted: false,
            cascaded: CascadedIds {
                deleted_folder_ids: Vec::new(),
                deleted_connection_ids: Vec::new(),
                moved_connection_ids: Vec::new(),
            },
            stopped_session_ids: Vec::new(),
            orphaned_credential_ids: Vec::new(),
        }
    }
}

/// Surface a typed store error as the command-layer string (existing
/// command style, design L358).
fn to_message(error: LibraryStoreError) -> String {
    error.to_string()
}

/// The stop-phase failure (Req 10.17): the bounded stop missed at least
/// one actor, so the deletion aborts with the saved data retained.
fn stop_timeout_error(offenders: &[String]) -> String {
    format!(
        "deletion aborted: session(s) [{}] did not stop within five seconds; the saved data \
         was retained (sessions that already stopped remain stopped)",
        offenders.join(", ")
    )
}

/// Best-effort keyring cleanup after a committed deletion (Req 1.5/6.2).
/// Failures are logged and returned as orphaned IDs — the deletion
/// itself already succeeded and must not be reported as failed.
fn cleanup_credentials(
    cred_store: &CredentialStore,
    deleted_connection_ids: &[String],
) -> Vec<String> {
    let mut orphaned = Vec::new();
    for id in deleted_connection_ids {
        if let Err(error) = cred_store.delete_password(id) {
            log::warn!(
                "Keyring cleanup for deleted connection {id} failed ({error}); the credential \
                 is orphaned and reported for an explicit retry"
            );
            orphaned.push(id.clone());
        }
    }
    orphaned
}

/// Req 1.5/6.2/10.17/10.18 (task 7.3): coordinate connection deletion —
/// stop, persist, then clean the credential — for the
/// `delete_connection` command. The v1 unknown-id contract (a no-op
/// that removes nothing) is preserved as `deleted: false`.
pub(crate) async fn delete_connection_coordinated(
    id: &str,
    store: &LibraryStore,
    cred_store: &CredentialStore,
    sessions: &SessionManager,
) -> Result<DeletionReport, String> {
    let blocked = [id.to_string()];
    // v1 contract: deleting an unknown id is a no-op (stops nothing,
    // persists nothing, cleans nothing).
    if store.get_connection(id).map_err(to_message)?.is_none() {
        return Ok(DeletionReport::noop());
    }

    // Serialize against Connect (tombstones under the admission lock),
    // cancel retries and await bounded actor stop. A stop timeout
    // aborts before any store mutation: saved data retained.
    let stopped = sessions
        .stop_sessions_for_connections(&blocked)
        .await
        .map_err(|offenders| stop_timeout_error(&offenders))?;

    // Atomic store deletion. Failure retains the snapshot and file; the
    // error reports that sessions were stopped (they cannot be
    // restarted by this operation) and the tombstones roll back so the
    // surviving connection is connectable again.
    let removed = match store.delete_connection(id) {
        Ok(removed) => removed,
        Err(error) => {
            sessions.rollback_connection_deletions(&blocked).await;
            return Err(format!(
                "{error}; sessions were stopped ({}) but the saved connection was retained",
                stopped.len()
            ));
        }
    };

    // The transaction committed: the connection no longer exists even
    // when this request lost a delete/delete race (`Ok(false)` —
    // another request's commit removed it), so the tombstone stays.

    let cascaded = if removed {
        CascadedIds {
            deleted_folder_ids: Vec::new(),
            deleted_connection_ids: vec![id.to_string()],
            moved_connection_ids: Vec::new(),
        }
    } else {
        CascadedIds {
            deleted_folder_ids: Vec::new(),
            deleted_connection_ids: Vec::new(),
            moved_connection_ids: Vec::new(),
        }
    };
    let orphaned = if removed {
        cleanup_credentials(cred_store, &[id.to_string()])
    } else {
        Vec::new()
    };
    Ok(DeletionReport {
        deleted: removed,
        cascaded,
        stopped_session_ids: stopped,
        orphaned_credential_ids: orphaned,
    })
}

/// Req 10.2/10.17/10.18 (task 7.3): coordinate library deletion for the
/// `delete_library` command — preview the affected connections, stop
/// and tombstone them, cascade the whole library in ONE store
/// transaction, settle the tombstones against what the transaction
/// actually deleted, then clean every deleted connection's credential
/// best-effort.
pub(crate) async fn delete_library_coordinated(
    id: &str,
    store: &LibraryStore,
    cred_store: &CredentialStore,
    sessions: &SessionManager,
) -> Result<DeletionReport, String> {
    // Preview: the typed store rejection for unknown libraries happens
    // before anything is stopped or deleted.
    let blocked = store.library_connection_ids(id).map_err(to_message)?;

    let mut stopped = sessions
        .stop_sessions_for_connections(&blocked)
        .await
        .map_err(|offenders| stop_timeout_error(&offenders))?;

    let cascaded = match store.delete_library(id) {
        Ok(cascaded) => cascaded,
        Err(error) => {
            sessions.rollback_connection_deletions(&blocked).await;
            return Err(format!(
                "{error}; sessions were stopped ({}) but the saved library was retained",
                stopped.len()
            ));
        }
    };

    // The committed transaction is authoritative. Connections that
    // joined the library between preview and commit were deleted by it
    // without being previewed: stop any session admitted for them in
    // that window too (their tombstones from this stop are permanent —
    // the connections are gone). A timeout here cannot abort the
    // committed deletion: the tombstones SURVIVE (a stale-config
    // Connect may never resurrect a committed deletion), the actor
    // stays tracked, and the sessions that did stop before the bound
    // are still reported (Req 10.18).
    let extras: Vec<String> = cascaded
        .deleted_connection_ids
        .iter()
        .filter(|deleted| !blocked.contains(deleted))
        .cloned()
        .collect();
    if !extras.is_empty() {
        match sessions
            .stop_sessions_for_committed_connections(&extras)
            .await
        {
            Ok(mut more) => stopped.append(&mut more),
            Err(timed_out) => {
                // Review round 1: no tombstone rollback and no
                // discarded session ids — the commit already deleted
                // every extra, and the extras that stopped inside the
                // bound still count in the report.
                stopped.extend(timed_out.stopped);
                log::warn!(
                    "Post-commit stop timed out for sessions {:?} of deleted connections \
                     {extras:?}; the deletion stays committed and the connections stay \
                     tombstoned",
                    timed_out.offenders
                );
            }
        }
    }

    // Keep tombstones only for connections the transaction deleted;
    // blocked-but-surviving ids (moved out of the library mid-deletion)
    // are released and connectable again.
    sessions
        .finalize_connection_deletions(&blocked, &cascaded.deleted_connection_ids)
        .await;

    let orphaned = cleanup_credentials(cred_store, &cascaded.deleted_connection_ids);
    Ok(DeletionReport {
        deleted: true,
        cascaded,
        stopped_session_ids: stopped,
        orphaned_credential_ids: orphaned,
    })
}

#[cfg(test)]
mod tests {
    //! Task 7.3 integration suite: the real [`LibraryStore`] on a temp
    //! file, the real [`SessionManager`] with scripted connectors (a
    //! mock-loop session is live `Connected` and stops on `Disconnect`;
    //! a never-resolving connector pins live `Connecting` past the
    //! five-second stop bound; a failing frame transport with
    //! auto-reconnect pins live `Reconnecting` backoff) and the scripted
    //! keyring from `store::credentials::tests`. Everything is
    //! deterministic: the tokio test-util paused clock drives the stop
    //! bound, there is no network I/O, no wall-clock sleep and no
    //! user-data access (Req 12.7).

    use super::*;
    use crate::rdp::client::ConnectionOutcome;
    use crate::rdp::session::{
        ConnectFn, RecordedEvents, SessionEventSink, SessionManager, SessionState,
    };
    use crate::store::connections::ConnectionConfig;
    use crate::store::credentials::tests::{BuilderGuard, Fail, install_scripted};
    use crate::store::library::{Folder, Library, LibraryDocument, LibraryStore};
    use std::sync::{Arc, Mutex, MutexGuard};
    use tauri::ipc::InvokeResponseBody;

    /// The process-wide scripted-keyring guards must outlive every
    /// `CredentialStore` used while they are held.
    type KeyringGuards = (MutexGuard<'static, ()>, BuilderGuard);

    fn config(id: &str, library_id: &str, folder_id: Option<&str>, name: &str) -> ConnectionConfig {
        let now = chrono::Utc::now();
        ConnectionConfig {
            id: id.to_string(),
            name: name.to_string(),
            host: format!("{name}.invalid"),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            mock: false,
            library_id: library_id.to_string(),
            folder_id: folder_id.map(str::to_string),
            created_at: now,
            updated_at: now,
            last_connected_at: None,
        }
    }

    /// Deterministic, valid UUID v4 connection ids: document validation
    /// rejects non-UUID entity ids, so fixtures may not use free-form
    /// strings (unknown-id lookups can, they never enter the document).
    fn cid(seed: u32) -> String {
        format!("{seed:08x}-0000-4000-8000-000000000000")
    }

    /// Scripted connector that falls back to the mock frame loop: the
    /// actor reaches its live `Connected` state and stops promptly on a
    /// `Disconnect` command (no network, Req 12.7).
    fn mock_loop_connect() -> ConnectFn {
        Arc::new(
            |_config, _password: Option<String>, _width: u32, _height: u32| {
                let outcome: std::pin::Pin<
                    Box<dyn std::future::Future<Output = ConnectionOutcome> + Send>,
                > = Box::pin(async {
                    ConnectionOutcome::Failed("scripted: fall back to the mock loop".to_string())
                });
                outcome
            },
        )
    }

    /// Scripted connector that never resolves: the actor stays in its
    /// live `Connecting` state forever — past the five-second stop
    /// bound — which pins the stop-timeout abort path.
    fn never_resolving_connect() -> ConnectFn {
        Arc::new(
            |_config, _password: Option<String>, _width: u32, _height: u32| {
                let pending: std::pin::Pin<
                    Box<dyn std::future::Future<Output = ConnectionOutcome> + Send>,
                > = Box::pin(std::future::pending());
                pending
            },
        )
    }

    async fn admit(
        manager: &SessionManager,
        config: &crate::store::connections::ConnectionConfig,
        auto_reconnect: bool,
        connect: ConnectFn,
        fail_transport: bool,
    ) -> Result<String, String> {
        let (events, _events_rx) = RecordedEvents::new();
        let frame = if fail_transport {
            tauri::ipc::Channel::new(|_: InvokeResponseBody| {
                Err(tauri::Error::AssetNotFound(
                    "scripted frame-transport failure".to_string(),
                ))
            })
        } else {
            tauri::ipc::Channel::new(|_: InvokeResponseBody| Ok(()))
        };
        manager
            .create_session_with_transport(
                config.clone(),
                None,
                frame,
                SessionEventSink::recorded(events),
                auto_reconnect,
                None,
                connect,
            )
            .await
    }

    async fn live_session(
        manager: &SessionManager,
        config: &crate::store::connections::ConnectionConfig,
        connect: ConnectFn,
    ) -> String {
        admit(manager, config, false, connect, false)
            .await
            .expect("scripted session creation should succeed")
    }

    /// Await a session status without sleeping: the actor publishes
    /// through its watch channel and every poll here yields, so the
    /// paused clock only advances when both tasks are parked.
    async fn wait_for_status(
        manager: &SessionManager,
        session_id: &str,
        pred: impl Fn(&SessionState) -> bool,
    ) {
        for _ in 0..10_000 {
            if let Some(info) = manager.get_session_info(session_id).await
                && pred(&info.status)
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("status predicate was never satisfied for session {session_id}");
    }

    /// Install the process-wide scripted keyring, optionally failing
    /// every `delete_password` (the orphan-cleanup path).
    fn seeded_keyring(fail_delete: Option<Fail>) -> (CredentialStore, KeyringGuards) {
        let guards = install_scripted(
            Arc::new(Mutex::new(Vec::new())),
            false,
            None,
            None,
            fail_delete,
        );
        (CredentialStore::new(), guards)
    }

    fn opened_store(dir: &tempfile::TempDir) -> LibraryStore {
        LibraryStore::open(dir.path().join("connections.json"))
    }

    fn reopened_store(dir: &tempfile::TempDir) -> LibraryStore {
        opened_store(dir)
    }

    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    fn connection_ids(store: &LibraryStore) -> Vec<String> {
        sorted(
            store
                .document()
                .expect("reopened store keeps a valid ready snapshot")
                .connections
                .into_iter()
                .map(|connection| connection.id)
                .collect(),
        )
    }

    // ---- 1. Happy path (Req 1.5, 10.17, 10.18) -------------------------

    #[tokio::test(start_paused = true)]
    async fn live_connected_session_stops_then_deletion_persists_and_cleans_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let conn_id = cid(1);
        let cfg = config(&conn_id, &work.id, None, "Happy");
        store
            .save(LibraryDocument::new(
                vec![work],
                Vec::new(),
                vec![cfg.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&conn_id, "s3cret").unwrap();

        let manager = SessionManager::new();
        let session = live_session(&manager, &cfg, mock_loop_connect()).await;
        wait_for_status(&manager, &session, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;

        let report = delete_connection_coordinated(&conn_id, &store, &creds, &manager)
            .await
            .expect("coordinated connection deletion should succeed");
        assert!(report.deleted);
        assert_eq!(
            report.cascaded,
            CascadedIds {
                deleted_folder_ids: Vec::new(),
                deleted_connection_ids: vec![conn_id.clone()],
                moved_connection_ids: Vec::new(),
            }
        );
        assert_eq!(report.stopped_session_ids, vec![session.clone()]);
        assert!(
            report.orphaned_credential_ids.is_empty(),
            "successful cleanup reports no orphans"
        );

        // The stopped session's entry is gone (Req 10.17: stop before
        // committing the deletion).
        assert!(manager.get_session_info(&session).await.is_none());
        assert!(manager.list_sessions().await.is_empty());

        // Snapshot and on-disk file both dropped the connection; the
        // reopened store stays valid (restart validity).
        assert!(store.get_connection(&conn_id).unwrap().is_none());
        assert_eq!(connection_ids(&reopened_store(&dir)), Vec::<String>::new());

        // Keyring cleaned only after the successful persistence
        // (Req 1.5/6.2).
        assert_eq!(creds.get_password(&conn_id).unwrap(), None);

        // A post-deletion Connect cannot resurrect the connection.
        let error = admit(&manager, &cfg, false, mock_loop_connect(), false)
            .await
            .expect_err("admission must fail for a deleted connection");
        assert!(
            error.contains("was deleted"),
            "admission error should name the deletion: {error}"
        );
    }

    // ---- 2. Stop timeout aborts (Req 10.17) ----------------------------

    #[tokio::test(start_paused = true)]
    async fn stop_timeout_aborts_deletion_and_retains_saved_data_and_credential() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let conn_id = cid(2);
        let cfg = config(&conn_id, &work.id, None, "Stuck");
        store
            .save(LibraryDocument::new(
                vec![work],
                Vec::new(),
                vec![cfg.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&conn_id, "s3cret").unwrap();

        let manager = SessionManager::new();
        let session = live_session(&manager, &cfg, never_resolving_connect()).await;
        wait_for_status(&manager, &session, |status| {
            matches!(status, SessionState::Connecting)
        })
        .await;

        let error = delete_connection_coordinated(&conn_id, &store, &creds, &manager)
            .await
            .expect_err("an actor that never stops must abort the deletion");
        assert!(
            error.contains("did not stop within five seconds"),
            "error should state the bounded stop failure: {error}"
        );
        assert!(
            error.contains("aborted"),
            "error should state the abort: {error}"
        );

        // Saved data retained: in-memory snapshot and on-disk file.
        assert!(store.get_connection(&conn_id).unwrap().is_some());
        assert_eq!(connection_ids(&reopened_store(&dir)), vec![conn_id.clone()]);
        // Keyring NOT cleaned (cleanup happens only after persistence).
        assert_eq!(
            creds.get_password(&conn_id).unwrap(),
            Some("s3cret".to_string())
        );
        // The stuck actor keeps its entry and stays live.
        let info = manager.get_session_info(&session).await.unwrap();
        assert_eq!(info.status, SessionState::Connecting);
        // Tombstones rolled back: Connect reuses the live entry instead
        // of failing with "was deleted".
        let again = live_session(&manager, &cfg, never_resolving_connect()).await;
        assert_eq!(again, session, "the live entry must be reusable");
    }

    // ---- 3. Persist failure retains data, reports stopped sessions -----

    #[tokio::test(start_paused = true)]
    async fn mixed_stop_timeout_keeps_already_stopped_sessions_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let stoppable = config(&cid(9), &work.id, None, "Stoppable");
        let stuck = config(&cid(10), &work.id, None, "Stuck");
        store
            .save(LibraryDocument::new(
                vec![work.clone()],
                Vec::new(),
                vec![stoppable.clone(), stuck.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&stuck.id, "s3cret").unwrap();

        let manager = SessionManager::new();
        // One Connected session that stops promptly on Disconnect…
        let s_ok = live_session(&manager, &stoppable, mock_loop_connect()).await;
        wait_for_status(&manager, &s_ok, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;
        // …and one live Connecting actor that never stops in the bound.
        let s_stuck = live_session(&manager, &stuck, never_resolving_connect()).await;

        let error = delete_library_coordinated(&work.id, &store, &creds, &manager)
            .await
            .expect_err("the never-stopping actor must abort the library deletion");
        assert!(
            error.contains("did not stop within five seconds"),
            "error should state the bounded stop failure: {error}"
        );

        // Deletion aborted: both connections and their credentials are
        // retained (snapshot and on-disk file).
        assert!(store.get_connection(&stuck.id).unwrap().is_some());
        assert!(store.get_connection(&stoppable.id).unwrap().is_some());
        assert_eq!(
            creds.get_password(&stuck.id).unwrap(),
            Some("s3cret".into())
        );
        assert_eq!(
            connection_ids(&reopened_store(&dir)),
            sorted(vec![stoppable.id.clone(), stuck.id.clone()])
        );

        // Already-stopped sessions remain stopped (entry gone); the
        // stuck actor keeps its live entry.
        assert!(manager.get_session_info(&s_ok).await.is_none());
        let info = manager.get_session_info(&s_stuck).await.unwrap();
        assert_eq!(info.status, SessionState::Connecting);

        // Tombstones rolled back: the survivors are connectable again —
        // the stuck entry is reused (live), the stopped one restarts.
        let again = live_session(&manager, &stuck, never_resolving_connect()).await;
        assert_eq!(again, s_stuck);
        let fresh = live_session(&manager, &stoppable, mock_loop_connect()).await;
        assert_ne!(fresh, s_ok);
    }

    #[tokio::test(start_paused = true)]
    async fn persist_failure_retains_saved_data_and_reports_sessions_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let conn_id = cid(3);
        let cfg = config(&conn_id, &work.id, None, "Persist");
        store
            .save(LibraryDocument::new(
                vec![work],
                Vec::new(),
                vec![cfg.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&conn_id, "s3cret").unwrap();

        let manager = SessionManager::new();
        let session = live_session(&manager, &cfg, mock_loop_connect()).await;
        wait_for_status(&manager, &session, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;

        store.fail_next_commit();
        let error = delete_connection_coordinated(&conn_id, &store, &creds, &manager)
            .await
            .expect_err("a failed persist must fail the deletion");
        assert!(
            error.contains("sessions were stopped"),
            "persist failure must report that sessions were stopped: {error}"
        );
        assert!(
            error.contains("retained"),
            "persist failure must state the saved data was retained: {error}"
        );

        // Sessions stopped, but the saved connection survives in the
        // snapshot AND on disk (restart validity after the failure).
        assert!(manager.get_session_info(&session).await.is_none());
        assert!(store.get_connection(&conn_id).unwrap().is_some());
        assert_eq!(connection_ids(&reopened_store(&dir)), vec![conn_id.clone()]);
        // Keyring NOT cleaned: cleanup only follows successful persists.
        assert_eq!(
            creds.get_password(&conn_id).unwrap(),
            Some("s3cret".to_string())
        );
        // Tombstones rolled back: Connect works again for the survivor
        // (fresh identity — its stopped entry was removed).
        let fresh = live_session(&manager, &cfg, mock_loop_connect()).await;
        assert_ne!(fresh, session);
    }

    // ---- 4. Keyring cleanup failure reports orphans (Req 1.5/6.2) ------

    #[tokio::test(start_paused = true)]
    async fn keyring_cleanup_failure_still_deletes_and_reports_orphan_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let conn_id = cid(4);
        let cfg = config(&conn_id, &work.id, None, "Orphan");
        store
            .save(LibraryDocument::new(
                vec![work],
                Vec::new(),
                vec![cfg.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(Some(Fail::Platform));
        creds.store_password(&conn_id, "s3cret").unwrap();

        let manager = SessionManager::new();
        let session = live_session(&manager, &cfg, mock_loop_connect()).await;
        wait_for_status(&manager, &session, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;

        let report = delete_connection_coordinated(&conn_id, &store, &creds, &manager)
            .await
            .expect("best-effort cleanup must not fail the deletion");
        assert!(report.deleted);
        assert_eq!(
            report.orphaned_credential_ids,
            vec![conn_id.clone()],
            "the failed cleanup is reported for an explicit retry"
        );
        // The orphan is never mistaken for a deleted record: the store
        // connection IS gone (persist succeeded) while the credential
        // still resolves for the retry.
        assert!(store.get_connection(&conn_id).unwrap().is_none());
        assert_eq!(connection_ids(&reopened_store(&dir)), Vec::<String>::new());
        assert_eq!(
            creds.get_password(&conn_id).unwrap(),
            Some("s3cret".to_string())
        );
    }

    // ---- 5. Delete/Connect race + double-click storm (design L365) ------

    #[tokio::test(start_paused = true)]
    async fn concurrent_connect_cannot_resurrect_a_deleting_connection() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let conn_id = cid(5);
        let cfg = config(&conn_id, &work.id, None, "Race");
        store
            .save(LibraryDocument::new(
                vec![work],
                Vec::new(),
                vec![cfg.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&conn_id, "s3cret").unwrap();

        let manager = Arc::new(SessionManager::new());
        let session = live_session(&manager, &cfg, mock_loop_connect()).await;
        wait_for_status(&manager, &session, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;

        // Genuinely concurrent: the deletion and a double-click Connect
        // storm run as spawned tasks on the same runtime, all racing the
        // admission lock.
        let store = Arc::new(store);
        let creds = Arc::new(creds);
        let deletion = {
            let (store, creds, manager, conn_id) = (
                Arc::clone(&store),
                Arc::clone(&creds),
                Arc::clone(&manager),
                conn_id.clone(),
            );
            tokio::spawn(async move {
                delete_connection_coordinated(&conn_id, &store, &creds, &manager).await
            })
        };
        let mut connects = Vec::new();
        for _ in 0..3 {
            let manager = Arc::clone(&manager);
            let config = cfg.clone();
            connects.push(tokio::spawn(async move {
                admit(&manager, &config, false, mock_loop_connect(), false).await
            }));
        }

        let report = deletion
            .await
            .expect("deletion task must join")
            .expect("deletion should win the race against Connect");
        // Exactly one actor was live and stopped — no double-stop of the
        // same session, no admission between stop and commit.
        assert_eq!(report.stopped_session_ids, vec![session.clone()]);
        assert!(report.deleted);

        for attempt in connects {
            let error = attempt
                .await
                .expect("connect task must join")
                .expect_err("no actor may appear for a deleted connection");
            assert!(
                error.contains("was deleted"),
                "racing Connect must fail closed with the tombstone error: {error}"
            );
        }
        assert!(manager.list_sessions().await.is_empty());
        assert!(store.get_connection(&conn_id).unwrap().is_none());
    }

    // ---- 6. Library deletion cascades across live sessions (10.2/10.17/10.18)

    #[tokio::test(start_paused = true)]
    async fn library_deletion_stops_and_cancels_all_cascaded_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let other = Library::new("Other");
        let f1 = Folder::new(&work.id, None, "F1");
        let c1 = config(&cid(6), &work.id, Some(&f1.id), "Retry");
        let c2 = config(&cid(7), &work.id, None, "Live");
        let o1 = config(&cid(8), &other.id, None, "Keep");
        store
            .save(LibraryDocument::new(
                vec![work.clone(), other.clone()],
                vec![f1.clone()],
                vec![c1.clone(), c2.clone(), o1.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&c1.id, "pw-1").unwrap();
        creds.store_password(&c2.id, "pw-2").unwrap();
        creds.store_password(&o1.id, "pw-3").unwrap();

        let manager = SessionManager::new();
        // c1: live Reconnecting backoff — the retry the deletion cancels.
        let s1 = admit(&manager, &c1, true, mock_loop_connect(), true)
            .await
            .expect("reconnecting session should be admitted");
        wait_for_status(&manager, &s1, |status| {
            matches!(status, SessionState::Reconnecting)
        })
        .await;
        // c2: live Connected mock loop.
        let s2 = live_session(&manager, &c2, mock_loop_connect()).await;
        wait_for_status(&manager, &s2, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;
        // o1 in the surviving library: live Connecting, never addressed.
        let s_keep = live_session(&manager, &o1, never_resolving_connect()).await;

        let report = delete_library_coordinated(&work.id, &store, &creds, &manager)
            .await
            .expect("coordinated library deletion should succeed");
        assert!(report.deleted);
        assert_eq!(
            sorted(report.cascaded.deleted_folder_ids),
            sorted(vec![f1.id.clone()])
        );
        assert_eq!(
            sorted(report.cascaded.deleted_connection_ids),
            sorted(vec![c1.id.clone(), c2.id.clone()])
        );
        assert!(report.cascaded.moved_connection_ids.is_empty());
        assert_eq!(
            sorted(report.stopped_session_ids),
            sorted(vec![s1.clone(), s2.clone()])
        );
        assert!(report.orphaned_credential_ids.is_empty());

        // Both cascaded actors are gone; the survivor keeps running.
        assert!(manager.get_session_info(&s1).await.is_none());
        assert!(manager.get_session_info(&s2).await.is_none());
        let survivors: Vec<String> = manager
            .list_sessions()
            .await
            .into_iter()
            .map(|info| info.id)
            .collect();
        assert_eq!(survivors, vec![s_keep.clone()]);

        // Keyring cleaned for every deleted connection, kept for the
        // survivor (cleanup follows the one committed transaction).
        assert_eq!(creds.get_password(&c1.id).unwrap(), None);
        assert_eq!(creds.get_password(&c2.id).unwrap(), None);
        assert_eq!(
            creds.get_password(&o1.id).unwrap(),
            Some("pw-3".to_string())
        );

        // The surviving library is intact in the snapshot and on disk.
        assert_eq!(connection_ids(&store), vec![o1.id.clone()]);
        assert_eq!(connection_ids(&reopened_store(&dir)), vec![o1.id.clone()]);
        let folders: Vec<String> = reopened_store(&dir)
            .document()
            .unwrap()
            .folders
            .into_iter()
            .map(|folder| folder.id)
            .collect();
        assert!(
            !folders.contains(&f1.id),
            "the deleted library's folders must not survive on disk: {folders:?}"
        );
    }

    // ---- 7. Unknown ids keep the v1 no-op / typed-error contracts -------

    #[tokio::test(start_paused = true)]
    async fn unknown_ids_are_noop_for_connections_and_typed_for_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        store
            .save(LibraryDocument::new(
                vec![work.clone()],
                Vec::new(),
                Vec::new(),
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        let manager = SessionManager::new();

        // v1 contract: deleting an unknown connection id is a no-op that
        // stops nothing and touches nothing.
        let report = delete_connection_coordinated("conn-404", &store, &creds, &manager)
            .await
            .expect("unknown connection ids are a v1 no-op");
        assert!(!report.deleted);
        assert_eq!(
            report.cascaded,
            CascadedIds {
                deleted_folder_ids: Vec::new(),
                deleted_connection_ids: Vec::new(),
                moved_connection_ids: Vec::new(),
            }
        );
        assert!(report.stopped_session_ids.is_empty());
        assert!(report.orphaned_credential_ids.is_empty());

        // Unknown library ids are typed rejections, never silent no-ops.
        let error = delete_library_coordinated("lib-404", &store, &creds, &manager)
            .await
            .expect_err("unknown library ids must be typed rejections");
        assert!(
            error.contains("unknown"),
            "library deletion error should be the typed store rejection: {error}"
        );
        assert!(store.list_tree().unwrap().libraries.len() == 1);
    }

    // ---- 8. Extras sweep timeout cannot resurrect a committed deletion

    /// Regression (review round 1): a connection that JOINED the library
    /// between the `library_connection_ids` preview and the
    /// `delete_library` commit (an "extra") was deleted by the committed
    /// transaction without being previewed. When the post-commit stop of
    /// its actor times out, the deletion cannot abort: the extras'
    /// tombstones must SURVIVE the timeout — a stale-config Connect may
    /// never admit an actor for a connection that no longer exists in
    /// the store — and the extras that DID stop before the bound must
    /// still be reported in `stopped_session_ids` (Req 10.18 exactness).
    #[tokio::test(start_paused = true)]
    async fn extras_stop_timeout_after_commit_keeps_deleted_connections_unconnectable() {
        let dir = tempfile::tempdir().unwrap();
        let store = opened_store(&dir);
        let work = Library::new("Work");
        let previewed = config(&cid(11), &work.id, None, "Previewed");
        store
            .save(LibraryDocument::new(
                vec![work.clone()],
                Vec::new(),
                vec![previewed.clone()],
            ))
            .unwrap();
        let (creds, _keyring) = seeded_keyring(None);
        creds.store_password(&previewed.id, "pw-a").unwrap();

        let manager = SessionManager::new();
        // Two connections join the library INSIDE the commit transaction
        // (the preview→commit window): one whose live Connected actor
        // stops inside the sweep bound, one whose live Connecting actor
        // never does — pinning the exact sweep-timeout arm.
        let joiner_stops = config(&cid(12), &work.id, None, "JoinerStops");
        let joiner_stuck = config(&cid(13), &work.id, None, "JoinerStuck");
        creds.store_password(&joiner_stops.id, "pw-b").unwrap();
        creds.store_password(&joiner_stuck.id, "pw-c").unwrap();
        let s_stops = live_session(&manager, &joiner_stops, mock_loop_connect()).await;
        wait_for_status(&manager, &s_stops, |status| {
            matches!(status, SessionState::Connected)
        })
        .await;
        let s_stuck = live_session(&manager, &joiner_stuck, never_resolving_connect()).await;
        wait_for_status(&manager, &s_stuck, |status| {
            matches!(status, SessionState::Connecting)
        })
        .await;
        store.join_connections_inside_next_library_delete(vec![
            joiner_stops.clone(),
            joiner_stuck.clone(),
        ]);

        let report = delete_library_coordinated(&work.id, &store, &creds, &manager)
            .await
            .expect("the committed deletion must succeed despite the extras' stop timeout");
        assert!(report.deleted);
        assert_eq!(
            sorted(report.cascaded.deleted_connection_ids),
            sorted(vec![
                previewed.id.clone(),
                joiner_stops.id.clone(),
                joiner_stuck.id.clone(),
            ])
        );

        // All three connections are gone from the snapshot and on disk:
        // the commit is authoritative, the sweep timeout changes nothing.
        assert!(store.get_connection(&previewed.id).unwrap().is_none());
        assert!(store.get_connection(&joiner_stops.id).unwrap().is_none());
        assert!(store.get_connection(&joiner_stuck.id).unwrap().is_none());
        assert_eq!(connection_ids(&reopened_store(&dir)), Vec::<String>::new());

        // No resurrection (the review-round-1 defect): Connect fails
        // closed with the tombstone error for EVERY committed deletion —
        // including the joiners whose post-commit stop timed out.
        for deleted in [&previewed, &joiner_stops, &joiner_stuck] {
            let error = admit(&manager, deleted, false, mock_loop_connect(), false)
                .await
                .expect_err("admission must fail for a deleted connection");
            assert!(
                error.contains("was deleted"),
                "admission error should name the deletion: {error}"
            );
        }

        // The joiner that stopped inside the sweep bound is still
        // reported — the timeout must not discard its session id.
        assert_eq!(
            sorted(report.stopped_session_ids),
            sorted(vec![s_stops.clone()]),
            "extras that stopped before the bound must be reported (Req 10.18)"
        );

        // The stuck joiner keeps its live entry (still addressable), and
        // the keyring cleanup followed the committed cascade.
        let info = manager.get_session_info(&s_stuck).await.unwrap();
        assert_eq!(info.status, SessionState::Connecting);
        assert_eq!(creds.get_password(&previewed.id).unwrap(), None);
        assert_eq!(creds.get_password(&joiner_stops.id).unwrap(), None);
        assert_eq!(creds.get_password(&joiner_stuck.id).unwrap(), None);
    }
}
