use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub color_accent: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub display_width: Option<u32>,
    #[serde(default)]
    pub display_height: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub last_connected_at: Option<DateTime<Utc>>,
    /// Task 5.3 (Req 12.2/12.10): marks a synthetic (mock) saved connection.
    /// Data only — production builds reject mock-session requests at the
    /// connect boundary (`rdp::session::admit_mock_session`); test-support
    /// builds execute it as a no-network synthetic session. The v2 library
    /// model keeps this field (design "LibraryStore v2", ConnectionConfigV2).
    #[serde(default)]
    pub mock: bool,
}

/// Input type for creating/updating connections (no id or timestamps).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub color_accent: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub display_width: Option<u32>,
    #[serde(default)]
    pub display_height: Option<u32>,
    /// Task 5.3: persisted mock flag (see `ConnectionConfig::mock`).
    #[serde(default)]
    pub mock: bool,
    #[serde(default)]
    pub password: Option<String>,
}

fn default_port() -> u16 {
    3389
}

pub struct ConnectionStore {
    connections: Arc<Mutex<HashMap<String, ConnectionConfig>>>,
    file_path: PathBuf,
}

impl ConnectionStore {
    pub fn new(data_dir: PathBuf) -> Self {
        let file_path = data_dir.join("connections.json");
        let connections = if file_path.exists() {
            match std::fs::read_to_string(&file_path) {
                Ok(data) => match serde_json::from_str::<Vec<ConnectionConfig>>(&data) {
                    Ok(list) => {
                        let map: HashMap<String, ConnectionConfig> =
                            list.into_iter().map(|c| (c.id.clone(), c)).collect();
                        log::info!("Loaded {} connections from disk", map.len());
                        map
                    }
                    Err(e) => {
                        log::error!("Failed to parse connections file: {}", e);
                        HashMap::new()
                    }
                },
                Err(e) => {
                    log::error!("Failed to read connections file: {}", e);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Self {
            connections: Arc::new(Mutex::new(connections)),
            file_path,
        }
    }

    pub async fn list(&self) -> Vec<ConnectionConfig> {
        let conns = self.connections.lock().await;
        let mut list: Vec<ConnectionConfig> = conns.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    pub async fn get(&self, id: &str) -> Option<ConnectionConfig> {
        let conns = self.connections.lock().await;
        conns.get(id).cloned()
    }

    pub async fn create(&self, input: ConnectionInput) -> Result<ConnectionConfig, String> {
        let now = Utc::now();
        let config = ConnectionConfig {
            id: Uuid::new_v4().to_string(),
            name: input.name,
            host: input.host,
            port: input.port,
            username: input.username,
            domain: input.domain,
            color_accent: input.color_accent,
            tags: input.tags,
            display_width: input.display_width,
            display_height: input.display_height,
            mock: input.mock,
            created_at: now,
            updated_at: now,
            last_connected_at: None,
        };

        let mut conns = self.connections.lock().await;
        conns.insert(config.id.clone(), config.clone());
        drop(conns);
        self.save().await?;
        Ok(config)
    }

    pub async fn update(
        &self,
        id: &str,
        input: ConnectionInput,
    ) -> Result<Option<ConnectionConfig>, String> {
        let mut conns = self.connections.lock().await;
        if let Some(existing) = conns.get_mut(id) {
            existing.name = input.name;
            existing.host = input.host;
            existing.port = input.port;
            existing.username = input.username;
            existing.domain = input.domain;
            existing.color_accent = input.color_accent;
            existing.tags = input.tags;
            existing.display_width = input.display_width;
            existing.display_height = input.display_height;
            existing.mock = input.mock;
            existing.updated_at = Utc::now();
            let updated = existing.clone();
            drop(conns);
            self.save().await?;
            Ok(Some(updated))
        } else {
            Ok(None)
        }
    }

    pub async fn delete(&self, id: &str) -> Result<bool, String> {
        let mut conns = self.connections.lock().await;
        let removed = conns.remove(id).is_some();
        drop(conns);
        if removed {
            self.save().await?;
        }
        Ok(removed)
    }

    pub async fn update_last_connected(&self, id: &str) -> Result<(), String> {
        let mut conns = self.connections.lock().await;
        if let Some(conn) = conns.get_mut(id) {
            conn.last_connected_at = Some(Utc::now());
            drop(conns);
            self.save().await?;
        }
        Ok(())
    }

    async fn save(&self) -> Result<(), String> {
        let conns = self.connections.lock().await;
        let list: Vec<&ConnectionConfig> = conns.values().collect();
        let data = serde_json::to_string_pretty(&list)
            .map_err(|e| format!("Failed to serialize connections: {}", e))?;
        drop(conns);

        if let Some(parent) = self.file_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create data directory: {}", e))?;
        }
        tokio::fs::write(&self.file_path, data)
            .await
            .map_err(|e| format!("Failed to save connections: {}", e))?;
        Ok(())
    }
}

/// Parse a .rdp file and extract connection info.
pub fn parse_rdp_file(path: &str) -> Result<ConnectionInput, String> {
    // File size check (max 1MB)
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("Failed to read file metadata: {}", e))?;
    if metadata.len() > 1_048_576 {
        return Err("RDP file exceeds maximum size of 1MB".to_string());
    }

    let content =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read file: {}", e))?;

    let mut host = String::new();
    let mut port: u16 = 3389;
    let mut username = String::new();
    let mut domain = String::new();
    let mut width: Option<u32> = None;
    let mut height: Option<u32> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // RDP file format: "key:type:value"
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            continue;
        }

        let key = parts[0].trim();
        let value = parts[2].trim();

        match key {
            "full address" => {
                // May contain host:port
                if let Some((h, p)) = value.rsplit_once(':') {
                    host = h.to_string();
                    port = p.parse().unwrap_or(3389);
                } else {
                    host = value.to_string();
                }
            }
            "username" => username = value.to_string(),
            "domain" => domain = value.to_string(),
            "desktopwidth" => width = value.parse().ok(),
            "desktopheight" => height = value.parse().ok(),
            _ => {}
        }
    }

