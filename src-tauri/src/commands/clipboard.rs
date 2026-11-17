use crate::rdp::client::SessionCommand;
use crate::rdp::clipboard::ClipboardContext;
use crate::rdp::clipboard_payload::ClipboardPayload;
use crate::rdp::session::SessionManager;
use tauri::State;
#[tauri::command]
pub async fn clipboard_write(
    session_id: String,
    text: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let context = clipboard_context(session_id.clone(), session_mgr.clone()).await?;
    clipboard_sync(
        session_id,
        context.epoch,
        Some(ClipboardPayload::Text(text)),
        session_mgr,
    )
    .await
}
#[tauri::command]
pub async fn clipboard_context(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<ClipboardContext, String> {
    let tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or("Session is unavailable")?;
    let (reply, result) = tokio::sync::oneshot::channel();
    tx.send(SessionCommand::ClipboardContext { reply })
        .await
        .map_err(|_| "Session has closed")?;
    tokio::time::timeout(std::time::Duration::from_secs(3), result)
        .await
        .map_err(|_| "Clipboard setup timed out")?
        .map_err(|_| "Clipboard requires a connected real session".into())
}
#[tauri::command]
pub async fn clipboard_sync(
    session_id: String,
    epoch: String,
    payload: Option<ClipboardPayload>,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    if let Some(payload) = &payload {
        payload.validate()?;
    }
    let tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or("Session is unavailable")?;
    let (reply, result) = tokio::sync::oneshot::channel();
    tx.send(SessionCommand::ClipboardSync {
        epoch,
        payload,
        reply,
    })
    .await
    .map_err(|_| "Session has closed")?;
    tokio::time::timeout(std::time::Duration::from_secs(8), result).await
        .map_err(|_| "Clipboard timed out. Retry after the outstanding response arrives; reconnect if it remains unresponsive.")?
        .map_err(|_| "Clipboard channel is unavailable. Reconnect the session.")?
}
#[tauri::command]
pub async fn clipboard_read(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<String, String> {
    session_mgr
        .get_remote_clipboard(&session_id)
        .await
        .ok_or_else(|| "No remote clipboard text available".into())
}

#[tauri::command]
pub async fn clipboard_paste(
    session_id: String,
    epoch: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or("Session is unavailable")?;
    let (reply, result) = tokio::sync::oneshot::channel();
    tx.send(SessionCommand::ClipboardPaste { epoch, reply })
        .await
        .map_err(|_| "Session closed")?;
    tokio::time::timeout(std::time::Duration::from_secs(3), result)
        .await
        .map_err(|_| "Remote paste timed out")?
        .map_err(|_| "Session closed")?
}
