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

/// Partial update payload for the `update_settings` command (task 5.2,
/// B3-11 fix): `None` fields keep their stored values on merge. camelCase
/// mirrors the TS `Partial<AppSettings>` keys (`src/types.ts` ↔ this file).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsPatch {
    pub theme: Option<String>,
    pub default_port: Option<u16>,
    pub auto_reconnect: Option<bool>,
    pub show_performance_hud: Option<bool>,
    pub sidebar_collapsed: Option<bool>,
    pub default_resolution: Option<String>,
    pub color_depth: Option<u32>,
    pub quality: Option<u32>,
    pub reconnect_timeout: Option<u64>,
    pub connection_timeout: Option<u64>,
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

    /// Merge a partial update onto the stored settings (task 5.2, B3-11
    /// fix, Req 5.2): absent fields keep their stored values instead of
    /// silently resetting to the serde defaults, then persist to
    /// settings.json. The serde defaults stay the file-load contract for
    /// fresh installs and settings.json files from older builds.
    pub async fn apply(&self, patch: AppSettingsPatch) -> Result<AppSettings, String> {
        let mut settings = self.settings.lock().await;
        if let Some(theme) = patch.theme {
            settings.theme = theme;
        }
        if let Some(default_port) = patch.default_port {
            settings.default_port = default_port;
        }
        if let Some(auto_reconnect) = patch.auto_reconnect {
            settings.auto_reconnect = auto_reconnect;
        }
        if let Some(show_performance_hud) = patch.show_performance_hud {
            settings.show_performance_hud = show_performance_hud;
        }
        if let Some(sidebar_collapsed) = patch.sidebar_collapsed {
            settings.sidebar_collapsed = sidebar_collapsed;
        }
        if let Some(default_resolution) = patch.default_resolution {
            settings.default_resolution = default_resolution;
        }
        if let Some(color_depth) = patch.color_depth {
            settings.color_depth = color_depth;
        }
        if let Some(quality) = patch.quality {
            settings.quality = quality;
        }
        if let Some(reconnect_timeout) = patch.reconnect_timeout {
            settings.reconnect_timeout = reconnect_timeout;
        }
        if let Some(connection_timeout) = patch.connection_timeout {
            settings.connection_timeout = connection_timeout;
        }
        let merged = settings.clone();
        drop(settings);
        self.save(&merged).await?;
        Ok(merged)
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
        // D2 fixed (task 5.2, Req 5.2): the TS default
        // (stores/settingsStore.ts defaultSettings.connectionTimeout) now
        // equals this value; parity is pinned on both sides of the mirror
        // (settingsStore.test.ts BACKEND_DEFAULTS).
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
        // Task 5.2: the update flow is partial (`apply`); a patch carrying
        // every customized field is the full-replacement equivalent.
        let saved = store
            .apply(AppSettingsPatch {
                theme: Some(custom.theme.clone()),
                default_port: Some(custom.default_port),
                show_performance_hud: Some(custom.show_performance_hud),
                connection_timeout: Some(custom.connection_timeout),
                ..AppSettingsPatch::default()
            })
            .await
            .unwrap();
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

    // ---- Task 5.2 tests (Req 5.2: partial updates merge; persisted
    // settings and fresh defaults match; default parity with the TS
    // literal in stores/settingsStore.ts). The B3-11 defect pin that used
    // to live here was flipped by the merge fix above. ----

    #[test]
    fn test_settings_patch_deserializes_partial_ipc_payloads() {
        // The command payload is the TS Partial<AppSettings> (camelCase);
        // absent fields deserialize to None so the merge keeps stored
        // values (B3-11 fix).
        let patch: AppSettingsPatch = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(patch.theme.as_deref(), Some("dark"));
        assert_eq!(patch.connection_timeout, None);
        assert_eq!(patch.quality, None);

        let patch: AppSettingsPatch =
            serde_json::from_str(r#"{"defaultPort":4000,"connectionTimeout":12345}"#).unwrap();
        assert_eq!(patch.default_port, Some(4000));
        assert_eq!(patch.connection_timeout, Some(12345));
        assert_eq!(patch.theme, None);

        let empty: AppSettingsPatch = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, AppSettingsPatch::default());
    }

    #[tokio::test]
    async fn test_partial_update_merges_onto_stored_settings_b3_11() {
        // B3-11 fixed (task 5.2): `update_settings` used to deserialize the
        // partial IPC payload as a FULL AppSettings whose per-field serde
        // defaults silently reset every absent field — changing one setting
        // lost all other user settings on the next launch (Req 5.2). The
        // command now takes AppSettingsPatch and merges onto the stored
        // settings (the flipped assertions of the former defect pin).
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().to_path_buf());

        // Seed stored customizations through the update path itself, as
        // the real IPC payloads would.
        let seed: AppSettingsPatch =
            serde_json::from_str(r#"{"theme":"dark","quality":60,"connectionTimeout":12345}"#)
                .unwrap();
        store.apply(seed).await.unwrap();

        let patch: AppSettingsPatch = serde_json::from_str(r#"{"quality":42}"#).unwrap();
        let merged = store.apply(patch).await.unwrap();

        assert_eq!(merged.quality, 42, "the submitted field applies");
        assert_eq!(merged.theme, "dark", "stored theme survives the patch");
        assert_eq!(
            merged.connection_timeout, 12345,
            "stored timeout survives instead of resetting to the default"
        );

        // …and the merged state is what a relaunch restores (Req 5.2).
        let reopened = SettingsStore::new(dir.path().to_path_buf());
        let restored = reopened.get().await;
        assert_eq!(restored.quality, 42);
        assert_eq!(restored.theme, "dark");
        assert_eq!(restored.connection_timeout, 12345);
    }

    #[tokio::test]
    async fn test_empty_patch_keeps_fresh_defaults_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().to_path_buf());

        let merged = store.apply(AppSettingsPatch::default()).await.unwrap();
        assert!(
            json_eq(&merged, &AppSettings::default()),
            "an empty patch must leave a fresh store identical to the defaults"
        );
    }

    #[tokio::test]
    async fn test_persisted_defaults_round_trip_to_fresh_defaults() {
        // Task 5.2 done criterion: persisted settings and fresh defaults
        // match — persisting the defaults and relaunching yields exactly a
        // fresh default store, on both sides of the TS/Rust mirror.
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path().to_path_buf());
        store.apply(AppSettingsPatch::default()).await.unwrap();

        let reopened = SettingsStore::new(dir.path().to_path_buf());
        assert!(
            json_eq(&reopened.get().await, &AppSettings::default()),
            "persisted defaults must equal fresh defaults"
        );
    }

    #[test]
    fn test_partial_settings_file_load_fills_absent_fields_with_serde_defaults() {
        // The serde defaults stay the file-load contract (Req 5.2 reuses
        // them): settings.json from a fresh install or an older build may
        // omit fields; absent fields load as defaults rather than failing.
        let parsed: AppSettings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        let defaults = AppSettings::default();

        assert_eq!(parsed.theme, "dark", "the stored field loads");
        assert_eq!(parsed.connection_timeout, defaults.connection_timeout);
        assert_eq!(parsed.quality, defaults.quality);
        assert_eq!(parsed.default_resolution, defaults.default_resolution);
    }
}
