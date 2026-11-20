//! Folder sharing on Linux and macOS (rdp-folder-sharing tasks 7.2 to 7.4).
//!
//! The native `NixRdpdrBackend` resolves every server path one name at a time
//! below a directory descriptor and refuses links and mount points, but it
//! reopens its root by path for every request. [`UnixBoundRoot`] therefore
//! opens the granted folder without following links, checks its device and
//! inode against the grant and keeps the descriptor for the channel's
//! lifetime. The backend's root path is that descriptor's own path
//! (`/proc/self/fd/<n>` or `/dev/fd/<n>`), so each reopen reaches the held
//! directory whatever later happens to its name. A local rename of the folder
//! cannot redirect requests; a deleted folder makes them fail.
//!
//! The native backend serves all drives from one root, so [`DriveRouter`]
//! keeps one backend per device.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::{Component, Path};
use std::sync::Arc;

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::Rdpdr;
use ironrdp_rdpdr::backend::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, DecodedDeviceControlRequest, DeviceControlRequest, DeviceIoRequest,
    PrinterIoRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::pdu::esp::PortIoRequest;
use ironrdp_svc::SvcMessage;
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, open, openat};
use nix::sys::stat::{FileStat, Mode, SFlag, fstat, fstatat};

use super::ShareRoot;

/// Flags for every directory descriptor taken while walking a path.
const DIRECTORY: OFlag = OFlag::O_RDONLY
    .union(OFlag::O_DIRECTORY)
    .union(OFlag::O_CLOEXEC);

/// Where the process's descriptors appear as paths.
#[cfg(target_os = "linux")]
const DESCRIPTOR_DIRECTORY: &str = "/proc/self/fd";
#[cfg(target_os = "macos")]
const DESCRIPTOR_DIRECTORY: &str = "/dev/fd";

/// The identity a grant records for a folder: the device, and the inode as
/// 16 little-endian bytes (the field holds a Windows file ID elsewhere).
#[allow(
    clippy::unnecessary_cast,
    reason = "st_ino and st_dev are narrower on macOS"
)]
pub(crate) fn identity(stat: &FileStat) -> (u64, [u8; 16]) {
    let mut file_id = [0u8; 16];
    file_id[..8].copy_from_slice(&(stat.st_ino as u64).to_le_bytes());
    (stat.st_dev as u64, file_id)
}

/// Why a granted folder cannot be bound. The text is shown to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindError {
    /// The path is relative or names `.` or `..`.
    NotPlain,
    /// A component of the path is a symbolic link.
    Link,
    /// The folder, or a folder above it, does not exist.
    Missing,
    /// The path names something other than a folder.
    NotFolder,
    /// The operating system refused access.
    Denied,
    /// A different folder now has the granted name.
    Replaced,
    /// The held folder cannot be reopened through its descriptor path.
    Unreachable(String),
    /// Any other failure.
    Other(Errno),
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPlain => f.write_str("The folder path must be absolute, without . or .. parts."),
            Self::Link => f.write_str(
                "Folders reached through a symbolic link cannot be shared. Choose the folder at its real location.",
            ),
            Self::Missing => f.write_str("The folder no longer exists. Choose it again."),
            Self::NotFolder => f.write_str("The shared path is no longer a folder. Choose the folder again."),
            #[cfg(target_os = "macos")]
            Self::Denied => f.write_str(
                "macOS does not let 1-1 open this folder. Allow it in System Settings, Privacy & Security, Files and Folders, then reconnect.",
            ),
            #[cfg(not(target_os = "macos"))]
            Self::Denied => f.write_str("1-1 may not open this folder (permission denied)."),
            Self::Replaced => f.write_str(
                "Another folder now has this folder's name. Choose the folder again.",
            ),
            Self::Unreachable(cause) => write!(f, "The folder could not be held open: {cause}"),
            Self::Other(errno) => write!(f, "The folder could not be opened: {errno}"),
        }
    }
}

impl std::error::Error for BindError {}

fn classify(errno: Errno) -> BindError {
    match errno {
        // O_NOFOLLOW on a symbolic link.
        Errno::ELOOP => BindError::Link,
        Errno::ENOENT => BindError::Missing,
        Errno::ENOTDIR => BindError::NotFolder,
        Errno::EACCES | Errno::EPERM => BindError::Denied,
        other => BindError::Other(other),
    }
}