    if host.is_empty() {
        return Err("No host found in .rdp file".to_string());
    }

    let name = format!("{} (imported)", host);

    Ok(ConnectionInput {
        name,
        host,
        port,
        username,
        domain,
        color_accent: None,
        tags: vec!["imported".to_string()],
        display_width: width,
        display_height: height,
        mock: false,
        password: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_rdp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    #[test]
    fn test_parse_valid_rdp_file() {
        let f = write_temp_rdp(
            "full address:s:server.example.com\n\
             username:s:admin\n\
             domain:s:CORP\n",
        );
        let result = parse_rdp_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(result.host, "server.example.com");
        assert_eq!(result.port, 3389);
        assert_eq!(result.username, "admin");
        assert_eq!(result.domain, "CORP");
    }

    #[test]
    fn test_parse_rdp_file_with_port_in_address() {
        let f = write_temp_rdp("full address:s:myhost:3390\n");
        let result = parse_rdp_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(result.host, "myhost");
        assert_eq!(result.port, 3390);
    }

    #[test]
    fn test_parse_rdp_file_missing_host() {
        let f = write_temp_rdp("username:s:admin\n");
        let result = parse_rdp_file(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No host"));
    }

    #[test]
    fn test_parse_rdp_file_empty() {
        let f = write_temp_rdp("");
        let result = parse_rdp_file(f.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_rdp_file_with_resolution() {
        let f = write_temp_rdp(
            "full address:s:host.local\n\
             desktopwidth:i:1920\n\
             desktopheight:i:1080\n",
        );
        let result = parse_rdp_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(result.display_width, Some(1920));
        assert_eq!(result.display_height, Some(1080));
    }

    #[tokio::test]
    async fn test_create_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let input = ConnectionInput {
            name: "Test".into(),
            host: "host1".into(),
            port: 3389,
            username: "user".into(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: None,
            display_height: None,
            mock: false,
            password: None,
        };
        store.create(input).await.unwrap();
        let list = store.list().await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Test");
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let input = ConnectionInput {
            name: "MyConn".into(),
            host: "myhost".into(),
            port: 3390,
            username: "admin".into(),
            domain: "DOM".into(),
            color_accent: None,
            tags: vec!["tag1".into()],
            display_width: Some(1920),
            display_height: Some(1080),
            mock: false,
            password: None,
        };
        let created = store.create(input).await.unwrap();
        let fetched = store.get(&created.id).await.unwrap();
        assert_eq!(fetched.name, "MyConn");
        assert_eq!(fetched.host, "myhost");
        assert_eq!(fetched.port, 3390);
        assert_eq!(fetched.username, "admin");
        assert_eq!(fetched.domain, "DOM");
        assert_eq!(fetched.display_width, Some(1920));
    }

    #[tokio::test]
    async fn test_update_connection() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let input = ConnectionInput {
            name: "Old".into(),
            host: "old-host".into(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: None,
            display_height: None,
            mock: false,
            password: None,
        };
        let created = store.create(input).await.unwrap();
        let update_input = ConnectionInput {
            name: "New".into(),
            host: "new-host".into(),
            port: 3390,
            username: "newuser".into(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: None,
            display_height: None,
            mock: false,
            password: None,
        };
        let updated = store
            .update(&created.id, update_input)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.host, "new-host");
        assert_eq!(updated.port, 3390);
        assert_eq!(updated.username, "newuser");
    }

    #[tokio::test]
    async fn test_delete_connection() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let input = ConnectionInput {
            name: "ToDelete".into(),
            host: "host".into(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: None,
            display_height: None,
            mock: false,
            password: None,
        };
        let created = store.create(input).await.unwrap();
        let deleted = store.delete(&created.id).await.unwrap();
        assert!(deleted);
        let list = store.list().await;
        assert!(list.is_empty());
    }

    // ---- Task 3.1 characterization tests (persistence baseline, Req 1.2/1.5/6.1).
    // These pin CURRENT old-stack behavior for the baseline-coverage.md rows; they
    // do not approve it. Defect-marked tests reference their baseline row. ----

    fn sample_input(name: &str, host: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.into(),
            host: host.into(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: vec![],
            display_width: None,
            display_height: None,
            mock: false,
            password: None,
        }
    }

    #[tokio::test]
    async fn test_crud_round_trip_preserves_fields_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("connections.json");
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let mut rich = sample_input("Rich", "rich.example.com");
        rich.port = 3390;
        rich.username = "admin".into();
        rich.domain = "CORP".into();
        rich.color_accent = Some("#ff8800".into());
        rich.tags = vec!["prod".into(), "rdp".into()];
        rich.display_width = Some(2560);
        rich.display_height = Some(1440);
        let created = store.create(rich).await.unwrap();
        store.update_last_connected(&created.id).await.unwrap();
        store
            .create(sample_input("Alpha", "alpha.example.com"))
            .await
            .unwrap(); // list is name-sorted

        // Reopen from the same data dir: every field must survive the JSON round trip.
        drop(store);
        assert!(file_path.exists(), "connections.json must be written");
        let reopened = ConnectionStore::new(dir.path().to_path_buf());
        let reloaded = reopened.get(&created.id).await.unwrap();
        assert_eq!(reloaded.id, created.id);
        assert_eq!(reloaded.name, "Rich");
        assert_eq!(reloaded.host, "rich.example.com");
        assert_eq!(reloaded.port, 3390);
        assert_eq!(reloaded.username, "admin");
        assert_eq!(reloaded.domain, "CORP");
        assert_eq!(reloaded.color_accent, Some("#ff8800".to_string()));
        assert_eq!(reloaded.tags, vec!["prod".to_string(), "rdp".to_string()]);
        assert_eq!(reloaded.display_width, Some(2560));
        assert_eq!(reloaded.display_height, Some(1440));
        assert_eq!(reloaded.created_at, created.created_at);
        assert_eq!(reloaded.updated_at, created.updated_at);
        assert!(reloaded.last_connected_at.is_some());

        let list = reopened.list().await;
        assert_eq!(list.len(), 2);
        assert_eq!(
            list[0].name, "Alpha",
            "list() sorts by name (current behavior)"
        );
    }

    #[tokio::test]
    async fn test_created_file_is_json_array_without_password_key() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("connections.json");
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let mut input = sample_input("SecretHolder", "vault.example.com");
        input.password = Some("Sup3rSecret-Vault!".into());
        let created = store.create(input).await.unwrap();

        let raw = std::fs::read_to_string(&file_path).unwrap();
        assert!(
            !raw.contains("Sup3rSecret-Vault!"),
            "password value must never reach connections.json (Req 1.2/6.1)"
        );
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let array = value
            .as_array()
            .expect("connections.json is a v1 JSON array (current schema)");
        assert_eq!(array.len(), 1);
        assert!(
            array[0].get("password").is_none(),
            "persisted objects carry no password key"
        );
        assert_eq!(array[0]["id"].as_str(), Some(created.id.as_str()));
        assert_eq!(array[0]["host"].as_str(), Some("vault.example.com"));
    }

    // DEFECT B3-2 (baseline-coverage.md row 1.2): the update path silently drops the
    // submitted password (commands/connection.rs update_connection never touches the
    // CredentialStore). Pinned at the store seam: the file stays secret-free either way.
    #[tokio::test]
    async fn test_update_with_password_keeps_file_secret_free_defect_b3_2() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("connections.json");
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let created = store
            .create(sample_input("Editable", "edit.example.com"))
            .await
            .unwrap();
        let mut updated = sample_input("Editable", "edit2.example.com");
        updated.password = Some("EditedSecret-2026!".into());
        store.update(&created.id, updated).await.unwrap().unwrap();

        let raw = std::fs::read_to_string(&file_path).unwrap();
        assert!(!raw.contains("EditedSecret-2026!"));
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(value[0].get("password").is_none());
    }

    // DEFECT B3-1 (baseline-coverage.md row 1.1): the store update seam applies no
    // validation, so update_connection persists empty name/host (backend side). The
    // UI form validates, but nothing behind the IPC boundary does. Fix belongs to a
    // later group; this test documents the current acceptance, not the desired one.
    #[tokio::test]
    async fn test_update_accepts_empty_name_and_host_defect_b3_1() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let created = store
            .create(sample_input("Valid", "valid.example.com"))
            .await
            .unwrap();
        let result = store
            .update(&created.id, sample_input("", ""))
            .await
            .unwrap();
        let updated = result.expect("current behavior: update accepts empty name+host");
        assert_eq!(updated.name, "");
        assert_eq!(updated.host, "");
        // ...and the invalid values persist to disk.
        let reopened = ConnectionStore::new(dir.path().to_path_buf());
        let reloaded = reopened.get(&created.id).await.unwrap();
        assert_eq!(reloaded.name, "");
        assert_eq!(reloaded.host, "");
    }

    // DEFECT B3-4 (baseline-coverage.md row 6.1): a malformed connections.json loads
    // as an empty store (no error surfaced) and the NEXT save silently replaces the
    // file — user data loss. Write-blocked recovery is planned work (task 6.3); this
    // pins the current hazard without approving it.
    #[tokio::test]
    async fn test_corrupt_connections_file_replaced_by_next_save_defect_b3_4() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("connections.json");
        std::fs::write(&file_path, "{ not valid json !!").unwrap();

        let store = ConnectionStore::new(dir.path().to_path_buf());
        assert!(
            store.list().await.is_empty(),
            "current behavior: corrupt file loads as an empty store"
        );
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "{ not valid json !!",
            "load alone does not rewrite the file"
        );

