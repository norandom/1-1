use crate::rdp::peer_identity::pins::{IdentityReview, PinStore};
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
