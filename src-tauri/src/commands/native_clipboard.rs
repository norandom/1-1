use crate::rdp::{
    clipboard::ClipboardContext, clipboard_payload::ClipboardPayload, session::SessionManager,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};
use tauri::{Manager, State};
use tauri_plugin_clipboard_manager::ClipboardExt;

#[derive(Default)]
pub struct NativeClipboardState {
    owner: Mutex<Option<Arc<Lease>>>,
    operation: Mutex<()>,
    claim_sequence: AtomicU64,
}
struct Lease {
    id: String,
    session: String,
    epoch: String,
    valid: AtomicBool,
}
impl NativeClipboardState {
    fn reserve(&self) -> u64 {
        let mut owner = self.owner.lock().unwrap();
        let ticket = self.claim_sequence.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(old) = owner.take() {
            old.valid.store(false, Ordering::SeqCst);
        }
        ticket
    }
    fn claim(&self, ticket: u64, session: String, epoch: String) -> Result<String, String> {
        let mut owner = self.owner.lock().unwrap();
        if self.claim_sequence.load(Ordering::SeqCst) != ticket {
            return Err("Clipboard owner changed during setup.".into());
        }
        if let Some(old) = owner.take() {
            old.valid.store(false, Ordering::SeqCst);
        }
        let id = uuid::Uuid::new_v4().to_string();
        *owner = Some(Arc::new(Lease {
            id: id.clone(),
            session,
            epoch,
            valid: AtomicBool::new(true),
        }));
        Ok(id)
    }
    fn lease(&self, id: &str) -> Result<Arc<Lease>, String> {
        self.owner
            .lock()
            .unwrap()
            .as_ref()
            .filter(|owner| owner.id == id)
            .cloned()
            .ok_or_else(|| "Clipboard owner changed. Retry in the active session.".into())
    }
    fn release(&self, id: &str) {
        let mut owner = self.owner.lock().unwrap();
        if owner.as_ref().is_some_and(|owner| owner.id == id)
            && let Some(old) = owner.take()
        {
            old.valid.store(false, Ordering::SeqCst);
        }
    }
}
fn guard(lease: &Lease, deadline: Instant) -> Result<(), String> {
    if !lease.valid.load(Ordering::SeqCst) || Instant::now() >= deadline {
        Err("Clipboard operation expired or its session changed.".into())
    } else {
        Ok(())
    }
}

#[tauri::command]
pub async fn clipboard_local_acquire(
    session_id: String,
    epoch: String,
    sessions: State<'_, SessionManager>,
    native: State<'_, NativeClipboardState>,
) -> Result<String, String> {
    let ticket = native.reserve();
    let context = super::clipboard::clipboard_context(session_id.clone(), sessions).await?;
    if context.epoch != epoch {
        return Err("Connection changed. Retry clipboard sync.".into());
    }
    native.claim(ticket, session_id, epoch)
}
#[tauri::command]
pub fn clipboard_local_release(lease: String, native: State<'_, NativeClipboardState>) {
    native.release(&lease);
}

