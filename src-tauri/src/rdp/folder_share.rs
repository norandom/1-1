//! Binding a granted folder for redirection (rdp-folder-sharing task 4.1).
//!
//! The native IronRDP drive backend reopens its root by NT path on every
//! request. A path alone is therefore not a scope: another directory could
//! take its place, or a parent could be renamed or replaced by a junction.
//! [`BoundRoot`] holds a guard handle on the root and on every ancestor
//! directory for as long as the drive is attached. Each guard requests read
//! and traverse access and denies delete sharing, so no local process can
//! rename, move, delete or replace any component of the path meanwhile. The
//! root's volume and file identity is verified under those guards. Any
//! mismatch, link or unavailable folder fails closed and the drive is not
//! offered.

#[cfg(windows)]
pub(crate) mod attach;
pub(crate) mod status;
// Sessions start a worker only where folders can be shared (Windows); its
// tests run everywhere.
#[cfg(any(windows, test))]
pub(crate) mod worker;

#[cfg(windows)]
pub(crate) use windows_policy::ShareBackend;
#[cfg(all(windows, test))]
pub(crate) use windows_root::BindError;
#[cfg(windows)]
pub(crate) use windows_root::{BoundRoot, resolves_to};

/// Files with more than one hard link can have a name outside the granted
/// folder (pnpm and uv stores create them). The remote may read them, which
/// only shows the file's own content, but every request that would change
/// the file or its metadata is refused, so the share can never modify a copy
/// that lives outside it (user decision 2026-11-18). An already-open handle
/// that a local program moves out stays usable until closed; every new open
/// still resolves from the bound root.
///
/// A stream (`file:stream`) belongs to its file, so it counts that file's
/// links. When the count cannot be read, a change is refused. Renaming over a
/// multiply linked file is refused too. Remote hard-link creation
/// (FileLinkInformation) never reaches the backend: ironrdp-rdpdr 0.9 does not
/// decode that class, so such a request fails before it.
#[cfg(windows)]
mod windows_policy {
    use ironrdp_pdu::PduResult;
    use ironrdp_rdpdr::Rdpdr;
    use ironrdp_rdpdr::backend::RdpdrBackend;
    use ironrdp_rdpdr::pdu::RdpdrPdu;
    use ironrdp_rdpdr::pdu::efs::{
        AnyIoCtlCode, Boolean, ClientDriveQuerySecurityResponse, ClientDriveSetInformationResponse,
        ClientDriveSetSecurityResponse, CreateDisposition, CreateOptions,
        DecodedDeviceControlRequest, DesiredAccess, DeviceControlRequest, DeviceCreateRequest,
        DeviceCreateResponse, DeviceIoRequest, DeviceIoResponse, FileInformationClass, Information,
        NtStatus, PrinterIoRequest, SecurityInformation, ServerDeviceAnnounceResponse,
        ServerDriveIoRequest,
    };
    use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
    use ironrdp_rdpdr::pdu::esp::PortIoRequest;
    use ironrdp_svc::SvcMessage;
    use std::{
        io, mem,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::{Path, PathBuf},
    };
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
        FILE_STANDARD_INFO, FileStandardInfo, GetFileInformationByHandleEx,
    };

    /// The access the native backend's read-only mode also refuses, plus
    /// ACCESS_SYSTEM_SECURITY.
    const MUTATING_ACCESS: DesiredAccess = DesiredAccess::from_bits_retain(
        DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE.bits()
            | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY.bits()
            | DesiredAccess::FILE_WRITE_EA.bits()
            | DesiredAccess::FILE_WRITE_ATTRIBUTES.bits()
            | DesiredAccess::DELETE.bits()
            | DesiredAccess::WRITE_DAC.bits()
            | DesiredAccess::WRITE_OWNER.bits()
            | DesiredAccess::ACCESS_SYSTEM_SECURITY.bits()
            | DesiredAccess::MAXIMUM_ALLOWED.bits()
            | DesiredAccess::GENERIC_ALL.bits()
            | DesiredAccess::GENERIC_WRITE.bits(),
    );

    /// Whether the create request could change an existing file.
    pub(super) fn mutates(request: &DeviceCreateRequest) -> bool {
        request.desired_access.intersects(MUTATING_ACCESS)
            || request
                .create_options
                .contains(CreateOptions::FILE_DELETE_ON_CLOSE)
            || [
                CreateDisposition::FILE_SUPERSEDE,
                CreateDisposition::FILE_OVERWRITE,
                CreateDisposition::FILE_OVERWRITE_IF,
            ]
            .contains(&request.create_disposition)
    }

    /// The link count of the entry a remote path names below `root`. A stream
    /// name counts the file it belongs to. `Ok(None)` means nothing exists
    /// there yet; any other failure is an error, so callers fail closed.
    pub(super) fn link_count(root: &Path, remote_path: &str) -> io::Result<Option<u32>> {
        let relative = remote_path.trim_start_matches(['\\', '/']);
        // NTFS names cannot contain ':', so the first one starts a stream name.
        let relative = relative.split(':').next().unwrap_or_default();
        if relative.is_empty() {
            return Ok(None);
        }
        let opened = std::fs::File::options()
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode(0x7)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(root.join(relative));
        let file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut info = FILE_STANDARD_INFO::default();
        // SAFETY: The buffer is an owned struct of the size passed; the
        // handle stays open for the call.
        unsafe {
            GetFileInformationByHandleEx(
                HANDLE(file.as_raw_handle()),
                FileStandardInfo,
                (&raw mut info).cast(),
                mem::size_of::<FILE_STANDARD_INFO>() as u32,
            )
        }
        .map_err(io::Error::other)?;
        Ok(Some(info.NumberOfLinks))
    }

    /// The shared folder itself is never renamed or deleted from the remote.
    /// Both need an open with DELETE access; asked for explicitly, it is
    /// refused here. Broader requests (GENERIC_ALL, MAXIMUM_ALLOWED) that would
    /// grant DELETE fail on the root guard, which denies delete sharing.
    pub(super) fn removes_root(request: &DeviceCreateRequest) -> bool {
        request.path.trim_matches(['\\', '/']).is_empty()
            && (request.desired_access.contains(DesiredAccess::DELETE)
                || request
                    .create_options
                    .contains(CreateOptions::FILE_DELETE_ON_CLOSE))
    }

    /// A change is safe only for a new entry or one with a single name.
    fn may_change(root: &Path, remote_path: &str) -> bool {
        matches!(link_count(root, remote_path), Ok(None | Some(0 | 1)))
    }

    /// Security information the native backend can only read or write after
    /// enabling SeSecurityPrivilege (SACL, label and related parts) or the backup
    /// and restore privileges (`BACKUP`).
    const PRIVILEGED_SECURITY_INFORMATION: SecurityInformation = SecurityInformation::SACL
        .union(SecurityInformation::LABEL)
        .union(SecurityInformation::ATTRIBUTE)
        .union(SecurityInformation::SCOPE)
        .union(SecurityInformation::PROCESS_TRUST_LABEL)
        .union(SecurityInformation::BACKUP)
        .union(SecurityInformation::PROTECTED_SACL)
        .union(SecurityInformation::UNPROTECTED_SACL);

    /// The share acts with the workstation user's ordinary rights in either
    /// mode. The native backend would enable a privilege for these requests, so
    /// they are answered with STATUS_PRIVILEGE_NOT_HELD before it sees them.
    fn privilege_refusal(req: &ServerDriveIoRequest) -> PduResult<Option<SvcMessage>> {
        let denied = NtStatus::PRIVILEGE_NOT_HELD;
        let reply = match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(create)
                if create
                    .desired_access
                    .contains(DesiredAccess::ACCESS_SYSTEM_SECURITY) =>
            {
                RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                    device_io_reply: DeviceIoResponse::new(
                        create.device_io_request.clone(),
                        denied,
                    ),
                    file_id: 0,
                    information: Information::empty(),
                })
            }
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(query)
                if query
                    .security_information
                    .intersects(PRIVILEGED_SECURITY_INFORMATION) =>
            {
                RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                    device_io_response: DeviceIoResponse::new(
                        query.device_io_request.clone(),
                        denied,
                    ),
                    security_descriptor: None,
                })
            }
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(set)
                if set
                    .security_information
                    .intersects(PRIVILEGED_SECURITY_INFORMATION) =>
            {
                RdpdrPdu::ClientDriveSetSecurityResponse(
                    ClientDriveSetSecurityResponse::new(set, denied)
                        .map_err(|e| ironrdp_pdu::encode_err!(e))?,
                )
            }
            _ => return Ok(None),
        };
        Ok(Some(SvcMessage::from(reply)))
    }

    /// The share's policy in front of the native backend: no privileges, and no
    /// change to a multiply linked file. Read-only mode is the native backend's
    /// own (`RedirectedDrive::new(.., read_only)`); everything else is forwarded.
    #[derive(Debug)]
    pub(crate) struct ShareBackend {
        inner: Box<dyn RdpdrBackend>,
        root: PathBuf,
    }

    ironrdp_core::impl_as_any!(ShareBackend);

    impl ShareBackend {
        /// `root` must be the path of a [`super::BoundRoot`] held for the
        /// backend's lifetime.
        pub(crate) fn new(inner: Box<dyn RdpdrBackend>, root: PathBuf) -> Self {
            Self { inner, root }
        }

        fn refuses(&self, request: &DeviceCreateRequest) -> bool {
            removes_root(request) || (mutates(request) && !may_change(&self.root, &request.path))
        }
    }

    impl RdpdrBackend for ShareBackend {
        fn supports_drive_security(&self) -> bool {
            self.inner.supports_drive_security()
        }

        fn reset(&mut self) -> PduResult<()> {
            self.inner.reset()
        }

        fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
            self.inner.restore_drive(device_id)
        }

        fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
            self.inner.add_drive(device_id)
        }

        fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
            self.inner.remove_drive(device_id)
        }

        fn handle_server_device_announce_response(
            &mut self,
            pdu: ServerDeviceAnnounceResponse,
        ) -> PduResult<()> {
            self.inner.handle_server_device_announce_response(pdu)
        }

        fn handle_scard_call(
            &mut self,
            req: DeviceControlRequest<ScardIoCtlCode>,
            call: ScardCall,
        ) -> PduResult<Vec<SvcMessage>> {
            self.inner.handle_scard_call(req, call)
        }

        fn handle_drive_io_request(
            &mut self,
            req: ServerDriveIoRequest,
        ) -> PduResult<Vec<SvcMessage>> {
            if let Some(refusal) = privilege_refusal(&req)? {
                return Ok(vec![refusal]);
            }
            if let ServerDriveIoRequest::ServerCreateDriveRequest(create) = &req
                && self.refuses(create)
            {
                return Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
                    DeviceCreateResponse {
                        device_io_reply: DeviceIoResponse::new(
                            create.device_io_request.clone(),
                            NtStatus::ACCESS_DENIED,
                        ),
                        file_id: 0,
                        information: Information::empty(),
                    },
                ))]);
            }
            // Replacing a multiply linked file would unlink one of its names.
            if let ServerDriveIoRequest::ServerDriveSetInformationRequest(set) = &req
                && let FileInformationClass::Rename(rename) = &set.set_buffer
                && rename.replace_if_exists == Boolean::True
                && !may_change(&self.root, &rename.file_name)
            {
                let refusal = ClientDriveSetInformationResponse::new(set, NtStatus::ACCESS_DENIED)
                    .map_err(|e| ironrdp_pdu::encode_err!(e))?;
                return Ok(vec![SvcMessage::from(
                    RdpdrPdu::ClientDriveSetInformationResponse(refusal),
                )]);
            }
            self.inner.handle_drive_io_request(req)
        }

        fn handle_drive_device_control(
            &mut self,
            req: DecodedDeviceControlRequest<AnyIoCtlCode>,
        ) -> PduResult<Vec<SvcMessage>> {
            self.inner.handle_drive_device_control(req)
        }

        fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
            self.inner.poll_deferred_messages()
        }

        fn handle_user_logged_on(&mut self, rdpdr: &mut Rdpdr) -> PduResult<Vec<SvcMessage>> {
            self.inner.handle_user_logged_on(rdpdr)
        }

        fn handle_printer_io_request(
            &mut self,
            req: PrinterIoRequest,
        ) -> PduResult<Vec<SvcMessage>> {
            self.inner.handle_printer_io_request(req)
        }

        fn handle_port_io_request(&mut self, req: PortIoRequest) -> PduResult<Vec<SvcMessage>> {
            self.inner.handle_port_io_request(req)
        }

        fn reject_printer_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
            self.inner.reject_printer_write(req)
        }
    }
}

