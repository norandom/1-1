//! Folder grant commands (rdp-folder-sharing task 3.2).
//!
//! Choosing a folder offers a short-lived draft that binds the saved host as
//! selected, the server key proven trusted on a fresh connection (a valid
//! chain or the operator's pin) and the folder's volume and file identity.
//! Confirmation saves the grant inside the library lock, so a concurrent
//! address change either invalidates the draft or revokes the new grant.
//! Trusting a server key never grants a folder by itself.

use crate::rdp::peer_identity::{
    ConnectionTarget, ObservedPeer, discover_trusted, endpoint_key, fingerprint, pins::PinStore,
};
use crate::store::connections::ConnectionConfig;
use crate::store::library::LibraryStore;
use crate::store::share_grants::{
    GrantError, ShareGrant, ShareGrantStore, ShareMode, is_portable_share_name,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::State;
use uuid::Uuid;

#[cfg(test)]
mod tests;

const DRAFT_LIFETIME: Duration = Duration::from_secs(120);
const STALE: &str = "This folder choice has expired or closed. Choose the folder again.";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GrantView {
    endpoint: String,
    fingerprint: String,
    folder: String,
    share_name: String,
    mode: ShareMode,
}

/// The committed state of one host's folder share.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderShareView {
    /// Only the Windows build can share a folder.
    supported: bool,
    grant: Option<GrantView>,
    /// Why sharing is off although a grant may exist (unreadable store).
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FolderShareDraftView {
    selection: Uuid,
    endpoint: String,
    fingerprint: String,
    folder: String,
    suggested_share_name: String,
}

struct Draft {
    host: Arc<ConnectionConfig>,
    peer: ObservedPeer,
    folder: PathBuf,
    identity: native::FolderIdentity,
    expires: Instant,
}

#[derive(Default)]
pub(crate) struct FolderShareDrafts(Mutex<HashMap<Uuid, Draft>>);

impl FolderShareDrafts {
    fn insert(&self, draft: Draft) -> Uuid {
        let selection = Uuid::new_v4();
        let mut drafts = self.0.lock().expect("folder share drafts poisoned");
        let now = Instant::now();
        drafts.retain(|_, draft| draft.expires > now);
        drafts.insert(selection, draft);
        selection
    }

    fn take(&self, selection: Uuid) -> Option<Draft> {
        self.0
            .lock()
            .expect("folder share drafts poisoned")
            .remove(&selection)
    }
}

fn view(grants: &ShareGrantStore, profile_id: &str) -> FolderShareView {
    let (grant, error) = match grants.get(profile_id) {
        Ok(grant) => (grant, None),
        Err(error) => (None, Some(error.to_string())),
    };
    FolderShareView {
        supported: native::SUPPORTED,
        grant: grant.map(|grant| GrantView {
            endpoint: grant.endpoint,
            fingerprint: fingerprint(&grant.spki_sha256),
            folder: grant.root_path.display().to_string(),
            share_name: grant.share_name,
            mode: grant.mode,
        }),
        error,
    }
}

/// A portable default name from the folder's own name: letters, digits,
/// `_` and `-`, at most seven, or `share`.
fn suggested_share_name(folder: &std::path::Path) -> String {
    let name: String = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(7)
        .collect();
    if name.is_empty() {
        "share".into()
    } else {
        name
    }
}

fn trust_message(error: io::Error, endpoint: &str) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => format!(
            "The server key for {endpoint} changed since you confirmed it. Review the server identity again, then share the folder."
        ),
        io::ErrorKind::TimedOut
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::NotFound
        | io::ErrorKind::AddrNotAvailable => format!("Could not reach {endpoint}: {error}"),
        _ => format!(
            "The server identity for {endpoint} is not trusted yet. Use Review server identity first, then share the folder. ({error})"
        ),
    }
}

#[tauri::command]
pub(crate) fn get_folder_share(
    profile_id: String,
    grants: State<'_, Arc<ShareGrantStore>>,
) -> FolderShareView {
    view(&grants, &profile_id)
}

/// Prove the server key, then open the native folder picker. `None` when the
/// user cancels the picker.
#[tauri::command]
pub(crate) async fn select_share_folder(
    profile_id: String,
    window: tauri::WebviewWindow,
    library: State<'_, LibraryStore>,
    pins: State<'_, PinStore>,
    drafts: State<'_, FolderShareDrafts>,
) -> Result<Option<FolderShareDraftView>, String> {
    if !native::SUPPORTED {
        return Err(native::UNSUPPORTED.into());
    }
    let host = library
        .select_host_for_review(&profile_id)
        .map_err(|e| e.to_string())?;
    let target = ConnectionTarget::new(&host.host, host.port).map_err(|e| e.to_string())?;
    let endpoint = endpoint_key(&target.endpoint);
    let pin = pins.lookup(&target.endpoint).map_err(|e| e.to_string())?;
    let peer = discover_trusted(&target, pin.as_ref())
        .await
        .map_err(|error| trust_message(error, &endpoint))?;
    let owner = native::owner_window(&window);
    let Some(folder) = tokio::task::spawn_blocking(move || native::pick_folder(owner))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not open the folder picker: {e}"))?
    else {
        return Ok(None);
    };
    let identity = native::inspect(&folder)?;
    let suggested = suggested_share_name(&folder);
    let display = folder.display().to_string();
    let fingerprint = fingerprint(&peer.spki_sha256);
    let selection = drafts.insert(Draft {
        host,
        peer,
        folder,
        identity,
        expires: Instant::now() + DRAFT_LIFETIME,
    });
    Ok(Some(FolderShareDraftView {
        selection,
        endpoint,
        fingerprint,
        folder: display,
        suggested_share_name: suggested,
    }))
}

