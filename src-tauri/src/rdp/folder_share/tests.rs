//! Task 4.1: a bound root and its ancestors stay in place; replaced roots,
//! links and missing folders fail closed; ordinary in-folder work continues.
use super::identity_of;
use super::{BindError, BoundRoot};
use std::{fs, path::PathBuf};

const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_ACCESS_DENIED: i32 = 5;

/// The long, link-free form of the temp directory (CI uses 8.3 names).
fn real_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let resolved = fs::canonicalize(dir.path()).unwrap();
    let text = resolved.to_string_lossy().into_owned();
    let path = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
    (dir, path)
}

fn blocked(result: std::io::Result<()>) -> bool {
    matches!(
        result.map_err(|e| e.raw_os_error()),
        Err(Some(ERROR_SHARING_VIOLATION | ERROR_ACCESS_DENIED))
    )
}

#[test]
fn binding_holds_the_root_and_every_ancestor_in_place_until_released() {
    let (_dir, base) = real_dir();
    let parent = base.join("parent");
    let root = parent.join("shared");
    fs::create_dir_all(&root).unwrap();
    let (volume, file) = identity_of(&root);
    let bound = BoundRoot::bind(&root, volume, file).unwrap();
    assert_eq!(bound.path(), root);

    assert!(
        blocked(fs::rename(&root, base.join("moved-root"))),
        "root rename"
    );
    assert!(
        blocked(fs::rename(&parent, base.join("moved-parent"))),
        "ancestor move"
    );
    assert!(blocked(fs::remove_dir(&root).map(|_| ())), "root delete");
    assert!(root.is_dir() && parent.is_dir());

    drop(bound);
    fs::rename(&parent, base.join("moved-parent")).unwrap();
}

#[test]
fn ordinary_file_work_inside_a_bound_root_continues() {
    let (_dir, base) = real_dir();
    let root = base.join("shared");
    fs::create_dir(&root).unwrap();
    let (volume, file) = identity_of(&root);
    let _bound = BoundRoot::bind(&root, volume, file).unwrap();
    fs::write(root.join("a.txt"), b"first").unwrap();
    fs::rename(root.join("a.txt"), root.join("b.txt")).unwrap();
    assert_eq!(fs::read(root.join("b.txt")).unwrap(), b"first");
    fs::create_dir(root.join("sub")).unwrap();
    fs::rename(root.join("sub"), root.join("renamed")).unwrap();
    fs::remove_dir(root.join("renamed")).unwrap();
    fs::remove_file(root.join("b.txt")).unwrap();
}

#[test]
fn a_folder_replaced_at_the_granted_path_is_refused() {
    let (_dir, base) = real_dir();
    let root = base.join("shared");
    fs::create_dir(&root).unwrap();
    let (volume, file) = identity_of(&root);
    fs::rename(&root, base.join("original")).unwrap();
    fs::create_dir(&root).unwrap();
    assert_eq!(
        BoundRoot::bind(&root, volume, file).unwrap_err(),
        BindError::Replaced
    );
    // The original folder under its new name is not the path that was granted either.
    assert_eq!(
        BoundRoot::bind(&root, volume, [0; 16]).unwrap_err(),
        BindError::Replaced
    );
}

#[test]
fn links_missing_folders_and_non_folders_fail_closed() {
    let (_dir, base) = real_dir();
    let real = base.join("real");
    fs::create_dir_all(real.join("shared")).unwrap();
    let via = base.join("via");
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&via)
        .arg(&real)
        .output()
        .unwrap();
    assert!(output.status.success(), "mklink /J failed: {output:?}");
    let (volume, file) = identity_of(&real.join("shared"));
    assert_eq!(
        BoundRoot::bind(&via.join("shared"), volume, file).unwrap_err(),
        BindError::Link(via.clone())
    );
    assert!(matches!(
        BoundRoot::bind(&base.join("missing"), volume, file).unwrap_err(),
        BindError::Unavailable(_)
    ));
    fs::write(base.join("file.txt"), b"x").unwrap();
    assert_eq!(
        BoundRoot::bind(&base.join("file.txt"), volume, file).unwrap_err(),
        BindError::NotADirectory(base.join("file.txt"))
    );
    for path in [r"C:\", r"\\server\share\folder", r"relative\folder"] {
        assert_eq!(
            BoundRoot::bind(std::path::Path::new(path), volume, file).unwrap_err(),
            BindError::NotADriveFolder,
            "{path}"
        );
    }
}

