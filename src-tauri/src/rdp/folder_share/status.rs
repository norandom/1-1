//! What a session's shared folder actually did (rdp-folder-sharing task 5.3).
//!
//! The saved grant says what the user wants; this says what the connection
//! achieved, from protocol evidence only. Each connection attempt gets a fresh
//! [`ShareStatusCell`]. The connector records why no folder was offered, and
//! the drive's worker records the announcement, the server's answer and the
//! first successful file reply. A missing reply or a failed request never
//! advances the state. The latest failure is kept beside the state.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, MutexGuard};

/// The share's state on one connection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum ShareState {
    /// No folder is chosen for this host.
    #[default]
    Off,
    /// This build cannot share folders.
    Unsupported { reason: String },
    /// The address or the server's identity differs from the reviewed one.
    Unverified { reason: String },
    /// The chosen folder cannot be used right now.
    FolderUnavailable { reason: String },
    /// The share could not start for another reason.
    Unknown { reason: String },
    /// The drive channel is set up; the drive is not announced yet.
    Negotiating,
    /// The drive was announced to the server.
    Offered,
    /// The server acknowledged the drive.
    Accepted,
    /// A file operation on the drive succeeded.
    AccessObserved,
    /// The server refused the drive with this status.
    Refused { status: String },
    /// The connection that carried the drive ended.
    Ended,
}

/// The most recent file request that failed, for the session details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareFailure {
    pub operation: String,
    /// The NTSTATUS, as `0xC0000022`.
    pub status: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareStatus {
    #[serde(flatten)]
    pub state: ShareState,
    /// The drive name the server sees.
    pub name: Option<String>,
    pub read_only: Option<bool>,
    pub last_failure: Option<ShareFailure>,
}

/// One connection attempt's share status, shared by connector, worker and session.
#[derive(Debug, Clone, Default)]
pub(crate) struct ShareStatusCell(Arc<Mutex<ShareStatus>>);

impl ShareStatusCell {
    #[cfg(test)]
    pub(crate) fn new(state: ShareState) -> Self {
        Self(Arc::new(Mutex::new(ShareStatus {
            state,
            ..ShareStatus::default()
        })))
    }