#[tauri::command]
pub(crate) fn confirm_folder_share(
    selection: Uuid,
    share_name: String,
    mode: ShareMode,
    library: State<'_, LibraryStore>,
    grants: State<'_, Arc<ShareGrantStore>>,
    drafts: State<'_, FolderShareDrafts>,
) -> Result<FolderShareView, String> {
    confirm(&drafts, &library, &grants, selection, &share_name, mode)
}

fn confirm(
    drafts: &FolderShareDrafts,
    library: &LibraryStore,
    grants: &ShareGrantStore,
    selection: Uuid,
    share_name: &str,
    mode: ShareMode,
) -> Result<FolderShareView, String> {
    if !is_portable_share_name(share_name) {
        return Err("Use 1 to 7 letters, digits, _ or - for the share name.".into());
    }
    let draft = drafts.take(selection).ok_or(STALE)?;
    if Instant::now() >= draft.expires {
        return Err(STALE.into());
    }
    // The same directory, not just the same path, must still be there.
    if native::inspect(&draft.folder)? != draft.identity {
        return Err("The folder changed since you chose it. Choose it again.".into());
    }
    let grant = ShareGrant {
        profile_id: draft.host.id.clone(),
        endpoint: endpoint_key(&draft.peer.endpoint),
        spki_sha256: draft.peer.spki_sha256,
        root_path: draft.folder.clone(),
        volume_id: draft.identity.volume_id,
        directory_file_id: draft.identity.file_id,
        share_name: share_name.to_owned(),
        mode,
        revision: Uuid::new_v4(),
    };
    // Under the library lock: an edit committed since the draft fails this;
    // one committed after it revokes the grant inside its own transaction.
    let reached_save = std::cell::Cell::new(false);
    library
        .with_reviewed_host(&draft.host, || {
            reached_save.set(true);
            grants.put(grant).map_err(|error| match error {
                GrantError::Corrupt => io::Error::other(
                    "The saved folder shares are invalid. Stop sharing on this host to reset them, then try again.",
                ),
                other => io::Error::other(other.to_string()),
            })
        })
        .map_err(|error| {
            if reached_save.get() {
                error.to_string()
            } else {
                "The saved host changed since you chose the folder. Choose the folder again.".into()
            }
        })?;
    Ok(view(grants, &draft.host.id))
}

#[tauri::command]
pub(crate) fn cancel_folder_share(selection: Uuid, drafts: State<'_, FolderShareDrafts>) {
    drafts.take(selection);
}

/// Idempotent. Takes effect on the next connection.
#[tauri::command]
pub(crate) fn revoke_folder_share(
    profile_id: String,
    grants: State<'_, Arc<ShareGrantStore>>,
) -> Result<FolderShareView, String> {
    grants
        .revoke([profile_id.as_str()])
        .map_err(|e| e.to_string())?;
    Ok(view(&grants, &profile_id))
}

