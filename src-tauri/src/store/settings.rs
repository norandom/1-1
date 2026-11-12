use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_port")]
    pub default_port: u16,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub show_performance_hud: bool,
    #[serde(default)]
    pub sidebar_collapsed: bool,
    #[serde(default = "default_resolution")]
    pub default_resolution: String,
    #[serde(default = "default_color_depth")]
    pub color_depth: u32,
    #[serde(default = "default_quality")]
    pub quality: u32,
    #[serde(default = "default_reconnect_timeout")]
    pub reconnect_timeout: u64,
    #[serde(default = "default_connection_timeout")]
    pub connection_timeout: u64,
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_port() -> u16 {
    3389
}

fn default_true() -> bool {
    true
}

fn default_resolution() -> String {
    "1920x1080".to_string()
}

fn default_color_depth() -> u32 {
    32
}

fn default_quality() -> u32 {
    80
}

fn default_reconnect_timeout() -> u64 {
    5000
}

fn default_connection_timeout() -> u64 {
    30000
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            default_port: default_port(),
            auto_reconnect: true,
            show_performance_hud: false,
            sidebar_collapsed: false,
            default_resolution: default_resolution(),
            color_depth: default_color_depth(),
            quality: default_quality(),
            reconnect_timeout: default_reconnect_timeout(),
            connection_timeout: default_connection_timeout(),
        }
    }
}

pub struct SettingsStore {
    settings: Arc<Mutex<AppSettings>>,
    file_path: PathBuf,
}

impl SettingsStore {
    pub fn new(data_dir: PathBuf) -> Self {
        let file_path = data_dir.join("settings.json");
        let settings = if file_path.exists() {
            match std::fs::read_to_string(&file_path) {
                Ok(data) => match serde_json::from_str::<AppSettings>(&data) {
                    Ok(s) => {
                        log::info!("Loaded settings from disk");
                        s
                    }
                    Err(e) => {
                        log::error!("Failed to parse settings file: {}", e);
                        AppSettings::default()
                    }
                },
                Err(e) => {
                    log::error!("Failed to read settings file: {}", e);
                    AppSettings::default()
                }
            }
        } else {
            AppSettings::default()
        };

        Self {
            settings: Arc::new(Mutex::new(settings)),
            file_path,
        }
    }

    pub async fn get(&self) -> AppSettings {
        self.settings.lock().await.clone()
    }

    pub async fn update(&self, new_settings: AppSettings) -> Result<AppSettings, String> {
        let mut settings = self.settings.lock().await;
        *settings = new_settings.clone();
        drop(settings);
        self.save(&new_settings).await?;
        Ok(new_settings)
    }

    async fn save(&self, settings: &AppSettings) -> Result<(), String> {
        let data = serde_json::to_string_pretty(settings)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        if let Some(parent) = self.file_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create data directory: {}", e))?;
        }
        tokio::fs::write(&self.file_path, data)
            .await
            .map_err(|e| format!("Failed to save settings: {}", e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Task 3.1 characterization tests (persistence baseline, Req 6.1). ----

    /// AppSettings does not derive PartialEq; compare through its JSON shape
    /// (which is exactly what the persistence layer round-trips).
    fn json_eq(a: &AppSettings, b: &AppSettings) -> bool {
        serde_json::to_value(a).unwrap() == serde_json::to_value(b).unwrap()
    }

    #[test]
    fn test_settings_defaults_current_values() {
        let d = AppSettings::default();
        assert_eq!(d.theme, "system");
        assert_eq!(d.default_port, 3389);
        assert!(d.auto_reconnect);
        assert!(!d.show_performance_hud);
        assert!(!d.sidebar_collapsed);
        assert_eq!(d.default_resolution, "1920x1080");
        assert_eq!(d.color_depth, 32);
        assert_eq!(d.quality, 80);
        assert_eq!(d.reconnect_timeout, 5000);
        // D2 (research.md): TS default is 10000 — mismatch tracked at Req 5.2 (task
        // 3.4). This pins the CURRENT Rust-side default; not a behavior approval.
        assert_eq!(d.connection_timeout, 30000);
    }

    #[tokio::test]
    async fn test_settings_update_round_trip_via_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("settings.json");
        let store = SettingsStore::new(dir.path().to_path_buf());
        assert!(
            json_eq(&store.get().await, &AppSettings::default()),
            "empty dir -> defaults"
        );

        let mut custom = AppSettings::default();
        custom.theme = "dark".into();
        custom.default_port = 4000;
        custom.show_performance_hud = true;
        custom.connection_timeout = 12345;
        let saved = store.update(custom.clone()).await.unwrap();
        assert!(json_eq(&saved, &custom));

        // File is a pretty JSON object whose keys are camelCase; reopen loads it back.
        let raw = std::fs::read_to_string(&file_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["theme"].as_str(), Some("dark"));
        assert_eq!(value["defaultPort"].as_u64(), Some(4000));
        assert_eq!(value["showPerformanceHud"].as_bool(), Some(true));
        assert_eq!(value["connectionTimeout"].as_u64(), Some(12345));

        let reopened = SettingsStore::new(dir.path().to_path_buf());
        assert!(json_eq(&reopened.get().await, &custom));
    }

    #[tokio::test]
    async fn test_malformed_settings_file_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("settings.json");
        std::fs::write(&file_path, "]] not json [[").unwrap();

        let store = SettingsStore::new(dir.path().to_path_buf());
        assert!(
            json_eq(&store.get().await, &AppSettings::default()),
            "current behavior: malformed settings.json -> defaults, no error surfaced"
        );
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "]] not json [[",
            "load alone does not rewrite the file"
        );
    }

    // ---- Task 3.4 characterization tests (shell/settings baseline, Req 5.2). ----

    #[test]
    fn test_partial_settings_json_resets_absent_fields_to_defaults_defect_b3_11() {
        // B3-11: the frontend sends Partial<AppSettings> to the
        // `update_settings` command (tauri.ts updateSettings), but the
        // command deserializes a FULL AppSettings whose every field carries
        // #[serde(default = …)] — every absent field silently resets to the
        // backend default instead of keeping the stored value. Pinned as a
        // defect; a fix must merge onto the stored settings and will flip
        // these assertions.
        let parsed: AppSettings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        let defaults = AppSettings::default();

        assert_eq!(parsed.theme, "dark", "the submitted field applies");
        assert_eq!(parsed.connection_timeout, defaults.connection_timeout);
        assert_eq!(parsed.quality, defaults.quality);
        assert_eq!(parsed.default_resolution, defaults.default_resolution);
        assert!(
            parsed.connection_timeout == 30000,
            "a stored 10000/12345 ms timeout resets to the backend default"
        );
    }
}
