use crate::rdp::peer_identity::pins::{IdentityReview, PinStore};
use crate::rdp::peer_identity::{ConnectionTarget, fingerprint};
use crate::store::library::LibraryStore;
use tauri::State;
use uuid::Uuid;

#[tauri::command]
pub(crate) async fn review_server_identity(
    id: String,
    library: State<'_, LibraryStore>,
    pins: State<'_, PinStore>,
) -> Result<IdentityReview, String> {
    pins.review(&library, &id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) async fn confirm_server_identity(
    selection: Uuid,
    library: State<'_, LibraryStore>,
    pins: State<'_, PinStore>,
) -> Result<(), String> {
    pins.confirm(&library, selection).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn cancel_server_identity(selection: Uuid, pins: State<'_, PinStore>) {
    pins.cancel(selection);
}

/// The fingerprint confirmed for a saved host's address, if any, without
/// connecting. Used to carry the host's identity in a copied command line.
#[tauri::command]
pub(crate) fn confirmed_fingerprint(
    id: String,
    library: State<'_, LibraryStore>,
    pins: State<'_, PinStore>,
) -> Result<Option<String>, String> {
    let Some(host) = library.get_connection(&id).map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let Ok(target) = ConnectionTarget::new(&host.host, host.port) else {
        return Ok(None);
    };
    let pin = pins.lookup(&target.endpoint).map_err(|e| e.to_string())?;
    Ok(pin.map(|pin| fingerprint(&pin.spki_sha256())))
}
