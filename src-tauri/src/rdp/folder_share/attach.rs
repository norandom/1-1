//! Attaching the granted folder to one connection (rdp-folder-sharing task 5.2).
//!
//! The drive channel is decided per connection attempt, after TLS and before
//! the client announces its static channels. The saved grant must name this
//! address, the server must present the key the user reviewed, and the folder
//! must still be the one chosen. Otherwise the connection goes ahead without
//! the channel or a device name, so the desktop never depends on the share.

use super::status::{ShareState, ShareStatusCell};
use super::worker::{Limits, WorkerBackend};
use super::{BoundRoot, ShareBackend};
use crate::rdp::peer_identity::{self, ConfirmedPin, Endpoint, ObservedPeer, SignedTls};
use crate::store::share_grants::{ShareGrant, ShareMode};
use ironrdp_rdpdr::Rdpdr;
use ironrdp_rdpdr_native::{
    FIRST_NAMED_DRIVE_DEVICE_ID, RedirectedDrive, WindowsRdpdrBackendFactory,
};
use std::sync::Arc;

/// The RDPDR device ID of the shared folder; one drive per connection.
pub(crate) const SHARE_DEVICE_ID: u32 = FIRST_NAMED_DRIVE_DEVICE_ID;
/// The client name the redirection channel reports to the server.
const COMPUTER_NAME: &str = "1-1";

/// Why a connection goes ahead without its folder. Shown to the user.
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

/// The redirection channel for `grant`, if this connection qualifies.
pub(crate) async fn drive_channel<S>(
    tls: &SignedTls<S>,
    endpoint: &Endpoint,
    grant: ShareGrant,
    status: ShareStatusCell,
) -> Result<Rdpdr, NotOffered> {
    if peer_identity::endpoint_key(endpoint) != grant.endpoint {
        return Err(NotOffered::AddressChanged);
    }
    let reviewed = ConfirmedPin::from_operator_confirmation(ObservedPeer {
        endpoint: endpoint.clone(),
        spki_sha256: grant.spki_sha256,
    });
    tls.verify(Some(&reviewed))
        .await
        .map_err(|_| NotOffered::IdentityChanged)?;
    let name = grant.share_name.clone();
    let read_only = grant.mode == ShareMode::ReadOnly;
    // Binding opens every folder on the path; keep that off the async runtime.
    let worker = tokio::task::spawn_blocking(move || start(grant))
        .await
        .map_err(|error| NotOffered::DriveFailed(error.to_string()))??;
    status.negotiating(&name, read_only);
    let worker = worker.with_status(status);
    Ok(Rdpdr::new(Box::new(worker), COMPUTER_NAME.into())
        .with_drives(Some(vec![(SHARE_DEVICE_ID, name)])))
}

/// Bind the folder and start a fresh worker for this connection attempt.
fn start(grant: ShareGrant) -> Result<WorkerBackend, NotOffered> {
    let bound = BoundRoot::bind(&grant.root_path, grant.volume_id, grant.directory_file_id)
        .map_err(|error| NotOffered::FolderUnavailable(error.to_string()))?;
    let read_only = grant.mode == ShareMode::ReadOnly;
    let drive = RedirectedDrive::new(SHARE_DEVICE_ID, &grant.share_name, bound.path(), read_only)
        .map_err(|error| NotOffered::DriveFailed(error.to_string()))?;
    let native = WindowsRdpdrBackendFactory::new(drive).build();
    let share = ShareBackend::new(Box::new(native), bound.path().to_path_buf());
    // The worker keeps the guards until the channel ends.
    Ok(WorkerBackend::spawn(
        Box::new(share),
        bound,
        Limits::default(),
        Arc::new(|| {}),
    ))
}

#[cfg(test)]
mod tests;