#[test]
fn a_path_that_resolves_elsewhere_is_refused() {
    use windows::Win32::Storage::FileSystem::GetShortPathNameW;
    use windows::core::HSTRING;
    let (_dir, base) = real_dir();
    let root = base.join("a folder with a long name");
    fs::create_dir(&root).unwrap();
    let (volume, file) = identity_of(&root);
    let mut buffer = [0u16; 1024];
    // SAFETY: The buffer is owned and its length is passed.
    let length = unsafe { GetShortPathNameW(&HSTRING::from(root.as_os_str()), Some(&mut buffer)) };
    let short = PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize]));
    if length == 0 || short == root {
        return; // This volume does not create short names.
    }
    // The same folder, named differently: only the granted spelling binds.
    assert_eq!(
        BoundRoot::bind(&short, volume, file).unwrap_err(),
        BindError::Replaced
    );
    BoundRoot::bind(&root, volume, file).unwrap();
}

#[test]
fn a_component_held_open_for_rename_is_reported_busy() {
    use std::os::windows::fs::OpenOptionsExt;
    let (_dir, base) = real_dir();
    let root = base.join("shared");
    fs::create_dir(&root).unwrap();
    let (volume, file) = identity_of(&root);
    // An open with DELETE access, as a pending rename holds.
    let _renaming = fs::File::options()
        .access_mode(0x0001_0000) // DELETE
        .share_mode(0x7)
        .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS
        .open(&root)
        .unwrap();
    assert_eq!(
        BoundRoot::bind(&root, volume, file).unwrap_err(),
        BindError::Busy(root.clone())
    );
}

