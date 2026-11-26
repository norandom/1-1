use std::sync::Arc;
use tauri::{
    AppHandle, State,
    ipc::{Channel, InvokeResponseBody},
};

use crate::rdp::client::SessionCommand;
use crate::rdp::session::{SessionInfo, SessionManager, admit_mock_session};
use crate::renderer::shared_frame::SharedFrame;
use crate::store::connections::{ConnectionConfig, ResizeMode};
use crate::store::credentials::CredentialStore;
use crate::store::library::LibraryStore;
use crate::store::settings::SettingsStore;

#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct InitialSize {
    width: u32,
    height: u32,
    /// Desktop scale factor in percent for the local display density.
    #[serde(default)]
    scale: Option<u32>,
}

/// Scale factors outside 100–500% are invalid in RDP; clamp what the window measured.
fn desktop_scale(scale: Option<u32>) -> Option<u32> {
    scale.map(ironrdp_displaycontrol::pdu::MonitorLayoutEntry::adjust_desktop_scale_factor)
}

fn initial_config(
    saved: &ConnectionConfig,
    size: Option<InitialSize>,
) -> Result<ConnectionConfig, String> {
    let mut config = saved.clone();
    config.host_profile.desktop_scale = desktop_scale(size.and_then(|size| size.scale));
    if config.host_profile.resize_mode == ResizeMode::Dynamic
        && let Some(size) = size
    {
        if size.width == 0 || size.height == 0 {
            return Err("Initial viewport dimensions must be positive".into());
        }
        let (width, height) = ironrdp_displaycontrol::pdu::MonitorLayoutEntry::adjust_display_size(
            size.width,
            size.height,
        );
        config.display_width = Some(width);
        config.display_height = Some(height);
    }
    Ok(config)
}

/// A connection's folder grant, read again on every connection attempt. An
/// unreadable grant file offers no folder; the desktop still connects.
fn share_source(
    grants: Arc<crate::store::share_grants::ShareGrantStore>,
) -> crate::rdp::session::ShareSource {
    Arc::new(move |connection_id| match grants.get(connection_id) {
        Ok(folders) => folders,
        Err(error) => {
            log::warn!("Folder share grants unavailable: {error}");
            Vec::new()
        }
    })
}

/// The password a session dials with: one entered at connect, otherwise the
/// keychain's, and only for a host that saves its password.
fn session_password(
    entered: Option<String>,
    host_saves: bool,
    keychain: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    match entered.filter(|password| !password.is_empty()) {
        Some(password) => Ok(Some(password)),
        None if host_saves => keychain(),
        None => Ok(None),
    }
}

#[tauri::command]
pub async fn pause_display(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    session_mgr.set_display_paused(&session_id, true).await
}

#[tauri::command]
pub async fn resume_display(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    session_mgr.set_display_paused(&session_id, false).await
}

#[tauri::command]
pub async fn session_view_returned(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let sender = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| "Session not found".to_string())?;
    sender
        .send(SessionCommand::ViewReturned)
        .await
        .map_err(|_| "Session stopped".to_string())
}

