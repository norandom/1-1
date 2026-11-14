//! Task 6.4/6.5 (Req 10.1/10.2/10.3/10.5/10.8/10.15): thin library/folder
//! CRUD, read-model and tree-move commands over [`LibraryStore`]. Every
//! command only marshals arguments and results — validation, transaction
//! serialization and persistence live in the store layer (design "Library
//! Commands"; errors surface as `Result<T, String>` matching the existing
//! style).
//!
//! Task 7.2 (Req 10.16/10.18) adds `delete_folder`: the folder subtree is
//! removed and descendant connections move to the library root in one
//! store transaction; the command reports the exact cascaded counts/IDs.
//! Scope boundary: library deletion coordinates session actors and
//! keyring cleanup — that stays with task 7.3. Connection
//! CRUD/import/duplication commands landed with task 6.6 in
//! `commands::connection`, over the same unified store. Tree moves
//! (drag/keyboard UI in group 8) landed with task 6.5 (`move_node`).

use tauri::State;

use crate::store::library::{
    CascadedIds, ConnectionSummary, Folder, Library, LibraryStore, LibraryStoreError, LibraryTree,
    NodeId, StoreStatus,
};

/// Surface a typed store error as the command-layer string (existing
/// command style, design L358).
fn to_message(error: LibraryStoreError) -> String {
    error.to_string()
}

/// Marshal IPC `kind` + `id` strings into a typed [`NodeId`]; shared by
/// the node-addressed commands (`list_descendants`, `move_node`).
fn parse_node(kind: &str, id: &str) -> Result<NodeId, String> {
    match kind {
        "library" => Ok(NodeId::library(id)),
        "folder" => Ok(NodeId::folder(id)),
        "connection" => Ok(NodeId::connection(id)),
        other => Err(format!(
            "unknown node kind '{other}' (expected library, folder or connection)"
        )),
    }
}

#[tauri::command]
pub async fn create_library(
    name: String,
    store: State<'_, LibraryStore>,
) -> Result<Library, String> {
    let library = store.create_library(&name).map_err(to_message)?;
    log::info!("Created library: {} ({})", library.name, library.id);
    Ok(library)
}

#[tauri::command]
pub async fn rename_library(
    id: String,
    name: String,
    store: State<'_, LibraryStore>,
) -> Result<Library, String> {
    let library = store.rename_library(&id, &name).map_err(to_message)?;
    log::info!("Renamed library {} to {}", library.id, library.name);
    Ok(library)
}

#[tauri::command]
pub async fn create_folder(
    library_id: String,
    parent_id: Option<String>,
    name: String,
    store: State<'_, LibraryStore>,
) -> Result<Folder, String> {
    let folder = store
        .create_folder(&library_id, parent_id.as_deref(), &name)
        .map_err(to_message)?;
    log::info!(
        "Created folder: {} ({}) in library {}",
        folder.name,
        folder.id,
        folder.library_id
    );
    Ok(folder)
}

#[tauri::command]
pub async fn rename_folder(
    id: String,
    name: String,
    store: State<'_, LibraryStore>,
) -> Result<Folder, String> {
    let folder = store.rename_folder(&id, &name).map_err(to_message)?;
    log::info!("Renamed folder {} to {}", folder.id, folder.name);
    Ok(folder)
}

/// Req 10.8/10.15: atomically move a folder or connection — possibly
/// across libraries — in one store transaction (design "Library
/// Commands": `move_node(node, target)`). Thin marshal: validation,
/// descendant `library_id` rewriting, transaction serialization and
/// persistence live in [`LibraryStore::move_node`].
#[tauri::command]
pub async fn move_node(
    kind: String,
    id: String,
    target_kind: String,
    target_id: String,
    store: State<'_, LibraryStore>,
) -> Result<(), String> {
    let node = parse_node(&kind, &id)?;
    let target = parse_node(&target_kind, &target_id)?;
    store.move_node(node, target).map_err(to_message)?;
    log::info!("Moved {kind} {id} under {target_kind} {target_id}");
    Ok(())
}