        store
            .create(sample_input("AfterCorrupt", "fresh.example.com"))
            .await
            .unwrap();
        let raw = std::fs::read_to_string(&file_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw)
            .expect("current behavior: next save replaces the corrupt file (data loss)");
        assert_eq!(value.as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_delete_rewrites_file_without_deleted_connection() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("connections.json");
        let store = ConnectionStore::new(dir.path().to_path_buf());
        let keep = store
            .create(sample_input("Keep", "keep.example.com"))
            .await
            .unwrap();
        let drop_conn = store
            .create(sample_input("Drop", "drop.example.com"))
            .await
            .unwrap();
        assert!(store.delete(&drop_conn.id).await.unwrap());
        assert!(
            !store.delete(&drop_conn.id).await.unwrap(),
            "second delete: already gone"
        );

        let raw = std::fs::read_to_string(&file_path).unwrap();
        assert!(!raw.contains(&drop_conn.id), "deleted id leaves the file");
        let reopened = ConnectionStore::new(dir.path().to_path_buf());
        assert!(reopened.get(&keep.id).await.is_some());
        assert!(reopened.get(&drop_conn.id).await.is_none());
        assert_eq!(reopened.list().await.len(), 1);
    }

    // DEFECT B3-5 (baseline-coverage.md row 1.3): IPv6 literals in `full address`
    // are mangled — rsplit_once(':') treats the last colon-group separator as a
    // host:port split, yielding host "fe80:" and port 1.
    #[test]
    fn test_parse_rdp_ipv6_literal_mangled_defect_b3_5() {
        let f = write_temp_rdp("full address:s:fe80::1\n");
        let result = parse_rdp_file(f.path().to_str().unwrap()).unwrap();
        assert_eq!(result.host, "fe80:", "current (defective) host split");
        assert_eq!(result.port, 1, "current (defective) port parse");
    }