#[tauri::command]
pub async fn frame_consumed(
    session_id: String,
    receipt: crate::rdp::frame_delivery::FrameReceipt,
    refresh: bool,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let sender = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| "Session not found".to_string())?;
    sender
        .send(SessionCommand::FrameConsumed { receipt, refresh })
        .await
        .map_err(|_| "Session stopped".to_string())
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn connect(
    connection_id: String,
    frame_channel: Channel<InvokeResponseBody>,
    initial_size: Option<InitialSize>,
    // Entered at connect for a host that does not save its password, or has
    // none saved; held by the session only.
    password: Option<String>,
    // Task 5.3 (Req 12.2/12.10): optional command-level mock flag. Absent
    // or `false` is a normal real connection request.
    mock: Option<bool>,
    app: AppHandle,
    // Task 6.6: the unified v2 library store is THE connection store.
    conn_store: State<'_, LibraryStore>,
    cred_store: State<'_, CredentialStore>,
    session_mgr: State<'_, SessionManager>,
    settings_store: State<'_, SettingsStore>,
    display_path: State<'_, Arc<crate::renderer::display_path::DisplayPathState>>,
    share_grants: State<'_, Arc<crate::store::share_grants::ShareGrantStore>>,
) -> Result<String, String> {
    // Task 7.1 (Req 2.2/11.8/11.9): the session cap and per-connection
    // idempotent admission are enforced atomically inside SessionManager
    // when the actor is spawned, so a duplicate Connect for a live session
    // reaches that reuse path instead of being rejected at this boundary.

    // Look up the connection config
    let config = conn_store
        .get_connection(&connection_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Connection '{}' not found", connection_id))?;

    // Task 5.3 mock-session admission at the command boundary: production
    // builds reject any mock flag — persisted or command-level — regardless
    // of environment variables; test-support builds admit the request and
    // the actor runs the synthetic no-network session (Req 12.2/12.10).
    let mock_session = admit_mock_session(config.mock, mock)?;
    let mut config = initial_config(&config, initial_size)?;
    config.mock = mock_session;
    log::info!(
        "Initial RDP display request: {}x{} at {}% (sizing {:?})",
        config.display_width.unwrap_or(1920),
        config.display_height.unwrap_or(1080),
        config.host_profile.desktop_scale.unwrap_or(100),
        config.host_profile.resize_mode
    );

    let password = match session_password(password, config.host_profile.save_password, || {
        cred_store.get_password(&connection_id)
    }) {
        Ok(pw) => pw,
        Err(e) => {
            log::warn!(
                "Failed to retrieve password for connection {}: {}",
                connection_id,
                e
            );
            None
        }
    };

    // Read auto_reconnect setting
    let auto_reconnect = settings_store.get().await.auto_reconnect;

    log::info!(
        "Starting session for connection {} ({}:{})",
        connection_id,
        config.host,
        config.port
    );

    // Create and spawn the session actor (with password for CredSSP/NLA)
    // Spec rdp-native-renderer: on the native path the session draws into
    // its own frame store, which the render thread reads.
    let native_frame =
        (display_path.path() == crate::renderer::display_path::Path::Native).then(|| {
            SharedFrame::new(
                config.display_width.unwrap_or(1920),
                config.display_height.unwrap_or(1080),
            )
        });
    let registered_frame = native_frame.clone();
    let session_id = session_mgr
        .create_session(
            config,
            password,
            frame_channel,
            app,
            auto_reconnect,
            native_frame,
            share_source(Arc::clone(&share_grants)),
        )
        .await?;
    // The layer may have failed while this session was being created, after
    // the fallback told the sessions it knew about; then the store is refused.
    if let Some(store) = &registered_frame
        && !display_path.register(&session_id, store)
        && let Some(sender) = session_mgr.get_cmd_sender(&session_id).await
    {
        let _ = sender
            .send(crate::rdp::client::SessionCommand::UseCanvas)
            .await;
    }

    // Update last_connected_at on the connection
    conn_store
        .update_last_connected(&connection_id)
        .map_err(|e| e.to_string())?;

    Ok(session_id)
}

#[cfg(test)]
mod viewport_tests {
    use super::*;
    fn saved() -> ConnectionConfig {
        serde_json::from_value(serde_json::json!({
            "id":"viewport", "name":"viewport", "host":"test.invalid", "port":3389,
            "username":"user", "domain":"", "displayWidth":1920,"displayHeight":1080,
            "createdAt":"2026-01-01T00:00:00Z", "updatedAt":"2026-01-01T00:00:00Z"
        }))
        .unwrap()
    }
    #[test]
    fn viewport_initial_request_changes_only_transient_dimensions() {
        let saved = saved();
        let mut expected = saved.clone();
        expected.display_width = Some(1200);
        expected.display_height = Some(701);
        assert_eq!(
            initial_config(
                &saved,
                Some(InitialSize {
                    width: 1201,
                    height: 701,
                    scale: None
                })
            )
            .unwrap(),
            expected
        );
        assert_eq!(saved.display_width, Some(1920));
        assert_eq!(saved.display_height, Some(1080));
        assert_eq!(initial_config(&saved, None).unwrap(), saved);
    }
    #[test]
    fn viewport_fixed_preserves_saved_dimensions_and_dynamic_validates_bounds() {
        let mut saved = saved();
        assert!(
            initial_config(
                &saved,
                Some(InitialSize {
                    width: 0,
                    height: 500,
                    scale: None
                })
            )
            .is_err()
        );
        let bounded = initial_config(
            &saved,
            Some(InitialSize {
                width: u32::MAX,
                height: 1,
                scale: None,
            }),
        )
        .unwrap();
        assert_eq!(
            (bounded.display_width, bounded.display_height),
            (Some(8192), Some(200))
        );
        saved.host_profile.resize_mode = ResizeMode::Fixed;
        assert_eq!(
            initial_config(
                &saved,
                Some(InitialSize {
                    width: 0,
                    height: 0,
                    scale: None
                })
            )
            .unwrap(),
            saved
        );
        assert!(serde_json::from_str::<InitialSize>(r#"{"width":-1,"height":600}"#).is_err());
    }

    #[test]
    fn an_entered_password_wins_and_the_keychain_serves_only_hosts_that_save() {
        let keychain = || Ok(Some("saved".to_string()));
        let entered = Some("typed".to_string());
        assert_eq!(
            session_password(entered.clone(), true, keychain),
            Ok(Some("typed".into()))
        );
        assert_eq!(
            session_password(entered, false, keychain),
            Ok(Some("typed".into()))
        );
        assert_eq!(
            session_password(None, true, keychain),
            Ok(Some("saved".into()))
        );
        assert_eq!(
            session_password(Some(String::new()), true, keychain),
            Ok(Some("saved".into()))
        );
        let untouched = || -> Result<Option<String>, String> {
            panic!("keychain read for a host that does not save")
        };
        assert_eq!(session_password(None, false, untouched), Ok(None));
    }

    #[test]
    fn the_measured_scale_is_requested_for_this_session_only() {
        let saved = saved();
        let size = |scale| InitialSize {
            width: 2880,
            height: 1800,
            scale,
        };
        let config = initial_config(&saved, Some(size(Some(200)))).unwrap();
        assert_eq!(config.host_profile.desktop_scale, Some(200));
        let stored = serde_json::to_value(&config).unwrap();
        assert!(
            stored["hostProfile"].get("desktopScale").is_none(),
            "never saved"
        );
        // Out-of-range measurements are clamped to RDP's 100–500%.
        let low = initial_config(&saved, Some(size(Some(50)))).unwrap();
        assert_eq!(low.host_profile.desktop_scale, Some(100));
        let high = initial_config(&saved, Some(size(Some(900)))).unwrap();
        assert_eq!(high.host_profile.desktop_scale, Some(500));
        // Older frontends omit the scale; the server then keeps its default.
        let legacy: InitialSize = serde_json::from_str(r#"{"width":1280,"height":720}"#).unwrap();
        let config = initial_config(&saved, Some(legacy)).unwrap();
        assert_eq!(config.host_profile.desktop_scale, None);
    }
}

#[tauri::command]
pub async fn disconnect(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    session_mgr.remove_session(&session_id).await
}

/// Cancel an ongoing reconnection attempt. This sends a Disconnect command
/// to the session actor, which will stop the reconnect loop.
#[tauri::command]
pub async fn cancel_reconnect(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    // Sending Disconnect to the actor will cause it to exit the reconnect loop
    let cmd_tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;

    cmd_tx
        .send(SessionCommand::Disconnect)
        .await
        .map_err(|_| format!("Session '{}' actor is no longer running", session_id))?;

    log::info!("Cancel reconnect requested for session {}", session_id);
    Ok(())
}

#[tauri::command]
pub async fn list_sessions(
    session_mgr: State<'_, SessionManager>,
) -> Result<Vec<SessionInfo>, String> {
    Ok(session_mgr.list_sessions().await)
}

#[tauri::command]
pub async fn get_session_info(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<SessionInfo, String> {
    session_mgr
        .get_session_info(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))
}

#[tauri::command]
pub async fn send_key_event(
    session_id: String,
    key_code: u32,
    is_down: bool,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let cmd_tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;

    cmd_tx
        .send(SessionCommand::SendKey { key_code, is_down })
        .await
        .map_err(|_| format!("Session '{}' actor is no longer running", session_id))
}

/// Typed characters, sent as Unicode (spec rdp-keyboard-layout 2.1).
#[tauri::command]
pub async fn send_text(
    session_id: String,
    text: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let cmd_tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;
    cmd_tx
        .send(SessionCommand::SendText { text })
        .await
        .map_err(|_| format!("Session '{}' actor is no longer running", session_id))
}

#[tauri::command]
pub async fn send_mouse_event(
    session_id: String,
    x: i32,
    y: i32,
    button: Option<String>,
    event_type: String,
    scroll_delta: Option<i32>,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    let cmd_tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;

    cmd_tx
        .send(SessionCommand::SendMouse {
            x,
            y,
            button,
            event_type,
            scroll_delta: scroll_delta.unwrap_or(0),
        })
        .await
        .map_err(|_| format!("Session '{}' actor is no longer running", session_id))
}

#[tauri::command]
pub async fn resize_session(
    session_id: String,
    width: u32,
    height: u32,
    scale: Option<u32>,
    session_mgr: State<'_, SessionManager>,
) -> Result<(), String> {
    if width > 8192 || height > 8192 || width == 0 || height == 0 {
        return Err(format!(
            "Invalid resolution {}x{} (max 8192x8192)",
            width, height
        ));
    }

    let cmd_tx = session_mgr
        .get_cmd_sender(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))?;

    cmd_tx
        .send(SessionCommand::Resize {
            width,
            height,
            scale: desktop_scale(scale),
        })
        .await
        .map_err(|_| format!("Session '{}' actor is no longer running", session_id))
}

#[tauri::command]
pub async fn get_frame(
    session_id: String,
    session_mgr: State<'_, SessionManager>,
) -> Result<String, String> {
    session_mgr
        .get_frame(&session_id)
        .await
        .ok_or_else(|| format!("Session '{}' not found", session_id))
}