#[cfg(windows)]
mod native {
    use std::{
        ffi::c_void,
        io, mem,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::{Component, Path, PathBuf, Prefix},
    };
    use windows::Win32::Foundation::{HANDLE, HWND};
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_READ_ATTRIBUTES, FileAttributeTagInfo, FileIdInfo, GetDriveTypeW,
        GetFileInformationByHandleEx,
    };
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoTaskMemFree, CoUninitialize,
    };
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH,
    };
    use windows::core::HSTRING;

    use super::native_paths::{file_system_name, resolves_to};

    pub(super) const SUPPORTED: bool = true;
    pub(super) const UNSUPPORTED: &str = "";

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    /// HRESULT_FROM_WIN32(ERROR_CANCELLED), returned when the user closes the dialog.
    const CANCELLED: i32 = 0x8007_04C7_u32 as i32;
    const SHARE_ALL: u32 = 0x7; // FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct FolderIdentity {
        pub(super) volume_id: u64,
        pub(super) file_id: [u8; 16],
    }

    pub(super) fn owner_window(window: &tauri::WebviewWindow) -> Option<isize> {
        window.hwnd().ok().map(|hwnd| hwnd.0 as isize)
    }

    pub(super) fn pick_folder(owner: Option<isize>) -> io::Result<Option<PathBuf>> {
        // SAFETY: COM is initialized and uninitialized on this blocking thread
        // around the dialog's whole lifetime, and every returned string is
        // copied before it is freed with CoTaskMemFree.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(io::Error::other)?;
            let picked = (|| -> windows::core::Result<Option<PathBuf>> {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
                dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
                dialog.SetTitle(&HSTRING::from("Choose a folder to share"))?;
                if let Err(error) = dialog.Show(owner.map(|hwnd| HWND(hwnd as *mut c_void))) {
                    return if error.code().0 == CANCELLED {
                        Ok(None)
                    } else {
                        Err(error)
                    };
                }
                let name = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
                let path = name.to_string();
                CoTaskMemFree(Some(name.0 as *const c_void));
                Ok(Some(PathBuf::from(path.map_err(|_| {
                    windows::core::Error::from(windows::core::HRESULT(0x8007_007B_u32 as i32))
                })?)))
            })();
            CoUninitialize();
            picked.map_err(io::Error::other)
        }
    }

    /// A local drive-letter subdirectory that is not itself a link, with the
    /// volume and file identity the grant binds to.
    pub(super) fn inspect(path: &Path) -> Result<FolderIdentity, String> {
        let mut components = path.components();
        let letter = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) => letter,
                _ => {
                    return Err("Choose a folder on a local drive letter, such as D:\\work.".into());
                }
            },
            _ => return Err("Choose a folder on a local drive letter, such as D:\\work.".into()),
        };
        if components.next() != Some(Component::RootDir) || components.next().is_none() {
            return Err("Choose a folder inside a drive, not the whole drive.".into());
        }
        let root = HSTRING::from(format!("{}:\\", letter as char));
        // SAFETY: GetDriveTypeW only reads the NUL-terminated root path.
        let kind = unsafe { GetDriveTypeW(&root) };
        if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE {
            return Err(
                "Choose a folder on a local disk. Network and virtual drives cannot be shared."
                    .into(),
            );
        }
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(SHARE_ALL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(|e| format!("Could not open the folder: {e}"))?;
        let handle = HANDLE(file.as_raw_handle());
        let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
        let mut id = FILE_ID_INFO::default();
        // SAFETY: Each buffer is a correctly sized, owned struct for its
        // information class, and the handle stays open until `file` drops.
        unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                (&raw mut tag).cast(),
                mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
            .map_err(|e| format!("Could not read the folder: {e}"))?;
            GetFileInformationByHandleEx(
                handle,
                FileIdInfo,
                (&raw mut id).cast(),
                mem::size_of::<FILE_ID_INFO>() as u32,
            )
            .map_err(|e| format!("Could not read the folder identity: {e}"))?;
        }
        if tag.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("Choose a folder, not a file.".into());
        }
        if tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Folders that are links or junctions cannot be shared.".into());
        }
        // A link or junction in a parent folder, or a substituted drive, is
        // followed silently and can lead elsewhere, even to a network share.
        // The opened directory must resolve to the path that was chosen.
        if !resolves_to(handle, path)? {
            return Err("Folders reached through a link, junction or substituted drive cannot be shared. Choose the folder at its real location.".into());
        }
        // Only these keep a directory's file ID stable across restarts.
        let file_system = file_system_name(handle)?;
        if !file_system.eq_ignore_ascii_case("NTFS") && !file_system.eq_ignore_ascii_case("ReFS") {
            return Err(format!(
                "Choose a folder on an NTFS or ReFS drive. This {file_system} drive cannot keep the folder's identity stable."
            ));
        }
        Ok(FolderIdentity {
            volume_id: id.VolumeSerialNumber,
            file_id: id.FileId.Identifier,
        })
    }
}

#[cfg(windows)]
mod native_paths {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;

    /// Shared with the drive binding, which checks the same thing.
    pub(super) use crate::rdp::folder_share::resolves_to;

    pub(super) fn file_system_name(handle: HANDLE) -> Result<String, String> {
        let mut name = [0u16; 64];
        // SAFETY: Only the file system name buffer is requested, with its length.
        unsafe { GetVolumeInformationByHandleW(handle, None, None, None, None, Some(&mut name)) }
            .map_err(|e| format!("Could not read the folder's drive: {e}"))?;
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        Ok(String::from_utf16_lossy(&name[..end]))
    }
}

#[cfg(not(windows))]
mod native {
    use std::{io, path::Path, path::PathBuf};

    pub(super) const SUPPORTED: bool = false;
    pub(super) const UNSUPPORTED: &str = "Folder sharing is available in the Windows build.";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) struct FolderIdentity {
        pub(super) volume_id: u64,
        pub(super) file_id: [u8; 16],
    }

    pub(super) fn owner_window(_window: &tauri::WebviewWindow) -> Option<isize> {
        None
    }

    pub(super) fn pick_folder(_owner: Option<isize>) -> io::Result<Option<PathBuf>> {
        Err(io::Error::other(UNSUPPORTED))
    }

    pub(super) fn inspect(_path: &Path) -> Result<FolderIdentity, String> {
        Err(UNSUPPORTED.into())
    }
}
