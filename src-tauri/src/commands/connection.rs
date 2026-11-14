//! Task 6.6 (Req 1.1–1.5, 10.19, 12.10): connection commands over the
//! unified v2 [`LibraryStore`] (the v1 `ConnectionStore` is retired).
//! Commands stay thin marshalers: validation, ownership resolution,
//! transaction serialization and persistence live in the store layer
//! (`store::library`); only the keyring side effects (store password on
//! create/update, best-effort delete) live here, because credentials
//! never enter the document model (design "Data Models").

use tauri::State;

use crate::commands::deletion::{DeletionReport, delete_connection_coordinated};
use crate::rdp::session::SessionManager;
use crate::store::connections::{ConnectionConfig, ConnectionInput};
use crate::store::credentials::CredentialStore;
use crate::store::library::{LibraryStore, LibraryStoreError};

/// Surface a typed store error as the command-layer string (existing
/// command style, mirroring `commands::library`).
fn to_message(error: LibraryStoreError) -> String {
    error.to_string()
}

/// Store a submitted password in the OS credential store (Req 1.2), never
/// in the connection file: absent/empty passwords leave the keyring alone.
fn store_password_if_provided(
    cred_store: &CredentialStore,
    id: &str,
    password: Option<&str>,
) -> Result<(), String> {
    if let Some(pw) = password
        && !pw.is_empty()
    {
        cred_store.store_password(id, pw)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn list_connections(
    store: State<'_, LibraryStore>,
) -> Result<Vec<ConnectionConfig>, String> {
    store.list_connections().map_err(to_message)
}

#[tauri::command]
pub async fn get_connection(
    id: String,
    store: State<'_, LibraryStore>,
) -> Result<ConnectionConfig, String> {
    store
        .get_connection(&id)
        .map_err(to_message)?
        .ok_or_else(|| format!("Connection '{}' not found", id))
}

#[tauri::command]
pub async fn create_connection(
    config: ConnectionInput,
    store: State<'_, LibraryStore>,
    cred_store: State<'_, CredentialStore>,
) -> Result<ConnectionConfig, String> {
    let password = config.password.clone();
    let conn = store.create_connection(config).map_err(to_message)?;

    // Store password in the OS credential store if provided (Req 1.2);
    // the store file never contains it.
    store_password_if_provided(&cred_store, &conn.id, password.as_deref())?;

    log::info!("Created connection: {} ({})", conn.name, conn.id);
    Ok(conn)
}

#[tauri::command]
pub async fn update_connection(
    id: String,
    config: ConnectionInput,
    store: State<'_, LibraryStore>,
    cred_store: State<'_, CredentialStore>,
) -> Result<ConnectionConfig, String> {
    let password = config.password.clone();
    let updated = store
        .update_connection(&id, config)
        .map_err(to_message)?
        .ok_or_else(|| format!("Connection '{}' not found", id))?;

    // Regression for baseline defect B3-2: a submitted password used to be
    // silently dropped; it now reaches the keyring AFTER the store
    // transaction committed (mirroring create; unchanged passwords arrive
    // as absent/empty and leave the entry alone).
    store_password_if_provided(&cred_store, &id, password.as_deref())?;

    Ok(updated)
}

/// Req 1.5/6.2/10.17/10.18 (task 7.3): delete a connection through the
/// coordination layer (`commands::deletion`) — serialize against
/// Connect for the id, cancel retries and await actor stop with a
/// five-second bound, atomically persist the deletion, and only then
/// clean the stored credential best-effort. A stop timeout or persist
/// failure retains the saved data (reported); a failed keyring cleanup
/// still reports the successful deletion with the orphaned credential
/// listed for an explicit retry.
#[tauri::command]
pub async fn delete_connection(
    id: String,
    store: State<'_, LibraryStore>,
    cred_store: State<'_, CredentialStore>,
    session_mgr: State<'_, SessionManager>,
) -> Result<DeletionReport, String> {
    let report = delete_connection_coordinated(&id, &store, &cred_store, &session_mgr).await?;
    log::info!(
        "Deleted connection {}: {} connection(s), {} session(s) stopped, {} orphaned \
         credential(s)",
        id,
        report.cascaded.deleted_connection_ids.len(),
        report.stopped_session_ids.len(),
        report.orphaned_credential_ids.len(),
    );
    Ok(report)
}

/// Req 10.19 (design "Destructive actions and identity"): duplicate a
/// connection — non-secret settings into the same folder, fresh id and
/// deterministic available name, assigned by the store transaction. The
/// keyring is never copied or shared: this command takes no credential
/// store at all, so the duplicate requires password re-entry.
#[tauri::command]
pub async fn duplicate_connection(
    id: String,
    store: State<'_, LibraryStore>,
) -> Result<ConnectionConfig, String> {
    let duplicate = store.duplicate_connection(&id).map_err(to_message)?;
    log::info!(
        "Duplicated connection {} as {} ({})",
        id,
        duplicate.name,
        duplicate.id
    );
    Ok(duplicate)
}

#[tauri::command]
pub async fn import_rdp_file(
    path: String,
    library_id: Option<String>,
    folder_id: Option<String>,
    store: State<'_, LibraryStore>,
) -> Result<ConnectionConfig, String> {
    let mut input = crate::store::connections::parse_rdp_file(&path)?;
    input.library_id = library_id.unwrap_or_default();
    input.folder_id = folder_id;
    // Imports are always non-mock (Req 12.10): the parser hardcodes
    // mock:false; nothing here can override it.
    let conn = store.create_connection(input).map_err(to_message)?;
    log::info!(
        "Imported connection from RDP file: {} ({})",
        conn.name,
        conn.id
    );
    Ok(conn)
}

#[tauri::command]
pub async fn import_ssh_config(
    library_id: Option<String>,
    folder_id: Option<String>,
    store: State<'_, LibraryStore>,
) -> Result<usize, String> {
    let hosts = crate::utils::ssh_config::parse_ssh_config()?;
    let mut count = 0;
    for host in hosts {
        let mut input = crate::store::connections::ssh_host_to_connection_input(&host);
        input.library_id = library_id.clone().unwrap_or_default();
        input.folder_id = folder_id.clone();
        store.create_connection(input).map_err(to_message)?;
        count += 1;
    }
    log::info!("Imported {} connections from SSH config", count);
    Ok(count)
}

#[tauri::command]
pub async fn test_connection(host: String, port: u16) -> Result<bool, String> {
    crate::utils::network::test_tcp_connection(&host, port, 5000).await
}

#[tauri::command]
pub async fn measure_latency(host: String, port: u16) -> Result<Option<u64>, String> {
    crate::utils::network::measure_tcp_latency(&host, port, 5000).await
}

#[tauri::command]
pub async fn get_connection_password(
    id: String,
    cred_store: State<'_, CredentialStore>,
) -> Result<Option<String>, String> {
    cred_store.get_password(&id)
}