#[cfg(all(test, windows))]
mod tests;

#[cfg(windows)]
mod windows_root {
    use std::{
        fs::File,
        io, mem,
        os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
        path::{Component, Path, PathBuf, Prefix},
    };
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_LIST_DIRECTORY, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES,
        FILE_TRAVERSE, FileAttributeTagInfo, FileIdInfo, GETFINALPATHNAMEBYHANDLE_FLAGS,
        GetFileInformationByHandleEx, GetFinalPathNameByHandleW, VOLUME_NAME_DOS,
    };

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    /// FILE_SHARE_READ | FILE_SHARE_WRITE: other opens may read and write,
    /// but none may request DELETE, which rename, move and delete require.
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const ERROR_SHARING_VIOLATION: i32 = 32;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(crate) enum BindError {
        /// The path is not a subfolder of a local drive letter.
        NotADriveFolder,
        /// A component is missing or cannot be opened.
        Unavailable(String),
        /// A component is a junction, symbolic link or other reparse point.
        Link(PathBuf),
        /// A component is not a directory.
        NotADirectory(PathBuf),
        /// Another program holds a component open for rename or delete.
        Busy(PathBuf),
        /// The folder at this path is not the granted folder.
        Replaced,
    }

    impl std::fmt::Display for BindError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::NotADriveFolder => {
                    f.write_str("The shared folder is not a folder on a local drive letter.")
                }
                Self::Unavailable(cause) => {
                    write!(f, "The shared folder is not available: {cause}")
                }
                Self::Link(path) => write!(
                    f,
                    "{} is a link or junction, so the folder is not shared.",
                    path.display()
                ),
                Self::NotADirectory(path) => write!(f, "{} is not a folder.", path.display()),
                Self::Busy(path) => write!(
                    f,
                    "{} is being renamed or deleted by another program.",
                    path.display()
                ),
                Self::Replaced => {
                    f.write_str("The shared folder was replaced. Choose it again to share it.")
                }
            }
        }
    }

    /// The granted folder, held in place for the drive's lifetime.
    pub(crate) struct BoundRoot {
        path: PathBuf,
        /// Volume root first, the granted folder last. Dropping them ends the guard.
        _guards: Vec<File>,
    }

    impl std::fmt::Debug for BoundRoot {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("BoundRoot")
                .field("path", &self.path)
                .finish_non_exhaustive()
        }
    }

    impl BoundRoot {
        /// Guard every component of `path` and verify that the folder there
        /// is the granted one.
        pub(crate) fn bind(
            path: &Path,
            volume_id: u64,
            file_id: [u8; 16],
        ) -> Result<Self, BindError> {
            let mut components = path.components();
            let (Some(Component::Prefix(prefix)), Some(Component::RootDir)) =
                (components.next(), components.next())
            else {
                return Err(BindError::NotADriveFolder);
            };
            if !matches!(prefix.kind(), Prefix::Disk(_)) {
                return Err(BindError::NotADriveFolder);
            }
            let names: Vec<_> = components.collect();
            if names.is_empty() || names.iter().any(|c| !matches!(c, Component::Normal(_))) {
                return Err(BindError::NotADriveFolder);
            }
            // The volume root cannot be renamed; guard each folder below it in
            // order. A guarded parent cannot be renamed, so a child found under
            // it is the child at that path from then on.
            let mut current = PathBuf::from(prefix.as_os_str());
            current.push(Component::RootDir.as_os_str());
            let mut guards = Vec::with_capacity(names.len());
            let mut paths = Vec::with_capacity(names.len());
            for name in names {
                current.push(name.as_os_str());
                guards.push(guard(&current)?);
                paths.push(current.clone());
            }
            // With every guard held, check again that no component became a
            // link and that the root resolves to the granted path; a
            // substituted drive letter or a short name resolves elsewhere.
            for (file, path) in guards.iter().zip(&paths) {
                check_directory(file, path)?;
            }
            let root = guards.last().expect("at least one component");
            match resolves_to(handle(root), &current) {
                Ok(true) => {}
                Ok(false) => return Err(BindError::Replaced),
                Err(cause) => return Err(BindError::Unavailable(cause)),
            }
            let (volume, file) =
                identity(root).map_err(|e| BindError::Unavailable(e.to_string()))?;
            if (volume, file) != (volume_id, file_id) {
                return Err(BindError::Replaced);
            }
            Ok(Self {
                path: current,
                _guards: guards,
            })
        }

        /// The path the native backend reopens; it cannot change while bound.
        pub(crate) fn path(&self) -> &Path {
            &self.path
        }
    }

    fn guard(path: &Path) -> Result<File, BindError> {
        let file = File::options()
            // Read and traverse access make the share mode binding on later opens.
            .access_mode(
                FILE_LIST_DIRECTORY.0 | FILE_TRAVERSE.0 | FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE,
            )
            .share_mode(SHARE_READ_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .open(path)
            .map_err(|error| match error.raw_os_error() {
                Some(ERROR_SHARING_VIOLATION) => BindError::Busy(path.to_path_buf()),
                _ => BindError::Unavailable(format!("{}: {error}", path.display())),
            })?;
        check_directory(&file, path)?;
        Ok(file)
    }

    /// The open component is a plain directory, not a link.
    fn check_directory(file: &File, path: &Path) -> Result<(), BindError> {
        let attributes = attributes(file).map_err(|e| BindError::Unavailable(e.to_string()))?;
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(BindError::Link(path.to_path_buf()));
        }
        if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(BindError::NotADirectory(path.to_path_buf()));
        }
        Ok(())
    }

    /// Whether the open handle's final DOS path is `path` itself.
    pub(crate) fn resolves_to(handle: HANDLE, path: &Path) -> Result<bool, String> {
        let mut buffer = vec![0u16; 512];
        loop {
            // SAFETY: The buffer is owned and its length is passed; the handle
            // belongs to an open file for the duration of the call.
            let length = unsafe {
                GetFinalPathNameByHandleW(
                    handle,
                    &mut buffer,
                    GETFINALPATHNAMEBYHANDLE_FLAGS(FILE_NAME_NORMALIZED.0 | VOLUME_NAME_DOS.0),
                )
            } as usize;
            if length == 0 {
                return Err("Could not resolve the folder's location.".into());
            }
            if length < buffer.len() {
                let resolved = String::from_utf16_lossy(&buffer[..length]);
                let resolved = resolved.strip_prefix(r"\\?\").unwrap_or(&resolved);
                let chosen = path.to_string_lossy();
                return Ok(resolved.to_lowercase() == chosen.trim_end_matches('\\').to_lowercase());
            }
            buffer.resize(length + 1, 0);
        }
    }

    fn handle(file: &File) -> HANDLE {
        HANDLE(file.as_raw_handle())
    }

    fn attributes(file: &File) -> io::Result<u32> {
        let mut tag = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: The buffer is an owned struct of the size passed; the handle
        // stays open for the call.
        unsafe {
            GetFileInformationByHandleEx(
                handle(file),
                FileAttributeTagInfo,
                (&raw mut tag).cast(),
                mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        }
        .map_err(io::Error::other)?;
        Ok(tag.FileAttributes)
    }

    fn identity(file: &File) -> io::Result<(u64, [u8; 16])> {
        let mut id = FILE_ID_INFO::default();
        // SAFETY: As in `attributes`.
        unsafe {
            GetFileInformationByHandleEx(
                handle(file),
                FileIdInfo,
                (&raw mut id).cast(),
                mem::size_of::<FILE_ID_INFO>() as u32,
            )
        }
        .map_err(io::Error::other)?;
        Ok((id.VolumeSerialNumber, id.FileId.Identifier))
    }

    /// For tests: the identity a grant would record for `path`.
    #[cfg(test)]
    pub(crate) fn identity_of(path: &Path) -> (u64, [u8; 16]) {
        identity(&guard(path).unwrap()).unwrap()
    }
}

#[cfg(all(test, windows))]
pub(crate) use windows_root::identity_of;
