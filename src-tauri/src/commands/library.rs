//! Task 6.4/6.5 (Req 10.1/10.2/10.3/10.5/10.8/10.15): thin library/folder
//! CRUD, read-model and tree-move commands over [`LibraryStore`]. Every
//! command only marshals arguments and results — validation, transaction
//! serialization and persistence live in the store layer (design "Library
//! Commands"; errors surface as `Result<T, String>` matching the existing
//! style).
//!
//! Scope boundary: delete commands belong to tasks 7.x (folder deletion
//! moves descendant connections; library deletion coordinates session
//! actors). Connection CRUD/import/duplication commands landed with task
//! 6.6 in `commands::connection`, over the same unified store. Tree moves
//! (drag/keyboard UI in group 8) landed with task 6.5 (`move_node`).

use tauri::State;

use crate::store::library::{
    ConnectionSummary, Folder, Library, LibraryStore, LibraryStoreError, LibraryTree, NodeId,
    StoreStatus,
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