/// Native reads run on the blocking pool, never the UI thread (required on Linux).
async fn native_operation(
    app: tauri::AppHandle,
    id: String,
    payload: Option<ClipboardPayload>,
) -> Result<Option<ClipboardPayload>, String> {
    let lease = app.state::<NativeClipboardState>().lease(&id)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let job = tauri::async_runtime::spawn_blocking(move || {
        let native = app.state::<NativeClipboardState>();
        let _operation = native.operation.try_lock().map_err(
            |_| "Native clipboard is busy. Retry resync after the current operation finishes.",
        )?;
        guard(&lease, deadline)?;
        let check_context = || -> Result<ClipboardContext, String> {
            guard(&lease, deadline)?;
            let context = tauri::async_runtime::block_on(super::clipboard::clipboard_context(
                lease.session.clone(),
                app.state::<SessionManager>(),
            ))?;
            if context.epoch != lease.epoch {
                return Err("Connection changed. Retry clipboard sync.".into());
            }
            guard(&lease, deadline)?;
            Ok(context)
        };
        let context = check_context()?;
        if let Some(payload) = payload {
            if !context.direction.receives() {
                return Err("Receiving clipboard is disabled for this session.".into());
            }
            match payload {
                ClipboardPayload::Text(text) => {
                    ClipboardPayload::Text(text.clone()).validate()?;
                    guard(&lease, deadline)?;
                    app.clipboard().write_text(text).map_err(
                        |_| "Cannot write the native clipboard. It may be busy; retry resync.",
                    )?;
                }
                image @ ClipboardPayload::Image { .. } => {
                    let rgba = image.rgba()?;
                    let (width, height) = rgba.dimensions();
                    if !check_context()?.direction.receives() {
                        return Err("Receiving clipboard is disabled for this session.".into());
                    }
                    guard(&lease, deadline)?;
                    app.clipboard().write_image(&tauri::image::Image::new_owned(rgba.into_raw(), width, height)).map_err(|_| "Cannot write the native image clipboard. It may be busy; retry resync.")?;
                }
            }
            Ok(None)
        } else {
            if !context.direction.sends() {
                return Err("Sending clipboard is disabled for this session.".into());
            }
            guard(&lease, deadline)?;
            // Image-first preserves screenshots when the OS also offers descriptive text.
            let payload = match app.clipboard().read_image() {
                Ok(image) => {
                    ClipboardPayload::from_rgba(image.width(), image.height(), image.rgba())?
                }
                Err(_) => {
                    guard(&lease, deadline)?;
                    let text = app.clipboard().read_text().map_err(|_| "No supported text or image is available, or the native clipboard is busy. Copy again and retry.")?;
                    let payload = ClipboardPayload::Text(text);
                    payload.validate()?;
                    payload
                }
            };
            check_context()?;
            Ok(Some(payload))
        }
    });
    tokio::time::timeout(Duration::from_secs(8), job)
        .await
        .map_err(|_| "Native clipboard timed out. Retry resync.")?
        .map_err(|_| "Native clipboard worker failed. Retry resync.")?
}
#[tauri::command]
pub async fn clipboard_local_read(
    app: tauri::AppHandle,
    lease: String,
) -> Result<ClipboardPayload, String> {
    native_operation(app, lease, None)
        .await?
        .ok_or_else(|| "No clipboard content is available.".into())
}
#[tauri::command]
pub async fn clipboard_local_write(
    app: tauri::AppHandle,
    lease: String,
    payload: ClipboardPayload,
) -> Result<(), String> {
    native_operation(app, lease, Some(payload))
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_clipboard_owner_fences_queued_and_disposed_operations() {
        let state = NativeClipboardState::default();
        let first = state
            .claim(state.reserve(), "first".into(), "epoch-1".into())
            .unwrap();
        let queued = state.lease(&first).unwrap();
        let second = state
            .claim(state.reserve(), "second".into(), "epoch-2".into())
            .unwrap();
        assert!(guard(&queued, Instant::now() + Duration::from_secs(1)).is_err());
        assert!(state.lease(&first).is_err());
        state.release(&first); // Late old cleanup cannot revoke a newer owner.
        let current = state.lease(&second).unwrap();
        assert!(guard(&current, Instant::now() + Duration::from_secs(1)).is_ok());
        assert!(guard(&current, Instant::now()).is_err());
        state.release(&second);
        assert!(guard(&current, Instant::now() + Duration::from_secs(1)).is_err());
        let old_ticket = state.reserve();
        let new_ticket = state.reserve();
        let newest = state
            .claim(new_ticket, "new".into(), "new-epoch".into())
            .unwrap();
        assert!(
            state
                .claim(old_ticket, "old".into(), "old-epoch".into())
                .is_err()
        );
        assert!(state.lease(&newest).is_ok());
    }
}