/// Req 10.16/10.18 (task 7.2, design "Destructive actions and identity"):
/// delete a folder and all nested folders while every descendant
/// connection moves to the same library's root — one store transaction,
/// never a deleted connection. Thin marshal: the subtree removal, root
/// re-filing, transaction serialization and persistence live in
/// [`LibraryStore::delete_folder`], which returns the exact cascaded
/// counts/IDs ([`CascadedIds`]) so the confirmation UI (group 8) can
/// state them verbatim. No keyring cleanup happens for moved records and
/// active sessions are not addressed on this path — their IDs are
/// unchanged (pinned in the tests below and in
/// `store::credentials::folder_deletion_tests`).
#[tauri::command]
pub async fn delete_folder(
    id: String,
    store: State<'_, LibraryStore>,
) -> Result<CascadedIds, String> {
    let cascaded = store.delete_folder(&id).map_err(to_message)?;
    log::info!(
        "Deleted folder {}: removed {} folder(s), moved {} connection(s) to the library root, deleted {} connection(s)",
        id,
        cascaded.deleted_folder_ids.len(),
        cascaded.moved_connection_ids.len(),
        cascaded.deleted_connection_ids.len(),
    );
    Ok(cascaded)
}

/// Read model for the sidebar tree (design `list_tree() -> LibraryTree`).
#[tauri::command]
pub async fn list_tree(store: State<'_, LibraryStore>) -> Result<LibraryTree, String> {
    store.list_tree().map_err(to_message)
}

/// Dashboard listing incl. connections in nested folders (Req 10.5).
/// `kind` + `id` are marshaled into a typed [`NodeId`] here.
#[tauri::command]
pub async fn list_descendants(
    kind: String,
    id: String,
    store: State<'_, LibraryStore>,
) -> Result<Vec<ConnectionSummary>, String> {
    let node = parse_node(&kind, &id)?;
    store.descendants(node).map_err(to_message)
}

/// Recovery read model (Req 10.12): current store mode for the UI; the
/// recovery instructions view (task 8.7) consumes this.
#[tauri::command]
pub async fn get_library_status(store: State<'_, LibraryStore>) -> Result<StoreStatus, String> {
    Ok(store.status())
}

/// Re-run the load/migration from disk (the only path out of write-blocked
/// recovery, Req 10.12) and report the resulting store mode.
#[tauri::command]
pub async fn reload_library_store(store: State<'_, LibraryStore>) -> Result<StoreStatus, String> {
    store.reload().map_err(|e| e.to_string())?;
    Ok(store.status())
}

#[cfg(test)]
mod tests {
    //! Task 7.2 (Req 10.16): active sessions survive folder deletion —
    //! the successful AND the failed path. [`LibraryStore::delete_folder`]
    //! never addresses the session manager (the store holds no reference
    //! to it) and never changes connection IDs, so a live session keyed by
    //! a moved connection keeps running and keeps resolving. Pinned here —
    //! the module where the store and the session manager meet at the
    //! command boundary — with a permanently-connecting scripted session:
    //! no network, no keyring, deterministic.
    //!
    //! Keyring survival for the same operation is pinned in
    //! `store::credentials::folder_deletion_tests`.

    use super::*;
    use crate::rdp::client::ConnectionOutcome;
    use crate::rdp::session::{
        ConnectFn, RecordedEvents, SessionEventSink, SessionManager, SessionState,
    };
    use crate::store::connections::ConnectionConfig;
    use crate::store::library::{Folder, Library, LibraryDocument};
    use std::sync::Arc;
    use tauri::ipc::InvokeResponseBody;
    use uuid::Uuid;

