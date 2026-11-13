use tauri::State;

use crate::store::settings::{AppSettings, AppSettingsPatch, SettingsStore};

#[tauri::command]
pub async fn get_settings(store: State<'_, SettingsStore>) -> Result<AppSettings, String> {
    Ok(store.get().await)
}

/// Partial update (task 5.2, B3-11 fix): the frontend sends
/// `Partial<AppSettings>`; absent fields keep their stored values
/// (Req 5.2) instead of resetting to the serde defaults.
#[tauri::command]
pub async fn update_settings(
    settings: AppSettingsPatch,
    store: State<'_, SettingsStore>,
) -> Result<AppSettings, String> {
    let updated = store.apply(settings).await?;
    log::info!("Settings updated");
    Ok(updated)
}
