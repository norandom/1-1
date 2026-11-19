//! Attaching the granted folders to one connection (rdp-folder-sharing 5.2).
//!
//! The drive channel is decided per connection attempt, after TLS and before
//! the client announces its static channels. Each saved folder must name this
//! address, the server must present the key the user reviewed for it, and the
//! folder must still be the one chosen. Folders that pass become drives of one
//! channel; any that do not are left out with their reason, and with none left
//! the connection goes ahead without the channel, so the desktop never depends
//! on a share.

use super::status::{ShareState, ShareStatusCell};
use super::worker::{Limits, WorkerBackend};
use super::{BoundRoot, ShareBackend};
use crate::rdp::peer_identity::{self, ConfirmedPin, Endpoint, ObservedPeer, SignedTls};
use crate::store::share_grants::{ShareGrant, ShareMode};
use ironrdp_rdpdr::Rdpdr;
use ironrdp_rdpdr_native::{
    FIRST_NAMED_DRIVE_DEVICE_ID, RedirectedDrive, WindowsRdpdrBackendFactory,
};
use std::{collections::HashMap, path::PathBuf, sync::Arc};

/// The RDPDR device ID of a host's first folder; the next ones follow it.
pub(crate) const FIRST_SHARE_DEVICE_ID: u32 = FIRST_NAMED_DRIVE_DEVICE_ID;
/// The client name the redirection channel reports to the server.
const COMPUTER_NAME: &str = "1-1";

/// Why a folder is left out of this connection. Shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NotOffered {
    /// The saved address no longer matches the grant's.
    AddressChanged,
    /// The server presented a different key than the one reviewed.
    IdentityChanged,
    /// The folder is gone, replaced, a link, or busy.
    FolderUnavailable(String),
    /// The local drive backend could not start.
    DriveFailed(String),
}

impl NotOffered {
    /// How the session details show this outcome.
    pub(crate) fn state(&self) -> ShareState {
        let reason = self.to_string();
        match self {
            Self::AddressChanged | Self::IdentityChanged => ShareState::Unverified { reason },
            Self::FolderUnavailable(_) => ShareState::FolderUnavailable { reason },
            Self::DriveFailed(_) => ShareState::Unknown { reason },
        }
    }
}

impl std::fmt::Display for NotOffered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AddressChanged => f.write_str(
                "The host's address changed since the folder was chosen. Choose the folder again.",
            ),
            Self::IdentityChanged => f.write_str(
                "The server's identity changed since the folder was chosen. Review the host and choose the folder again.",
            ),
            Self::FolderUnavailable(cause) => write!(f, "{cause}"),
            Self::DriveFailed(cause) => write!(f, "The shared folder could not be opened: {cause}"),
        }
    }
}

/// The connection's drive channel, if any folder qualifies, and each
/// folder's status in grant order.
pub(crate) async fn drive_channel<S>(
    tls: &SignedTls<S>,
    endpoint: &Endpoint,
    grants: Vec<ShareGrant>,
) -> (Option<Rdpdr>, Vec<ShareStatusCell>) {
    let statuses: Vec<ShareStatusCell> =
        grants.iter().map(|_| ShareStatusCell::default()).collect();
    let mut verified = Vec::new();
    for (index, grant) in grants.into_iter().enumerate() {
        let device_id = FIRST_SHARE_DEVICE_ID + index as u32;
        match verify(tls, endpoint, &grant).await {
            Ok(()) => verified.push((device_id, grant, statuses[index].clone())),
            Err(reason) => leave_out(&grant, &statuses[index], reason),
        }
    }
    if verified.is_empty() {
        return (None, statuses);
    }
    // Binding opens every folder on each path; keep that off the async runtime.
    let rdpdr = tokio::task::spawn_blocking(move || start(verified))
        .await
        .ok()
        .flatten();
    (rdpdr, statuses)
}

fn leave_out(grant: &ShareGrant, status: &ShareStatusCell, reason: NotOffered) {
    log::info!(
        "Not offering the shared folder {:?}: {reason}",
        grant.share_name
    );
    status.set(reason.state());
}

async fn verify<S>(
    tls: &SignedTls<S>,
    endpoint: &Endpoint,
    grant: &ShareGrant,
) -> Result<(), NotOffered> {
    if peer_identity::endpoint_key(endpoint) != grant.endpoint {
        return Err(NotOffered::AddressChanged);
    }
    let reviewed = ConfirmedPin::from_operator_confirmation(ObservedPeer {
        endpoint: endpoint.clone(),
        spki_sha256: grant.spki_sha256,
    });
    tls.verify(Some(&reviewed))
        .await
        .map(drop)
        .map_err(|_| NotOffered::IdentityChanged)
}

/// Bind the verified folders and start one fresh worker for all of them.
fn start(verified: Vec<(u32, ShareGrant, ShareStatusCell)>) -> Option<Rdpdr> {
    let mut drives = Vec::new();
    let mut bound = Vec::new();
    let mut roots: HashMap<u32, PathBuf> = HashMap::new();
    let mut statuses = HashMap::new();
    let mut names = Vec::new();
    for (device_id, grant, status) in verified {
        let root = match BoundRoot::bind(&grant.root_path, grant.volume_id, grant.directory_file_id)
        {
            Ok(root) => root,
            Err(error) => {
                leave_out(
                    &grant,
                    &status,
                    NotOffered::FolderUnavailable(error.to_string()),
                );
                continue;
            }
        };
        let read_only = grant.mode == ShareMode::ReadOnly;
        match RedirectedDrive::new(device_id, &grant.share_name, root.path(), read_only) {
            Ok(drive) => drives.push(drive),
            Err(error) => {
                leave_out(&grant, &status, NotOffered::DriveFailed(error.to_string()));
                continue;
            }
        }
        log::info!(
            "Offering the shared folder {:?} on this connection",
            grant.share_name
        );
        status.negotiating(&grant.share_name, read_only);
        roots.insert(device_id, root.path().to_path_buf());
        names.push((device_id, grant.share_name));
        statuses.insert(device_id, status);
        bound.push(root);
    }
    if drives.is_empty() {
        return None;
    }
    let factory = match WindowsRdpdrBackendFactory::from_drives(drives) {
        Ok(factory) => factory,
        Err(error) => {
            for status in statuses.values() {
                status.set(NotOffered::DriveFailed(error.to_string()).state());
            }
            return None;
        }
    };
    let share = ShareBackend::with_roots(Box::new(factory.build()), roots);
    // The worker keeps every guard until the channel ends.
    let worker = WorkerBackend::spawn(Box::new(share), bound, Limits::default(), Arc::new(|| {}))
        .with_statuses(statuses);
    Some(Rdpdr::new(Box::new(worker), COMPUTER_NAME.into()).with_drives(Some(names)))
}

#[cfg(test)]
mod tests;