/// Opens an absolute folder path from `/`, one name at a time, following no
/// symbolic link on the way.
pub(crate) fn open_folder(path: &Path) -> Result<OwnedFd, BindError> {
    if !path.is_absolute() {
        return Err(BindError::NotPlain);
    }
    let mut directory = open("/", DIRECTORY, Mode::empty()).map_err(classify)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                directory = match openat(
                    &directory,
                    name,
                    DIRECTORY | OFlag::O_NOFOLLOW,
                    Mode::empty(),
                ) {
                    Ok(next) => next,
                    // Linux reports a symbolic link opened with O_DIRECTORY as
                    // ENOTDIR, not ELOOP: look at the name itself to tell them apart.
                    Err(Errno::ENOTDIR) => {
                        let stat = fstatat(&directory, name, AtFlags::AT_SYMLINK_NOFOLLOW)
                            .map_err(classify)?;
                        return Err(
                            if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT
                                == SFlag::S_IFLNK
                            {
                                BindError::Link
                            } else {
                                BindError::NotFolder
                            },
                        );
                    }
                    Err(other) => return Err(classify(other)),
                };
            }
            _ => return Err(BindError::NotPlain),
        }
    }
    Ok(directory)
}

/// Whether the folder lives on a network file system, which the share does
/// not serve (the Windows build refuses network drives too).
pub(crate) fn on_network_file_system(folder: &OwnedFd) -> io::Result<bool> {
    let stat = nix::sys::statfs::fstatfs(folder)?;
    #[cfg(target_os = "linux")]
    {
        const NETWORK: [u64; 7] = [
            0x6969,      // NFS
            0x517B,      // SMB
            0xFF53_4D42, // CIFS
            0xFE53_4D42, // SMB2
            0x5346_414F, // AFS
            0x7375_7245, // Coda
            0x0102_1997, // 9P
        ];
        #[allow(
            clippy::unnecessary_cast,
            reason = "the type differs between C libraries"
        )]
        let kind = stat.filesystem_type().0 as u64;
        Ok(NETWORK.contains(&kind))
    }
    #[cfg(target_os = "macos")]
    {
        const NETWORK: [&str; 6] = ["nfs", "smbfs", "afpfs", "webdav", "ftp", "cifs"];
        Ok(NETWORK.contains(&stat.filesystem_type_name()))
    }
}

/// A granted folder, held open for one connection.
#[derive(Debug)]
pub(crate) struct UnixBoundRoot {
    folder: ShareRoot,
    file_base: String,
}

impl UnixBoundRoot {
    /// Opens `path` without following links and requires the device and inode
    /// the grant recorded.
    pub(crate) fn bind(path: &Path, volume_id: u64, file_id: [u8; 16]) -> Result<Self, BindError> {
        let folder = open_folder(path)?;
        let stat = fstat(&folder).map_err(classify)?;
        if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFDIR {
            return Err(BindError::NotFolder);
        }
        if identity(&stat) != (volume_id, file_id) {
            return Err(BindError::Replaced);
        }
        let file_base = format!("{DESCRIPTOR_DIRECTORY}/{}", folder.as_raw_fd());
        // The backend reopens this path for every request; it must reach the
        // held folder itself.
        let reopened = open(file_base.as_str(), DIRECTORY, Mode::empty())
            .map_err(|errno| BindError::Unreachable(format!("{file_base}: {errno}")))?;
        let reopened = fstat(&reopened).map_err(classify)?;
        if identity(&reopened) != (volume_id, file_id) {
            return Err(BindError::Unreachable(format!(
                "{file_base} leads to another folder"
            )));
        }
        Ok(Self {
            folder: Arc::new(folder),
            file_base,
        })
    }

    /// The root path to give the native backend.
    pub(crate) fn file_base(&self) -> &str {
        &self.file_base
    }

    /// The held descriptor, for the hard-link rule.
    pub(crate) fn root(&self) -> ShareRoot {
        Arc::clone(&self.folder)
    }
}

/// The link count of the entry a server path names below `root`: `Ok(None)`
/// when nothing exists there yet, a directory always one. Nothing on the way
/// is followed, so a link or an unreadable folder is an error and the caller
/// refuses the change.
pub(crate) fn link_count(root: &OwnedFd, remote_path: &str) -> io::Result<Option<u32>> {
    let names: Vec<&str> = remote_path
        .split(['\\', '/'])
        .filter(|name| !name.is_empty())
        .collect();
    let Some((last, parents)) = names.split_last() else {
        return Ok(None);
    };
    if names.iter().any(|name| matches!(*name, "." | "..")) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dot name in a share path",
        ));
    }
    let mut held: Option<OwnedFd> = None;
    for name in parents {
        let parent = held.as_ref().map_or(root.as_fd(), |fd| fd.as_fd());
        match openat(parent, *name, DIRECTORY | OFlag::O_NOFOLLOW, Mode::empty()) {
            Ok(fd) => held = Some(fd),
            Err(Errno::ENOENT) => return Ok(None),
            Err(errno) => return Err(errno.into()),
        }
    }
    let parent = held.as_ref().map_or(root.as_fd(), |fd| fd.as_fd());
    match fstatat(parent, *last, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) if SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFDIR => {
            Ok(Some(1))
        }
        #[allow(clippy::unnecessary_cast, reason = "nlink_t differs between platforms")]
        Ok(stat) => Ok(Some(
            u32::try_from(stat.st_nlink as u64).unwrap_or(u32::MAX),
        )),
        Err(Errno::ENOENT) => Ok(None),
        Err(errno) => Err(errno.into()),
    }
}

