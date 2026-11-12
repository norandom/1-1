use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshHost {
    pub name: String,
    pub hostname: String,
    pub port: Option<u16>,
    pub user: Option<String>,
}

/// Locate the user's SSH config file (~/.ssh/config).
fn ssh_config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Could not determine home directory".to_string())?;
    Ok(home.join(".ssh").join("config"))
}

/// Parse ~/.ssh/config and return a list of SSH hosts.
pub fn parse_ssh_config() -> Result<Vec<SshHost>, String> {
    let path = ssh_config_path()?;
    if !path.exists() {
        return Err(format!("SSH config not found at {:?}", path));
    }

    let content =
        fs::read_to_string(&path).map_err(|e| format!("Failed to read SSH config: {}", e))?;

    Ok(parse_ssh_config_content(&content))
}

/// Content-level SSH config parser (behavior unchanged).
/// Narrow test seam (task 3.1): the file location stays fixed at ~/.ssh/config,
/// which cannot be redirected for tests on Windows (Known Folder APIs), so the
/// parsing rules are pinned here against fixture content instead.
pub fn parse_ssh_config_content(content: &str) -> Vec<SshHost> {
    let mut hosts: Vec<SshHost> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_hostname: Option<String> = None;
    let mut current_port: Option<u16> = None;
    let mut current_user: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split on first whitespace or '='
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
            None => continue,
        };

        if key == "host" {
            // Flush previous host if it has a hostname
            if let (Some(name), Some(hostname)) = (&current_name, &current_hostname) {
                hosts.push(SshHost {
                    name: name.clone(),
                    hostname: hostname.clone(),
                    port: current_port,
                    user: current_user.clone(),
                });
            }

            // Skip wildcard hosts
            if value.contains('*') || value.contains('?') {
                current_name = None;
                current_hostname = None;
                current_port = None;
                current_user = None;
            } else {
                current_name = Some(value);
                current_hostname = None;
                current_port = None;
                current_user = None;
            }
        } else if current_name.is_some() {
            match key.as_str() {
                "hostname" => current_hostname = Some(value),
                "port" => current_port = value.parse().ok(),
                "user" => current_user = Some(value),
                _ => {}
            }
        }
    }

    // Flush the last host
    if let (Some(name), Some(hostname)) = (&current_name, &current_hostname) {
        hosts.push(SshHost {
            name: name.clone(),
            hostname: hostname.clone(),
            port: current_port,
            user: current_user.clone(),
        });
    }

    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Task 3.1 characterization tests (persistence baseline, Req 1.3).
    // These pin the CURRENT parser; the command layer's handling of the parsed
    // entries (port discarded, 3389 hardcoded) is recorded as defect B3-3 in
    // baseline-coverage.md — commands are not callable without an app harness. ----

    #[test]
    fn test_parse_basic_hosts_with_port_and_user() {
        let content = "# my hosts\n\
                      Host web\n\
                      \tHostName web.example.com\n\
                      \tPort 2222\n\
                      \tUser alice\n\
                      \n\
                      host lower\n\
                      hostname lower.example.com\n";
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 2, "EOF flush emits the trailing host");
        assert_eq!(hosts[0].name, "web");
        assert_eq!(hosts[0].hostname, "web.example.com");
        assert_eq!(hosts[0].port, Some(2222));
        assert_eq!(hosts[0].user, Some("alice".to_string()));
        // Keys are matched case-insensitively; unset fields stay None.
        assert_eq!(hosts[1].name, "lower");
        assert_eq!(hosts[1].hostname, "lower.example.com");
        assert_eq!(hosts[1].port, None);
        assert_eq!(hosts[1].user, None);
    }

    #[test]
    fn test_parse_compact_equals_syntax() {
        let content = "# comment line\n\
                      \n\
                      Host=db\n\
                      HostName=db.internal\n\
                      User=bob\n";
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "db");
        assert_eq!(hosts[0].hostname, "db.internal");
        assert_eq!(hosts[0].user, Some("bob".to_string()));
    }

    // DEFECT B3-6 (baseline-coverage.md row 1.3): the parser splits on the FIRST
    // whitespace-or-'=' character, so OpenSSH's `Key = Value` spaced form keeps the
    // '=' in the value: `Host = db` becomes name "= db" (and `Port = 2222` fails to
    // parse, since "= 2222" is not a u16). Pinned as-is; fix belongs to a later group.
    #[test]
    fn test_parse_spaced_equals_mangles_value_defect_b3_6() {
        let content = "Host = db\n\
                      HostName = db.internal\n\
                      Port = 2222\n";
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "= db", "current (defective) name");
        assert_eq!(hosts[0].hostname, "= db.internal");
        assert_eq!(hosts[0].port, None, "spaced Port value fails u16 parse");
    }

    #[test]
    fn test_parse_skips_wildcard_hosts() {
        let content = "Host *\n\
                      User global\n\
                      Host web\n\
                      HostName web.example.com\n";
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "web", "wildcard patterns are skipped");
    }

    #[test]
    fn test_parse_drops_host_without_hostname() {
        let content = "Host orphan\n\
                      User nobody\n\
                      Host web\n\
                      HostName web.example.com\n";
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "web", "hosts lacking HostName are dropped");
    }

    #[test]
    fn test_parse_invalid_port_becomes_none() {
        let content = "Host web\n\
                      HostName web.example.com\n\
                      Port not-a-number\n";
        let hosts = parse_ssh_config_content(content);
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].port, None, "unparseable Port is ignored (None)");
    }
}