    // ---- Task 5.3: persisted mock flag (Req 12.2/12.10).
    //
    // The `mock` field marks a saved connection as a synthetic (mock) session
    // target. It is data only: production builds reject mock-session requests
    // at the connect boundary (see rdp::session::admit_mock_session); the
    // field exists so test-support builds — and later the v2 library model —
    // can persist it (design "LibraryStore v2", ConnectionConfigV2.mock).

    /// The mock flag round-trips through save/reload and update; legacy files
    /// without the key load with mock=false (serde default).
    #[tokio::test]
    async fn test_mock_flag_round_trips_and_legacy_files_default_false() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConnectionStore::new(dir.path().to_path_buf());

        let mut input = sample_input("mock flag", "mock-connection.invalid");
        input.mock = true;
        let created = store.create(input).await.unwrap();
        assert!(created.mock, "created connection carries the mock flag");

        // Reload from disk: the flag is persisted, not just in-memory.
        let reloaded = ConnectionStore::new(dir.path().to_path_buf());
        assert!(
            reloaded.get(&created.id).await.unwrap().mock,
            "persisted mock flag survives reload"
        );

        // Update is a full replacement: clearing the flag persists too.
        let cleared = sample_input("mock flag", "mock-connection.invalid");
        let updated = store
            .update(&created.id, cleared)
            .await
            .unwrap()
            .expect("connection exists");
        assert!(!updated.mock, "update replaces the mock flag");
        assert!(
            !ConnectionStore::new(dir.path().to_path_buf())
                .get(&created.id)
                .await
                .unwrap()
                .mock,
            "cleared mock flag persists"
        );

        // A legacy file without the key still loads, defaulting to false.
        let legacy_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            legacy_dir.path().join("connections.json"),
            r#"[{"id":"legacy","name":"Legacy","host":"legacy.invalid","port":3389,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]"#,
        )
        .unwrap();
        let legacy = ConnectionStore::new(legacy_dir.path().to_path_buf());
        assert!(
            !legacy.get("legacy").await.unwrap().mock,
            "legacy files default mock=false"
        );
    }
}
