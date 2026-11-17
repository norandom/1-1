use crate::rdp::session::SessionManager;
use std::time::Duration;
use tauri::{AppHandle, State, ipc::Channel};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

#[derive(Default)]
pub struct UpdateState {
    // Serializes checks and downloads; no updater install IPC permission is exposed.
    pending: Mutex<Option<Update>>,
}
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    current_version: String,
    update: Option<UpdateInfo>,
}
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
    phase: &'static str,
}

#[tauri::command]
pub async fn check_for_updates(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<UpdateCheck, String> {
    let mut pending = state
        .pending
        .try_lock()
        .map_err(|_| "An update operation is already running.")?;
    *pending = None;
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Update check failed: {e}"))?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        notes: u.body.clone(),
    });
    *pending = update;
    Ok(UpdateCheck {
        current_version: app.package_info().version.to_string(),
        update: info,
    })
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
    sessions: State<'_, SessionManager>,
    progress: Channel<UpdateProgress>,
) -> Result<(), String> {
    let pending = state
        .pending
        .try_lock()
        .map_err(|_| "An update operation is already running.")?;
    let mut update = pending
        .as_ref()
        .ok_or("Check for an available update first.")?
        .clone();
    sessions.run_when_idle(|| Ok(())).await?;
    update.timeout = Some(Duration::from_secs(600));
    let mut downloaded = 0u64;
    // Official download verifies the embedded public-key signature before returning bytes.
    let bytes = update
        .download(
            |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                let _ = progress.send(UpdateProgress {
                    downloaded,
                    total,
                    phase: "downloading",
                });
            },
            || {},
        )
        .await
        .map_err(|e| format!("Update download or verification failed: {e}"))?;
    // Recheck after network work and hold the admission lock through install/restart.
    // A new Connect or terminal-session restart cannot race this final decision.
    sessions
        .run_when_idle(|| {
            let _ = progress.send(UpdateProgress {
                downloaded,
                total: Some(downloaded),
                phase: "installing",
            });
            update
                .install(&bytes)
                .map_err(|e| format!("Update installation failed: {e}"))?;
            app.restart();
        })
        .await
}
