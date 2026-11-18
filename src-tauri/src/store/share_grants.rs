//! Explicit folder grants for saved hosts (rdp-folder-sharing task 3.1).
//!
//! A grant authorizes one local folder for one saved host, at the endpoint
//! and server key the user confirmed. Grants live in a per-user file outside
//! the exported library. A missing file means no grants; an unreadable or
//! malformed file means sharing stays off for every host until it is
//! replaced. The library revokes grants inside its own mutation transaction
//! (see `LibraryStore::revoke_share_grants`), so a retarget or deletion can
//! never leave a grant behind for a later address change, ID reuse or failed
//! save to reactivate.

use crate::store::atomic_write::write_atomic;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};
use uuid::Uuid;

#[cfg(test)]
mod tests;

const FILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShareMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShareGrant {
    pub profile_id: String,
    /// Normalized `host:port` of the saved host when access was granted
    /// (the endpoint-pin key format).
    pub endpoint: String,
    /// SHA-256 of the confirmed server's DER SubjectPublicKeyInfo.
    pub spki_sha256: [u8; 32],
    /// Display path of the granted folder. Access binds to the volume and
    /// directory identity below, never to this path alone.
    pub root_path: PathBuf,
    pub volume_id: u64,
    pub directory_file_id: [u8; 16],
    /// Portable RDP preferred DOS name: 1–7 ASCII letters, digits, `_` or `-`.
    pub share_name: String,
    pub mode: ShareMode,
    /// New on every confirmation, so a session can tell a renewed grant
    /// from the one it attached.
    pub revision: Uuid,
}

impl ShareGrant {
    fn is_valid(&self) -> bool {
        !self.profile_id.is_empty()
            && !self.endpoint.is_empty()
            && !self.root_path.as_os_str().is_empty()
            && is_portable_share_name(&self.share_name)
    }
}

pub fn is_portable_share_name(name: &str) -> bool {
    (1..=7).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GrantFile {
    version: u32,
    grants: BTreeMap<String, ShareGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantError {
    /// The grant file exists but could not be read.
    Unreadable,
    /// The grant file is malformed. Sharing stays off until it is replaced.
    Corrupt,
    /// The change could not be saved; the previous state is retained.
    SaveFailed,
    /// The grant itself is not acceptable (empty fields or share name).
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the folder grant commands of task 3.2"
        )
    )]
    Invalid,
}

impl std::fmt::Display for GrantError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unreadable => "Could not read the saved folder shares.",
            Self::Corrupt => "The saved folder shares are invalid, so folder sharing is off.",
            Self::SaveFailed => "Could not save the folder share change.",
            Self::Invalid => "The folder share is incomplete or its name is not portable.",
        })
    }
}

impl std::error::Error for GrantError {}

enum Loaded {
    Grants(BTreeMap<String, ShareGrant>),
    Corrupt,
}

pub struct ShareGrantStore {
    path: PathBuf,
    /// Serializes this process's read-modify-write cycles.
    writer: Mutex<()>,
    #[cfg(test)]
    fail_next_write: std::sync::atomic::AtomicBool,
}

impl ShareGrantStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            writer: Mutex::new(()),
            #[cfg(test)]
            fail_next_write: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Test-only: make the next write fail before touching the file.
    #[cfg(test)]
    pub fn fail_next_write(&self) {
        self.fail_next_write
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn load(&self) -> Result<Loaded, GrantError> {
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Loaded::Grants(BTreeMap::new()));
            }
            Err(_) => return Err(GrantError::Unreadable),
        };
        let Ok(file) = serde_json::from_slice::<GrantFile>(&data) else {
            return Ok(Loaded::Corrupt);
        };
        let consistent = file.version == FILE_VERSION
            && file
                .grants
                .iter()
                .all(|(id, grant)| *id == grant.profile_id && grant.is_valid());
        Ok(if consistent {
            Loaded::Grants(file.grants)
        } else {
            Loaded::Corrupt
        })
    }

    /// The committed grant for a profile. `Err` means sharing stays off.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the folder grant commands of task 3.2"
        )
    )]
    pub fn get(&self, profile_id: &str) -> Result<Option<ShareGrant>, GrantError> {
        match self.load()? {
            Loaded::Grants(mut grants) => Ok(grants.remove(profile_id)),
            Loaded::Corrupt => Err(GrantError::Corrupt),
        }
    }

    /// Every committed grant, for reconciliation after a library reload.
    pub fn all(&self) -> Result<Vec<ShareGrant>, GrantError> {
        match self.load()? {
            Loaded::Grants(grants) => Ok(grants.into_values().collect()),
            Loaded::Corrupt => Err(GrantError::Corrupt),
        }
    }

    /// Save or replace a profile's single grant, atomically.
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "consumed by the folder grant commands of task 3.2"
        )
    )]
    pub fn put(&self, grant: ShareGrant) -> Result<(), GrantError> {
        if !grant.is_valid() {
            return Err(GrantError::Invalid);
        }
        let _writer = self.writer.lock().expect("share grant lock poisoned");
        let mut grants = match self.load()? {
            Loaded::Grants(grants) => grants,
            // Never overwrite a file we cannot interpret with a new grant.
            Loaded::Corrupt => return Err(GrantError::Corrupt),
        };
        grants.insert(grant.profile_id.clone(), grant);
        self.write(grants)
    }

    /// Durably remove the grants of these profiles. Idempotent: nothing is
    /// written when none of them has a grant. A malformed file is set aside
    /// and replaced by an empty one, because its grants can neither be
    /// trusted nor selectively removed.
    pub fn revoke<'a>(
        &self,
        profile_ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), GrantError> {
        let ids: Vec<&str> = profile_ids.into_iter().collect();
        if ids.is_empty() {
            return Ok(());
        }
        let _writer = self.writer.lock().expect("share grant lock poisoned");
        match self.load()? {
            Loaded::Grants(mut grants) => {
                let before = grants.len();
                grants.retain(|id, _| !ids.contains(&id.as_str()));
                if grants.len() == before {
                    return Ok(());
                }
                self.write(grants)
            }
            Loaded::Corrupt => {
                self.check_injected_failure()?;
                fs::rename(&self.path, corrupt_path(&self.path))
                    .map_err(|_| GrantError::SaveFailed)?;
                self.write(BTreeMap::new())
            }
        }
    }

    fn check_injected_failure(&self) -> Result<(), GrantError> {
        #[cfg(test)]
        if self
            .fail_next_write
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(GrantError::SaveFailed);
        }
        Ok(())
    }

    fn write(&self, grants: BTreeMap<String, ShareGrant>) -> Result<(), GrantError> {
        self.check_injected_failure()?;
        let data = serde_json::to_vec_pretty(&GrantFile {
            version: FILE_VERSION,
            grants,
        })
        .map_err(|_| GrantError::SaveFailed)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|_| GrantError::SaveFailed)?;
        }
        write_atomic(&self.path, &data).map_err(|_| GrantError::SaveFailed)
    }
}

fn corrupt_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".corrupt");
    path.with_file_name(name)
}