    fn library_config(library_id: &str, folder_id: Option<&str>, name: &str) -> ConnectionConfig {
        let now = chrono::Utc::now();
        ConnectionConfig {
            id: Uuid::new_v4().to_string(),
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

    /// A connector that never resolves: the actor stays in its live
    /// `Connecting` state for the whole test (task 7.1 admission keeps it
    /// running) without any network I/O.
    fn never_resolving_connect() -> ConnectFn {
        Arc::new(
            |_config: ConnectionConfig, _password: Option<String>, _width: u32, _height: u32| {
                let pending: std::pin::Pin<
                    Box<dyn std::future::Future<Output = ConnectionOutcome> + Send>,
                > = Box::pin(std::future::pending());
                pending
            },
        )
    }

    async fn live_session(manager: &SessionManager, config: &ConnectionConfig) -> String {
        let (events, _events_rx) = RecordedEvents::new();
        manager
            .create_session_with_transport(
                config.clone(),
                None,
                tauri::ipc::Channel::new(|_: InvokeResponseBody| Ok(())),
                SessionEventSink::recorded(events),
                false,
                None,
                never_resolving_connect(),
            )
            .await
            .expect("scripted session creation should succeed")
    }

    #[tokio::test]
    async fn live_session_survives_successful_folder_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let store = LibraryStore::open(dir.path().join("connections.json"));
        let work = Library::new("Work");
        let folder = Folder::new(&work.id, None, "F");
        let config = library_config(&work.id, Some(&folder.id), "conn");
        store
            .save(LibraryDocument::new(
                vec![work.clone()],
                vec![folder.clone()],
                vec![config.clone()],
            ))
            .unwrap();

        let manager = SessionManager::new();
        let session_id = live_session(&manager, &config).await;
        let info = manager.get_session_info(&session_id).await.unwrap();
        assert_eq!(info.connection_id, config.id);
        assert_eq!(info.status, SessionState::Connecting);

        let cascaded = store.delete_folder(&folder.id).unwrap();
        assert_eq!(cascaded.moved_connection_ids, vec![config.id.clone()]);
        assert!(cascaded.deleted_connection_ids.is_empty());

        // The session keeps running under the same identity and still
        // resolves (Req 10.16) — folder deletion only re-filed its
        // connection at the library root.
        let info = manager.get_session_info(&session_id).await.unwrap();
        assert_eq!(info.id, session_id);
        assert_eq!(info.connection_id, config.id);
        assert_eq!(info.status, SessionState::Connecting, "still live");
        assert!(
            manager.get_cmd_sender(&session_id).await.is_some(),
            "command channel still resolves"
        );

        // The moved record still exists, now at the library root.
        let tree = store.list_tree().unwrap();
        let summary = tree
            .connections
            .iter()
            .find(|connection| connection.id == config.id)
            .unwrap();
        assert_eq!(summary.folder_id, None);
        assert_eq!(summary.library_id, work.id);
    }

    #[tokio::test]
    async fn live_session_survives_failed_folder_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let store = LibraryStore::open(dir.path().join("connections.json"));
        let work = Library::new("Work");
        let folder = Folder::new(&work.id, None, "F");
        let config = library_config(&work.id, Some(&folder.id), "conn");
        store
            .save(LibraryDocument::new(
                vec![work],
                vec![folder.clone()],
                vec![config.clone()],
            ))
            .unwrap();

        let manager = SessionManager::new();
        let session_id = live_session(&manager, &config).await;

        store.fail_next_commit();
        assert!(matches!(
            store.delete_folder(&folder.id),
            Err(LibraryStoreError::PersistFailed { .. })
        ));

        // Session untouched and still live after the failed deletion.
        let info = manager.get_session_info(&session_id).await.unwrap();
        assert_eq!(info.connection_id, config.id);
        assert_eq!(info.status, SessionState::Connecting, "still live");
        assert!(
            manager.get_cmd_sender(&session_id).await.is_some(),
            "command channel still resolves"
        );

        // The store rolled back: the folder and its filed connection are
        // exactly as before.
        let tree = store.list_tree().unwrap();
        assert!(tree.folders.iter().any(|f| f.id == folder.id));
        let summary = tree
            .connections
            .iter()
            .find(|connection| connection.id == config.id)
            .unwrap();
        assert_eq!(summary.folder_id.as_deref(), Some(folder.id.as_str()));
    }
}
