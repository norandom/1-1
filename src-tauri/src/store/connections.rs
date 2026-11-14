//! Connection data model shared by the v1 legacy format and the unified
//! v2 library document (task 6.6): the persisted [`ConnectionConfig`], the
//! CRUD/import wire payload [`ConnectionInput`] (serde-defaulted ownership
//! fields), and the import parsers/mappers (`.rdp` files, SSH configs).
//!
//! The v1 `ConnectionStore` (bare-array `connections.json` owner) was
//! RETIRED by task 6.6: `store::library::LibraryStore` is now the single
//! owner of `connections.json` (v2 document) and serves all connection
//! CRUD through its single-lock transaction core (see `store::library`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// connect boundary (`rdp::session::admit_mock_session`) and reject
    /// mock-flagged CREATE requests at the store boundary (task 6.6); no
    /// production write path sets it. Test-support builds seed and persist
    /// it for the harness.
    #[serde(default)]
    pub mock: bool,
    /// Task 6.1 (Req 10.1/10.8): owning library in the v2 library document
    /// (design "LibraryStore v2", ConnectionConfigV2). Empty for v1-era
    /// records until the migration (task 6.3) assigns a default library;
    /// `store::library::LibraryDocument::validate` rejects unowned entries.
    #[serde(default)]
    pub library_id: String,
    /// Task 6.1: optional folder placement inside the owning library.
    #[serde(default)]
    pub folder_id: Option<String>,
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
    /// Rejected on create in production builds (Req 12.10).
    #[serde(default)]
    pub mock: bool,
    /// Task 6.6 (design "Data Contracts & Integration"): CRUD payloads gain
    /// library/folder ownership. Empty `library_id` routes the connection
    /// to the store's default library; a `folder_id` without a library
    /// resolves the folder's own library. Ownership is assigned on CREATE
    /// only — updates preserve placement (moves go through `move_node`).
    #[serde(default)]
    pub library_id: String,
    #[serde(default)]
    pub folder_id: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

fn default_port() -> u16 {
    3389
}

/// Map a parsed SSH-config host entry into a connection input (Req 1.3):
/// host, PORT (where available — regression for baseline defect B3-3,
/// which hardcoded 3389 and discarded the parsed Port), username, the
/// `ssh-import` tag, and a hard-coded non-mock flag (Req 12.10: imports
/// never create synthetic connections).
pub fn ssh_host_to_connection_input(host: &crate::utils::ssh_config::SshHost) -> ConnectionInput {
    ConnectionInput {
        name: host.name.clone(),
        host: host.hostname.clone(),
        port: host.port.unwrap_or(3389),
        username: host.user.clone().unwrap_or_default(),
        domain: String::new(),
        color_accent: None,
        tags: vec!["ssh-import".to_string()],
        display_width: None,
        display_height: None,
        mock: false,
        library_id: String::new(),
        folder_id: None,
        password: None,
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
        library_id: String::new(),
        folder_id: None,
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
        // Task 6.6: .rdp imports default to the store's default-library
        // destination and are always non-mock (Req 1.3/12.10).
        assert_eq!(result.library_id, "");
        assert_eq!(result.folder_id, None);
        assert!(!result.mock);
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

    // The task 3.1/5.3 ConnectionStore CRUD/persistence tests were PORTED
    // to the unified v2 store (task 6.6): see `store::library` modules
    // `connection_crud_tests` (round-trips, validation incl. the flipped
    // B3-1 regression, secret-free file, delete, duplicate),
    // `unification_tests` (the fail-closed replacement for defect B3-4's
    // corrupt-file characterization) and the cfg-gated mock-policy modules
    // (the v1 mock round-trip, split per build). The .rdp parser tests
    // above stay: the parser itself is unchanged (incl. the pinned B3-5,
    // restored verbatim — baseline-coverage.md row 1.3 evidence).

    // ---- Task 6.6 (Req 1.3): v2 CRUD payload ownership + import mapping ---

    #[test]
    fn test_connection_input_defaults_ownership_when_fields_are_absent() {
        // The wire payload (create/update) accepts v1-era bodies without
        // libraryId/folderId: serde defaults route them to the default
        // library destination (design "Data Contracts & Integration").
        let input: ConnectionInput =
            serde_json::from_str(r#"{"name":"Legacy Body","host":"legacy.invalid","port":3389}"#)
                .unwrap();
        assert_eq!(input.library_id, "");
        assert_eq!(input.folder_id, None);

        let round: ConnectionInput =
            serde_json::from_str(&serde_json::to_string(&input).unwrap()).unwrap();
        assert_eq!(round.library_id, "");
        assert_eq!(round.folder_id, None);
    }

    #[test]
    fn test_connection_input_carries_library_and_folder_ownership() {
        let input: ConnectionInput = serde_json::from_str(
            r#"{"name":"Filed","host":"filed.invalid","port":3390,
                "libraryId":"11111111-1111-4111-8111-111111111111",
                "folderId":"22222222-2222-4222-8222-222222222222"}"#,
        )
        .unwrap();
        assert_eq!(input.library_id, "11111111-1111-4111-8111-111111111111");
        assert_eq!(
            input.folder_id.as_deref(),
            Some("22222222-2222-4222-8222-222222222222")
        );
    }

    /// Regression for baseline defect B3-3: the SSH import command used to
    /// hardcode port 3389 and discard the parsed `Port`. The mapping helper
    /// now carries the parsed port (defaulting to 3389) and keeps imports
    /// non-mock (Req 1.3, 12.10).
    #[test]
    fn test_ssh_host_to_connection_input_uses_the_parsed_port_regression_b3_3() {
        use crate::utils::ssh_config::SshHost;

        let with_port = SshHost {
            name: "db".into(),
            hostname: "db.example.com".into(),
            port: Some(2222),
            user: Some("ops".into()),
        };
        let input = ssh_host_to_connection_input(&with_port);
        assert_eq!(input.name, "db");
        assert_eq!(input.host, "db.example.com");
        assert_eq!(input.port, 2222, "parsed SSH Port is not discarded");
        assert_eq!(input.username, "ops");
        assert!(!input.mock, "imports are always non-mock");
        assert_eq!(input.tags, vec!["ssh-import".to_string()]);
        assert_eq!(input.password, None);

        let without_port = SshHost {
            name: "plain".into(),
            hostname: "plain.example.com".into(),
            port: None,
            user: None,
        };
        let default_input = ssh_host_to_connection_input(&without_port);
        assert_eq!(default_input.port, 3389);
        assert_eq!(default_input.username, "");
    }
}