mod links {
    use super::super::ShareBackend;
    use super::super::windows_policy::{link_count, mutates};
    use super::real_dir;
    use ironrdp_pdu::PduResult;
    use ironrdp_rdpdr::backend::RdpdrBackend;
    use ironrdp_rdpdr::pdu::efs::{
        Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceControlRequest,
        DeviceCreateRequest, DeviceIoRequest, FileAttributes, FileInformationClass,
        FileRenameInformation, MajorFunction, MinorFunction, ServerDeviceAnnounceResponse,
        ServerDriveIoRequest, ServerDriveSetInformationRequest, SharedAccess,
    };
    use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
    use ironrdp_svc::SvcMessage;
    use std::{
        fs,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    fn create(
        path: &str,
        access: DesiredAccess,
        disposition: CreateDisposition,
        options: CreateOptions,
    ) -> DeviceCreateRequest {
        DeviceCreateRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id: 0,
                completion_id: 7,
                major_function: MajorFunction::Create,
                minor_function: MinorFunction::from(0),
            },
            desired_access: access,
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: disposition,
            create_options: options,
            path: path.into(),
        }
    }

    fn read(path: &str) -> DeviceCreateRequest {
        create(
            path,
            DesiredAccess::GENERIC_READ | DesiredAccess::FILE_READ_ATTRIBUTES,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        )
    }

    fn write(path: &str) -> DeviceCreateRequest {
        create(
            path,
            DesiredAccess::GENERIC_WRITE,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        )
    }

    #[test]
    fn requests_that_could_change_a_file_are_recognized() {
        assert!(!mutates(&read(r"\a.txt")));
        for access in [
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE,
            DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY,
            DesiredAccess::FILE_WRITE_ATTRIBUTES,
            DesiredAccess::DELETE,
            DesiredAccess::WRITE_DAC,
            DesiredAccess::MAXIMUM_ALLOWED,
            DesiredAccess::GENERIC_ALL,
            DesiredAccess::GENERIC_WRITE,
        ] {
            let bits = access.bits();
            assert!(
                mutates(&create(
                    r"\a.txt",
                    access,
                    CreateDisposition::FILE_OPEN,
                    CreateOptions::empty()
                )),
                "{bits:#x}"
            );
        }
        for disposition in [
            CreateDisposition::FILE_SUPERSEDE,
            CreateDisposition::FILE_OVERWRITE,
            CreateDisposition::FILE_OVERWRITE_IF,
        ] {
            assert!(mutates(&create(
                r"\a.txt",
                DesiredAccess::GENERIC_READ,
                disposition,
                CreateOptions::empty()
            )));
        }
        assert!(mutates(&create(
            r"\a.txt",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_OPEN,
            CreateOptions::FILE_DELETE_ON_CLOSE
        )));
    }

    #[test]
    fn link_counts_come_from_the_file_below_the_root() {
        let (_dir, root) = real_dir();
        fs::write(root.join("single.txt"), b"x").unwrap();
        fs::write(root.join("linked.txt"), b"x").unwrap();
        fs::hard_link(root.join("linked.txt"), root.join("alias.txt")).unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        let count = |path| link_count(&root, path).unwrap();
        assert_eq!(count(r"\single.txt"), Some(1));
        assert_eq!(count(r"\linked.txt"), Some(2));
        assert_eq!(count(r"\folder"), Some(1));
        assert_eq!(count(r"\missing.txt"), None);
        assert_eq!(count(r"\missing\deeper.txt"), None);
        assert_eq!(count(r"\"), None);
        // A new stream belongs to its file, so it counts that file's links.
        assert_eq!(count(r"\linked.txt:notes"), Some(2));
        assert_eq!(count(r"\linked.txt::`$DATA"), Some(2));
        // Anything but "not found" is an error, never "no links".
        assert!(link_count(&root, r"\bad<name>.txt").is_err());
    }

    #[derive(Debug, Default)]
    struct Recording(Arc<AtomicUsize>);
    ironrdp_core::impl_as_any!(Recording);

    impl RdpdrBackend for Recording {
        fn handle_server_device_announce_response(
            &mut self,
            _pdu: ServerDeviceAnnounceResponse,
        ) -> PduResult<()> {
            Ok(())
        }
        fn handle_scard_call(
            &mut self,
            _req: DeviceControlRequest<ScardIoCtlCode>,
            _call: ScardCall,
        ) -> PduResult<Vec<SvcMessage>> {
            unreachable!("no smart cards on a drive")
        }
        fn handle_drive_io_request(
            &mut self,
            _req: ServerDriveIoRequest,
        ) -> PduResult<Vec<SvcMessage>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    const ACCESS_DENIED: u32 = 0xC000_0022;

    /// The common part of an encoded device I/O reply.
    struct Reply {
        completion_id: u32,
        status: u32,
        rest: Vec<u8>,
    }

    impl Reply {
        fn of(message: &SvcMessage) -> Self {
            let bytes = message.encode_unframed_pdu().unwrap();
            let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            // RDPDR header (4 bytes), then DeviceId, CompletionId and IoStatus.
            Self {
                completion_id: word(8),
                status: word(12),
                rest: bytes[16..].to_vec(),
            }
        }
    }

    fn rename(to: &str, replace: bool) -> ServerDriveIoRequest {
        ServerDriveIoRequest::ServerDriveSetInformationRequest(ServerDriveSetInformationRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id: 3,
                completion_id: 9,
                major_function: MajorFunction::SetInformation,
                minor_function: MinorFunction::from(0),
            },
            set_buffer: FileInformationClass::Rename(FileRenameInformation {
                replace_if_exists: if replace {
                    Boolean::True
                } else {
                    Boolean::False
                },
                file_name: to.into(),
            }),
        })
    }

    #[test]
    fn renaming_over_a_multiply_linked_file_is_refused() {
        let (_dir, root) = real_dir();
        fs::write(root.join("linked.txt"), b"x").unwrap();
        fs::hard_link(root.join("linked.txt"), root.join("alias.txt")).unwrap();
        fs::write(root.join("single.txt"), b"x").unwrap();
        let forwarded = Arc::new(AtomicUsize::new(0));
        let mut backend =
            ShareBackend::new(Box::new(Recording(Arc::clone(&forwarded))), root.clone());

        let refused = backend
            .handle_drive_io_request(rename(r"\linked.txt", true))
            .unwrap();
        let reply = Reply::of(&refused[0]);
        assert_eq!((reply.completion_id, reply.status), (9, ACCESS_DENIED));
        assert_eq!(forwarded.load(Ordering::SeqCst), 0);

        // Without replacing, the native backend reports the collision itself.
        assert!(
            backend
                .handle_drive_io_request(rename(r"\linked.txt", false))
                .unwrap()
                .is_empty()
        );
        assert!(
            backend
                .handle_drive_io_request(rename(r"\single.txt", true))
                .unwrap()
                .is_empty()
        );
        assert!(
            backend
                .handle_drive_io_request(rename(r"\new.txt", true))
                .unwrap()
                .is_empty()
        );
        assert_eq!(forwarded.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn writes_to_multiply_linked_files_are_refused_and_everything_else_passes_through() {
        let (_dir, root) = real_dir();
        fs::write(root.join("linked.txt"), b"x").unwrap();
        fs::hard_link(root.join("linked.txt"), root.join("alias.txt")).unwrap();
        fs::write(root.join("single.txt"), b"x").unwrap();
        let forwarded = Arc::new(AtomicUsize::new(0));
        let mut backend =
            ShareBackend::new(Box::new(Recording(Arc::clone(&forwarded))), root.clone());
        let mut send = |request| {
            backend
                .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
                .unwrap()
        };

        let refused = send(write(r"\linked.txt"));
        assert_eq!(refused.len(), 1, "one create response");
        let reply = Reply::of(&refused[0]);
        assert_eq!((reply.completion_id, reply.status), (7, ACCESS_DENIED));
        assert_eq!(reply.rest, [0, 0, 0, 0, 0], "file id 0 and no information");
        // Adding a stream changes the linked file too.
        assert_eq!(
            Reply::of(&send(write(r"\linked.txt:notes"))[0]).status,
            ACCESS_DENIED
        );
        // A count that cannot be read refuses the change.
        assert_eq!(
            Reply::of(&send(write(r"\bad<name>.txt"))[0]).status,
            ACCESS_DENIED
        );
        assert_eq!(
            forwarded.load(Ordering::SeqCst),
            0,
            "the native backend never saw it"
        );

        assert!(send(read(r"\linked.txt")).is_empty());
        assert!(send(write(r"\single.txt")).is_empty());
        assert!(send(write(r"\new.txt")).is_empty());
        assert_eq!(forwarded.load(Ordering::SeqCst), 3);
        assert_eq!(
            fs::read(root.join("alias.txt")).unwrap(),
            b"x",
            "the outside alias is unchanged"
        );
    }
}

/// Task 4.2: the share's policy in front of the real native backend.
mod policy {
    use super::super::ShareBackend;
    use super::real_dir;
    use ironrdp_rdpdr::backend::RdpdrBackend;
    use ironrdp_rdpdr::pdu::efs::{
        AnyIoCtlCode, Boolean, CreateDisposition, CreateOptions, DesiredAccess,
        DeviceControlRequest, DeviceCreateRequest, DeviceIoRequest, DeviceReadRequest,
        DeviceWriteRequest, FileAttributes, FileEndOfFileInformation, FileInformationClass,
        FileInformationClassLevel, FileRenameInformation, MajorFunction, MinorFunction,
        SecurityInformation, ServerDriveIoRequest, ServerDriveQueryDirectoryRequest,
        ServerDriveQuerySecurityRequest, ServerDriveSetInformationRequest,
        ServerDriveSetSecurityRequest, SharedAccess,
    };
    use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackendFactory};
    use std::{fs, path::PathBuf, time::Duration};

    const SUCCESS: u32 = 0;
    const END_OF_FILE: u32 = 0xC000_0011;
    const ACCESS_DENIED: u32 = 0xC000_0022;
    const PRIVILEGE_NOT_HELD: u32 = 0xC000_0061;
    const MEDIA_WRITE_PROTECTED: u32 = 0xC000_00A2;
    const NOT_SUPPORTED: u32 = 0xC000_00BB;
    const CONTENT: &[u8] = b"original bytes";

    struct Drive {
        _dir: tempfile::TempDir,
        root: PathBuf,
        backend: ShareBackend,
        completion: u32,
    }

    /// The status and the body after it of one reply.
    struct Reply {
        status: u32,
        body: Vec<u8>,
    }

    impl Reply {
        fn word(&self, at: usize) -> u32 {
            u32::from_le_bytes(self.body[at..at + 4].try_into().unwrap())
        }
        /// DR_READ_RSP: Length, then the data.
        fn data(&self) -> &[u8] {
            &self.body[4..4 + self.word(0) as usize]
        }
    }

    fn drive(read_only: bool) -> Drive {
        let (dir, root) = real_dir();
        fs::write(root.join("data.bin"), CONTENT).unwrap();
        let share = RedirectedDrive::new(1, "Share", &root, read_only).unwrap();
        let native = WindowsRdpdrBackendFactory::new(share).build();
        let mut backend = ShareBackend::new(Box::new(native), root.clone());
        backend.add_drive(1).unwrap();
        Drive {
            _dir: dir,
            root,
            backend,
            completion: 0,
        }
    }

    impl Drive {
        fn io(&mut self, major_function: MajorFunction, file_id: u32) -> DeviceIoRequest {
            self.completion += 1;
            DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: self.completion,
                major_function,
                minor_function: MinorFunction::from(0),
            }
        }

        /// Exactly one reply per request, synchronous or deferred.
        fn send(&mut self, request: ServerDriveIoRequest) -> Reply {
            let mut replies = self.backend.handle_drive_io_request(request).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while replies.is_empty() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
                replies = self.backend.poll_deferred_messages().unwrap();
            }
            assert_eq!(replies.len(), 1, "one reply per request");
            let bytes = replies[0].encode_unframed_pdu().unwrap();
            let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            assert_eq!(word(8), self.completion, "the reply completes this request");
            Reply {
                status: word(12),
                body: bytes[16..].to_vec(),
            }
        }

        fn create(
            &mut self,
            path: &str,
            access: DesiredAccess,
            disposition: CreateDisposition,
            options: CreateOptions,
        ) -> Reply {
            let device_io_request = self.io(MajorFunction::Create, 0);
            self.send(ServerDriveIoRequest::ServerCreateDriveRequest(
                DeviceCreateRequest {
                    device_io_request,
                    desired_access: access,
                    allocation_size: 0,
                    file_attributes: FileAttributes::empty(),
                    shared_access: SharedAccess::FILE_SHARE_READ | SharedAccess::FILE_SHARE_WRITE,
                    create_disposition: disposition,
                    create_options: options,
                    path: path.into(),
                },
            ))
        }

        /// A file ID for a successful open.
        fn open(&mut self, path: &str, access: DesiredAccess) -> u32 {
            let reply = self.create(
                path,
                access,
                CreateDisposition::FILE_OPEN,
                CreateOptions::empty(),
            );
            assert_eq!(reply.status, SUCCESS, "open {path}");
            reply.word(0)
        }

        fn read(&mut self, file_id: u32, offset: u64, length: u32) -> Reply {
            let device_io_request = self.io(MajorFunction::Read, file_id);
            self.send(ServerDriveIoRequest::DeviceReadRequest(DeviceReadRequest {
                device_io_request,
                length,
                offset,
            }))
        }

        fn write(&mut self, file_id: u32, offset: u64, data: &[u8]) -> Reply {
            let device_io_request = self.io(MajorFunction::Write, file_id);
            self.send(ServerDriveIoRequest::DeviceWriteRequest(
                DeviceWriteRequest {
                    device_io_request,
                    offset,
                    write_data: data.to_vec(),
                },
            ))
        }

        fn set_information(&mut self, file_id: u32, set_buffer: FileInformationClass) -> Reply {
            let device_io_request = self.io(MajorFunction::SetInformation, file_id);
            self.send(ServerDriveIoRequest::ServerDriveSetInformationRequest(
                ServerDriveSetInformationRequest {
                    device_io_request,
                    set_buffer,
                },
            ))
        }

        fn query_security(&mut self, file_id: u32, information: SecurityInformation) -> Reply {
            let device_io_request = self.io(MajorFunction::QuerySecurity, file_id);
            self.send(ServerDriveIoRequest::ServerDriveQuerySecurityRequest(
                ServerDriveQuerySecurityRequest {
                    device_io_request,
                    security_information: information,
                },
            ))
        }

        fn set_security(&mut self, file_id: u32, information: SecurityInformation) -> Reply {
            let device_io_request = self.io(MajorFunction::SetSecurity, file_id);
            // A minimal self-relative descriptor: revision 1, SE_SELF_RELATIVE, nothing set.
            let descriptor = vec![
                1, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ];
            self.send(ServerDriveIoRequest::ServerDriveSetSecurityRequest(
                ServerDriveSetSecurityRequest {
                    device_io_request,
                    security_information: information,
                    security_descriptor: descriptor,
                },
            ))
        }

        fn control(&mut self, file_id: u32, code: u32, output_buffer_length: u32) -> Reply {
            let header = self.io(MajorFunction::DeviceControl, file_id);
            self.send(ServerDriveIoRequest::DeviceControlRequest(
                DeviceControlRequest {
                    header,
                    output_buffer_length,
                    input_buffer_length: 0,
                    io_control_code: AnyIoCtlCode(code),
                },
            ))
        }

        /// The raw directory entries for `\*`, query by query until the end.
        fn list(&mut self, folder: u32) -> Vec<u8> {
            let mut entries = Vec::new();
            for initial_query in std::iter::once(1).chain(std::iter::repeat(0)).take(64) {
                let mut device_io_request = self.io(MajorFunction::DirectoryControl, folder);
                device_io_request.minor_function = MinorFunction::IRP_MN_QUERY_DIRECTORY;
                let reply = self.send(ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(
                    ServerDriveQueryDirectoryRequest {
                        device_io_request,
                        file_info_class_lvl: FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
                        initial_query,
                        path: r"\*".into(),
                    },
                ));
                if reply.status != SUCCESS {
                    break;
                }
                entries.extend(reply.body);
            }
            entries
        }

        /// Everything a change could affect: names, bytes, size, times, attributes.
        fn snapshot(&self) -> Vec<(String, Vec<u8>, u64, std::time::SystemTime, bool)> {
            let mut entries: Vec<_> = fs::read_dir(&self.root)
                .unwrap()
                .map(|entry| {
                    let entry = entry.unwrap();
                    let meta = entry.metadata().unwrap();
                    let bytes = if meta.is_file() {
                        fs::read(entry.path()).unwrap()
                    } else {
                        vec![]
                    };
                    let name = entry.file_name().to_string_lossy().into_owned();
                    (
                        name,
                        bytes,
                        meta.len(),
                        meta.modified().unwrap(),
                        meta.permissions().readonly(),
                    )
                })
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries
        }
    }

    #[test]
    fn a_read_only_share_refuses_every_change_and_leaves_bytes_and_metadata_intact() {
        let mut drive = drive(true);
        let before = drive.snapshot();
        let read = || DesiredAccess::GENERIC_READ | DesiredAccess::FILE_READ_ATTRIBUTES;
        let open = CreateDisposition::FILE_OPEN;
        let none = CreateOptions::empty;
        let attempts = [
            (
                r"\data.bin",
                DesiredAccess::GENERIC_WRITE,
                open,
                none(),
                "write access",
            ),
            (
                r"\data.bin",
                read(),
                CreateDisposition::FILE_OVERWRITE_IF,
                none(),
                "overwrite",
            ),
            (
                r"\data.bin",
                read(),
                CreateDisposition::FILE_SUPERSEDE,
                none(),
                "supersede",
            ),
            (
                r"\data.bin",
                DesiredAccess::DELETE,
                open,
                CreateOptions::FILE_DELETE_ON_CLOSE,
                "delete on close",
            ),
            (
                r"\data.bin",
                DesiredAccess::FILE_WRITE_ATTRIBUTES,
                open,
                none(),
                "attributes",
            ),
            (
                r"\data.bin",
                DesiredAccess::WRITE_DAC,
                open,
                none(),
                "permissions",
            ),
            (
                r"\data.bin:extra",
                read(),
                CreateDisposition::FILE_OPEN_IF,
                none(),
                "new stream",
            ),
            (
                r"\new.txt",
                read(),
                CreateDisposition::FILE_CREATE,
                none(),
                "new file",
            ),
            (
                r"\folder",
                read(),
                CreateDisposition::FILE_CREATE,
                CreateOptions::FILE_DIRECTORY_FILE,
                "new folder",
            ),
        ];
        for (path, access, disposition, options, what) in attempts {
            let reply = drive.create(path, access, disposition, options);
            assert_eq!(reply.status, MEDIA_WRITE_PROTECTED, "{what}");
        }

        // Through a read handle: writes, truncation, rename, security, object IDs.
        let file = drive.open(r"\data.bin", read());
        assert_eq!(
            drive.write(file, 0, b"changed").status,
            MEDIA_WRITE_PROTECTED
        );
        let truncate = FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 });
        assert_eq!(
            drive.set_information(file, truncate).status,
            MEDIA_WRITE_PROTECTED
        );
        let rename = FileInformationClass::Rename(FileRenameInformation {
            replace_if_exists: Boolean::False,
            file_name: r"\renamed.bin".into(),
        });
        assert_eq!(
            drive.set_information(file, rename).status,
            MEDIA_WRITE_PROTECTED
        );
        assert_eq!(
            drive.set_security(file, SecurityInformation::DACL).status,
            MEDIA_WRITE_PROTECTED
        );
        const FSCTL_CREATE_OR_GET_OBJECT_ID: u32 = 0x0009_00C0;
        assert_eq!(
            drive
                .control(file, FSCTL_CREATE_OR_GET_OBJECT_ID, 64)
                .status,
            MEDIA_WRITE_PROTECTED
        );
        const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
        assert_eq!(
            drive.control(file, FSCTL_SET_REPARSE_POINT, 0).status,
            NOT_SUPPORTED,
            "unknown control"
        );

        // Listing still works.
        let folder = drive.create(r"\", read(), open, CreateOptions::FILE_DIRECTORY_FILE);
        assert_eq!(folder.status, SUCCESS, "open the shared folder");
        let listed = drive.list(folder.word(0));
        let name: Vec<u8> = "data.bin"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(
            listed.windows(name.len()).any(|w| w == name),
            "data.bin is listed"
        );

        assert_eq!(
            drive.snapshot(),
            before,
            "names, bytes, sizes, times and attributes"
        );
        assert!(
            fs::metadata(drive.root.join("data.bin:extra")).is_err(),
            "no stream was added"
        );
    }

    #[test]
    fn reads_return_exact_short_empty_and_end_of_file_results() {
        let mut drive = drive(true);
        // 64 MiB past the start; the gap reads as zeros.
        let high = 64 * 1024 * 1024;
        let tall = drive.root.join("tall.bin");
        let file = fs::File::create(&tall).unwrap();
        file.set_len(high + 8).unwrap();
        drop(file);

        let read = || DesiredAccess::GENERIC_READ;
        let data = drive.open(r"\data.bin", read());
        let whole = drive.read(data, 0, 4096);
        assert_eq!(
            (whole.status, whole.data()),
            (SUCCESS, CONTENT),
            "a short read returns what exists"
        );
        let middle = drive.read(data, 9, 3);
        assert_eq!(middle.data(), b"byt");
        let empty = drive.read(data, 0, 0);
        assert_eq!(
            (empty.status, empty.data()),
            (SUCCESS, &[][..]),
            "zero length"
        );
        let end = drive.read(data, CONTENT.len() as u64, 16);
        assert_eq!(end.status, END_OF_FILE, "at the end");
        assert!(end.data().is_empty());

        let gap = drive.open(r"\tall.bin", read());
        let zeros = drive.read(gap, high - 8, 32);
        assert_eq!(
            (zeros.status, zeros.data()),
            (SUCCESS, &[0u8; 16][..]),
            "high offset"
        );
    }

    #[test]
    fn privileged_requests_are_refused_in_both_modes_before_the_native_backend() {
        for read_only in [true, false] {
            let mut drive = drive(read_only);
            let sacl = DesiredAccess::GENERIC_READ | DesiredAccess::ACCESS_SYSTEM_SECURITY;
            let reply = drive.create(
                r"\data.bin",
                sacl,
                CreateDisposition::FILE_OPEN,
                CreateOptions::empty(),
            );
            assert_eq!(
                reply.status, PRIVILEGE_NOT_HELD,
                "SACL access, read_only={read_only}"
            );
            assert_eq!(reply.body, [0, 0, 0, 0, 0], "no file ID");

            let file = drive.open(
                r"\data.bin",
                DesiredAccess::GENERIC_READ | DesiredAccess::READ_CONTROL,
            );
            for information in [
                SecurityInformation::SACL,
                SecurityInformation::LABEL,
                SecurityInformation::BACKUP | SecurityInformation::DACL,
            ] {
                let bits = information.bits();
                assert_eq!(
                    drive.query_security(file, information).status,
                    PRIVILEGE_NOT_HELD,
                    "query {bits:#x}"
                );
                assert_eq!(
                    drive.set_security(file, information).status,
                    PRIVILEGE_NOT_HELD,
                    "set {bits:#x}"
                );
            }
            // The ordinary parts still work with the user's own rights.
            let dacl =
                drive.query_security(file, SecurityInformation::OWNER | SecurityInformation::DACL);
            assert_eq!(
                dacl.status, SUCCESS,
                "owner and DACL, read_only={read_only}"
            );
        }
    }

    #[test]
    fn a_read_write_share_writes_with_the_users_own_permissions() {
        let mut drive = drive(false);
        let file = drive.open(
            r"\data.bin",
            DesiredAccess::GENERIC_READ | DesiredAccess::GENERIC_WRITE,
        );
        assert_eq!(drive.write(file, 0, b"replaced").status, SUCCESS);
        assert_eq!(
            fs::read(drive.root.join("data.bin")).unwrap(),
            b"replaced bytes"
        );

        // A file the user may not write stays unchanged, and the remote is told.
        let locked = drive.root.join("locked.bin");
        fs::write(&locked, CONTENT).unwrap();
        let mut permissions = fs::metadata(&locked).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&locked, permissions.clone()).unwrap();
        let reply = drive.create(
            r"\locked.bin",
            DesiredAccess::GENERIC_WRITE,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        );
        assert_eq!(reply.status, ACCESS_DENIED);
        assert_eq!(fs::read(&locked).unwrap(), CONTENT);
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "lets the temporary folder be removed"
        )]
        permissions.set_readonly(false);
        fs::set_permissions(&locked, permissions).unwrap();
    }
}