    fn lock(&self) -> MutexGuard<'_, ShareStatus> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn get(&self) -> ShareStatus {
        self.lock().clone()
    }

    pub(crate) fn set(&self, state: ShareState) {
        self.lock().state = state;
    }

    /// Apply a change and log it when the state moved, so the app log shows
    /// how far the drive got.
    #[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
    fn change(&self, apply: impl FnOnce(&mut ShareStatus)) {
        let mut status = self.lock();
        let before = status.state.clone();
        apply(&mut status);
        if status.state != before {
            log::info!(
                "Folder share {:?}: {:?} -> {:?}",
                status.name.as_deref().unwrap_or_default(),
                before,
                status.state
            );
        }
    }

    /// The drive this connection offers, before it is announced.
    #[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
    pub(crate) fn negotiating(&self, name: &str, read_only: bool) {
        self.change(|status| {
            status.state = ShareState::Negotiating;
            status.name = Some(name.to_owned());
            status.read_only = Some(read_only);
        });
    }

    /// The device list with the drive went out.
    #[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
    pub(crate) fn offered(&self) {
        self.change(|status| {
            if status.state == ShareState::Negotiating {
                status.state = ShareState::Offered;
            }
        });
    }

    /// The server's answer to the announcement.
    #[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
    pub(crate) fn answered(&self, nt_status: u32) {
        self.change(|status| {
            if matches!(status.state, ShareState::Negotiating | ShareState::Offered) {
                status.state = if nt_status == 0 {
                    ShareState::Accepted
                } else {
                    ShareState::Refused {
                        status: hex(nt_status),
                    }
                };
            }
        });
    }

    /// A reply to a file request is on its way to the server.
    #[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
    pub(crate) fn replied(&self, major_function: Option<u32>, nt_status: u32) {
        self.change(|status| {
            if nt_status == 0 {
                if matches!(status.state, ShareState::Offered | ShareState::Accepted) {
                    status.state = ShareState::AccessObserved;
                }
            } else if is_failure(nt_status) {
                status.last_failure = Some(ShareFailure {
                    operation: operation_name(major_function).into(),
                    status: hex(nt_status),
                    description: describe(nt_status).into(),
                });
            }
        });
    }

    /// The channel that carried the drive is gone.
    #[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
    pub(crate) fn ended(&self) {
        self.change(|status| {
            if matches!(
                status.state,
                ShareState::Negotiating
                    | ShareState::Offered
                    | ShareState::Accepted
                    | ShareState::AccessObserved
            ) {
                status.state = ShareState::Ended;
            }
        });
    }
}
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn hex(nt_status: u32) -> String {
    format!("0x{nt_status:08X}")
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
/// Errors worth showing. Programs probe for missing files and read to the end
/// all the time, and Explorer asks for controls a drive need not support.
fn is_failure(nt_status: u32) -> bool {
    const ROUTINE: [u32; 7] = [
        0xC000_000F, // NO_SUCH_FILE
        0xC000_0034, // OBJECT_NAME_NOT_FOUND
        0xC000_003A, // OBJECT_PATH_NOT_FOUND
        0xC000_0011, // END_OF_FILE
        0xC000_00BB, // NOT_SUPPORTED
        0xC000_0010, // INVALID_DEVICE_REQUEST
        0xC000_0103, // NOT_A_DIRECTORY
    ];
    nt_status >> 30 == 0b11 && !ROUTINE.contains(&nt_status)
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn describe(nt_status: u32) -> &'static str {
    match nt_status {
        0xC000_0022 => "Access denied",
        0xC000_0043 => "In use by another program",
        0xC000_00A2 => "The share is read-only",
        0xC000_009A => "Too many requests at once",
        0xC000_0061 => "Needs a privilege the share does not use",
        0xC000_0121 => "Cannot be deleted",
        0xC000_0101 => "The folder is not empty",
        0xC000_0035 => "The name already exists",
        0xC000_007F => "The disk is full",
        0xC000_0033 => "The name is not allowed",
        0xC000_0001 => "Local file error",
        _ => "File operation failed",
    }
}

#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn operation_name(major_function: Option<u32>) -> &'static str {
    match major_function {
        Some(0x0) => "Open",
        Some(0x2) => "Close",
        Some(0x3) => "Read",
        Some(0x4) => "Write",
        Some(0x5) => "Get file information",
        Some(0x6) => "Change file information",
        Some(0x9) => "Flush",
        Some(0xA) => "Get volume information",
        Some(0xC) => "List folder",
        Some(0xE) => "Device control",
        Some(0x11) => "Lock",
        Some(0x14) => "Read permissions",
        Some(0x15) => "Change permissions",
        _ => "File operation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_advances_the_state_in_order_and_nothing_else_does() {
        let cell = ShareStatusCell::default();
        cell.negotiating("Work", false);
        // A success reply before the announcement proves nothing.
        cell.replied(Some(3), 0);
        assert_eq!(cell.get().state, ShareState::Negotiating);
        cell.offered();
        assert_eq!(cell.get().state, ShareState::Offered);
        // Failed reads never count as access.
        cell.replied(Some(3), 0xC000_0022);
        cell.answered(0);
        assert_eq!(cell.get().state, ShareState::Accepted);
        cell.replied(Some(3), 0xC000_0022);
        assert_eq!(cell.get().state, ShareState::Accepted);
        cell.replied(Some(3), 0);
        assert_eq!(cell.get().state, ShareState::AccessObserved);
        // A late answer does not undo observed access.
        cell.answered(0xC000_0001);
        assert_eq!(cell.get().state, ShareState::AccessObserved);
        cell.ended();
        let status = cell.get();
        assert_eq!(status.state, ShareState::Ended);
        assert_eq!(
            (status.name.as_deref(), status.read_only),
            (Some("Work"), Some(false))
        );
    }

    #[test]
    fn a_refusal_and_the_reasons_for_no_share_stay_distinct() {
        let refused = ShareStatusCell::default();
        refused.negotiating("Work", true);
        refused.offered();
        refused.answered(0xC000_0001);
        assert_eq!(
            refused.get().state,
            ShareState::Refused {
                status: "0xC0000001".into()
            }
        );
        refused.replied(Some(3), 0);
        assert!(
            matches!(refused.get().state, ShareState::Refused { .. }),
            "no access after refusal"
        );

        let unverified = ShareStatusCell::new(ShareState::Unverified {
            reason: "key".into(),
        });
        unverified.offered();
        unverified.ended();
        assert_eq!(
            unverified.get().state,
            ShareState::Unverified {
                reason: "key".into()
            }
        );
    }

    #[test]
    fn only_real_failures_are_kept_with_a_readable_description() {
        let cell = ShareStatusCell::default();
        cell.negotiating("Work", false);
        for routine in [0xC000_0034, 0xC000_0011, 0x8000_0006, 0xC000_00BB] {
            cell.replied(Some(0), routine);
        }
        assert_eq!(cell.get().last_failure, None);
        cell.replied(Some(4), 0xC000_0043);
        assert_eq!(
            cell.get().last_failure,
            Some(ShareFailure {
                operation: "Write".into(),
                status: "0xC0000043".into(),
                description: "In use by another program".into(),
            })
        );
    }

    #[test]
    fn the_status_reaches_the_window_as_flat_camel_case_json() {
        let cell = ShareStatusCell::default();
        cell.negotiating("Work", true);
        cell.replied(Some(4), 0xC000_00A2);
        let json = serde_json::to_value(cell.get()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "state": "negotiating", "name": "Work", "readOnly": true,
                "lastFailure": { "operation": "Write", "status": "0xC00000A2", "description": "The share is read-only" }
            })
        );
        let refused = serde_json::to_value(ShareState::Refused {
            status: "0xC0000001".into(),
        })
        .unwrap();
        assert_eq!(
            refused,
            serde_json::json!({ "state": "refused", "status": "0xC0000001" })
        );
    }
}