/// One native backend per drive device, each with its own folder.
#[derive(Debug)]
pub(crate) struct DriveRouter {
    drives: HashMap<u32, Box<dyn RdpdrBackend>>,
}

ironrdp_core::impl_as_any!(DriveRouter);

impl DriveRouter {
    pub(crate) fn new(drives: HashMap<u32, Box<dyn RdpdrBackend>>) -> Self {
        Self { drives }
    }

    fn drive(&mut self, device_id: u32) -> Option<&mut Box<dyn RdpdrBackend>> {
        let drive = self.drives.get_mut(&device_id);
        if drive.is_none() {
            log::warn!("folder share: request for unknown device {device_id}");
        }
        drive
    }

    /// Any drive, for the device classes none of them serve.
    fn any(&mut self) -> Option<&mut Box<dyn RdpdrBackend>> {
        self.drives.values_mut().next()
    }
}

fn io_request(req: &ServerDriveIoRequest) -> &DeviceIoRequest {
    match req {
        ServerDriveIoRequest::ServerCreateDriveRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceCloseRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceControlRequest(r) => &r.header,
        ServerDriveIoRequest::DeviceReadRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceWriteRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::DeviceFlushBuffersRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveSetInformationRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveLockControlRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveQuerySecurityRequest(r) => &r.device_io_request,
        ServerDriveIoRequest::ServerDriveSetSecurityRequest(r) => &r.device_io_request,
    }
}

impl RdpdrBackend for DriveRouter {
    fn supports_drive_security(&self) -> bool {
        !self.drives.is_empty() && self.drives.values().all(|d| d.supports_drive_security())
    }

    fn reset(&mut self) -> PduResult<()> {
        self.drives.values_mut().try_for_each(|drive| drive.reset())
    }

    fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.drive(device_id)
            .map_or(Ok(()), |d| d.restore_drive(device_id))
    }

    fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.drive(device_id)
            .map_or(Ok(()), |d| d.add_drive(device_id))
    }

    fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        self.drive(device_id)
            .map_or(Ok(Vec::new()), |d| d.remove_drive(device_id))
    }

    fn handle_server_device_announce_response(
        &mut self,
        pdu: ServerDeviceAnnounceResponse,
    ) -> PduResult<()> {
        let device_id = pdu.device_id;
        self.drive(device_id)
            .map_or(Ok(()), |d| d.handle_server_device_announce_response(pdu))
    }

    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        let device_id = io_request(&req).device_id;
        self.drive(device_id)
            .map_or(Ok(Vec::new()), |d| d.handle_drive_io_request(req))
    }

    fn handle_drive_device_control(
        &mut self,
        req: DecodedDeviceControlRequest<AnyIoCtlCode>,
    ) -> PduResult<Vec<SvcMessage>> {
        let device_id = req.request.header.device_id;
        self.drive(device_id)
            .map_or(Ok(Vec::new()), |d| d.handle_drive_device_control(req))
    }

    fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        let mut messages = Vec::new();
        for drive in self.drives.values_mut() {
            messages.extend(drive.poll_deferred_messages()?);
        }
        Ok(messages)
    }

    fn handle_user_logged_on(&mut self, rdpdr: &mut Rdpdr) -> PduResult<Vec<SvcMessage>> {
        let mut messages = Vec::new();
        for drive in self.drives.values_mut() {
            messages.extend(drive.handle_user_logged_on(rdpdr)?);
        }
        Ok(messages)
    }

    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        self.any()
            .map_or(Ok(Vec::new()), |d| d.handle_printer_io_request(req))
    }

    fn handle_port_io_request(&mut self, req: PortIoRequest) -> PduResult<Vec<SvcMessage>> {
        self.any()
            .map_or(Ok(Vec::new()), |d| d.handle_port_io_request(req))
    }

    fn reject_printer_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        self.any()
            .map_or(Ok(Vec::new()), |d| d.reject_printer_write(req))
    }
}

#[cfg(test)]
#[path = "unix_tests.rs"]
mod tests;
