//! Linux and macOS folder sharing against the real native drive backend
//! (rdp-folder-sharing tasks 7.2 to 7.4).

use super::{BindError, DriveRouter, UnixBoundRoot, link_count, open_folder};
use crate::rdp::folder_share::{ShareBackend, ShareRoot, identity_of};
use ironrdp_rdpdr::backend::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest,
    DeviceCreateRequest, DeviceIoRequest, DeviceReadRequest, DeviceWriteRequest, FileAttributes,
    FileDispositionInformation, FileEndOfFileInformation, FileInformationClass,
    FileRenameInformation, MajorFunction, MinorFunction, SecurityInformation, ServerDriveIoRequest,
    ServerDriveSetInformationRequest, ServerDriveSetSecurityRequest, SharedAccess,
};
use ironrdp_rdpdr_native::backend::NixRdpdrBackend;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SUCCESS: u32 = 0;
const ACCESS_DENIED: u32 = 0xC000_0022;
const MEDIA_WRITE_PROTECTED: u32 = 0xC000_00A2;
const FIRST: u32 = 27;
const CONTENT: &[u8] = b"original bytes";

/// A temporary folder at its real location (macOS reaches /var through a link).
fn real_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    (dir, path)
}

fn bind(path: &Path) -> Result<UnixBoundRoot, BindError> {
    let (volume, file) = identity_of(path);
    UnixBoundRoot::bind(path, volume, file)
}

/// Fields drop in order: the backend's handles, the bound folders, the base.
struct Drives {
    backend: ShareBackend,
    _bound: Vec<UnixBoundRoot>,
    folders: Vec<PathBuf>,
    base: PathBuf,
    device: u32,
    completion: u32,
    _dir: tempfile::TempDir,
}

struct Reply {
    status: u32,
    body: Vec<u8>,
}

impl Reply {
    fn word(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.body[at..at + 4].try_into().unwrap())
    }
    fn data(&self) -> &[u8] {
        &self.body[4..4 + self.word(0) as usize]
    }
}

/// `count` folders as drives `FIRST..`, each holding `data.bin`, bound and
/// routed as a session does.
fn drives(read_only: bool, count: u32) -> Drives {
    let (dir, base) = real_dir();
    let mut bound = Vec::new();
    let mut folders = Vec::new();
    let mut roots: HashMap<u32, ShareRoot> = HashMap::new();
    let mut natives: HashMap<u32, Box<dyn RdpdrBackend>> = HashMap::new();
    for index in 0..count {
        let folder = base.join(format!("share{index}"));
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("data.bin"), CONTENT).unwrap();
        let root = bind(&folder).unwrap();
        let device = FIRST + index;
        roots.insert(device, root.root());
        natives.insert(
            device,
            Box::new(NixRdpdrBackend::new(root.file_base().to_owned())),
        );
        bound.push(root);
        folders.push(folder);
    }
    let router = DriveRouter::new(natives);
    let mut backend = ShareBackend::with_roots(Box::new(router), roots);
    if read_only {
        backend = backend.with_read_only(FIRST..FIRST + count);
    }
    Drives {
        backend,
        _bound: bound,
        folders,
        base,
        device: FIRST,
        completion: 0,
        _dir: dir,
    }
}

impl Drives {
    fn root(&self) -> &Path {
        &self.folders[(self.device - FIRST) as usize]
    }

    fn io(&mut self, major_function: MajorFunction, file_id: u32) -> DeviceIoRequest {
        self.completion += 1;
        DeviceIoRequest {
            device_id: self.device,
            file_id,
            completion_id: self.completion,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

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
        assert_eq!(word(4), self.device, "the reply names this device");
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

    fn rename(&mut self, file_id: u32, to: &str, replace: bool) -> u32 {
        let rename = FileInformationClass::Rename(FileRenameInformation {
            replace_if_exists: if replace {
                Boolean::True
            } else {
                Boolean::False
            },
            file_name: to.into(),
        });
        self.set_information(file_id, rename).status
    }

    fn set_security(&mut self, file_id: u32) -> Reply {
        let device_io_request = self.io(MajorFunction::SetSecurity, file_id);
        self.send(ServerDriveIoRequest::ServerDriveSetSecurityRequest(
            ServerDriveSetSecurityRequest {
                device_io_request,
                security_information: SecurityInformation::DACL,
                security_descriptor: vec![
                    1, 0, 0, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
            },
        ))
    }

    fn close(&mut self, file_id: u32) -> u32 {
        let device_io_request = self.io(MajorFunction::Close, file_id);
        self.send(ServerDriveIoRequest::DeviceCloseRequest(
            DeviceCloseRequest { device_io_request },
        ))
        .status
    }

    /// Names, bytes, sizes, modification times and permissions under the base.
    fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>, u64, std::time::SystemTime, bool)> {
        let mut entries = Vec::new();
        let mut pending = vec![self.base.clone()];
        while let Some(folder) = pending.pop() {
            for entry in fs::read_dir(&folder).unwrap() {
                let entry = entry.unwrap();
                let meta = fs::symlink_metadata(entry.path()).unwrap();
                if meta.is_dir() {
                    pending.push(entry.path());
                }
                let bytes = if meta.is_file() {
                    fs::read(entry.path()).unwrap()
                } else {
                    vec![]
                };
                entries.push((
                    entry.path(),
                    bytes,
                    meta.len(),
                    meta.modified().unwrap(),
                    meta.permissions().readonly(),
                ));
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

#[test]
fn binding_refuses_links_replacements_missing_folders_and_files() {
    let (_dir, base) = real_dir();
    let real = base.join("real");
    fs::create_dir(&real).unwrap();
    let (volume, file) = identity_of(&real);

    std::os::unix::fs::symlink(&real, base.join("link")).unwrap();
    assert_eq!(
        UnixBoundRoot::bind(&base.join("link"), volume, file).unwrap_err(),
        BindError::Link
    );
    fs::create_dir(base.join("parent")).unwrap();
    std::os::unix::fs::symlink(&base, base.join("parent").join("up")).unwrap();
    assert_eq!(
        UnixBoundRoot::bind(&base.join("parent/up/real"), volume, file).unwrap_err(),
        BindError::Link,
        "a link above the folder is refused too"
    );

    fs::rename(&real, base.join("original")).unwrap();
    fs::create_dir(&real).unwrap();
    assert_eq!(
        UnixBoundRoot::bind(&real, volume, file).unwrap_err(),
        BindError::Replaced
    );
    fs::remove_dir(&real).unwrap();
    assert_eq!(
        UnixBoundRoot::bind(&real, volume, file).unwrap_err(),
        BindError::Missing
    );
    fs::write(&real, b"file").unwrap();
    assert_eq!(
        UnixBoundRoot::bind(&real, volume, file).unwrap_err(),
        BindError::NotFolder
    );
    assert_eq!(
        UnixBoundRoot::bind(Path::new("relative"), volume, file).unwrap_err(),
        BindError::NotPlain
    );
}

#[test]
fn a_renamed_folder_is_still_served_and_a_folder_taking_its_name_is_not() {
    let mut drive = drives(false, 1);
    let moved = drive.base.join("moved");
    fs::rename(drive.root(), &moved).unwrap();
    fs::create_dir(drive.root()).unwrap();

    let created = drive.create(
        r"\new.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_CREATE,
        CreateOptions::empty(),
    );
    assert_eq!(created.status, SUCCESS);
    assert!(
        moved.join("new.txt").is_file(),
        "the held folder got the file"
    );
    assert!(
        !drive.root().join("new.txt").exists(),
        "the impostor did not"
    );
}

#[test]
fn a_deleted_folder_fails_requests() {
    let mut drive = drives(false, 1);
    let root = drive.root().to_path_buf();
    fs::remove_dir_all(&root).unwrap();
    let created = drive.create(
        r"\new.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_CREATE,
        CreateOptions::empty(),
    );
    assert_ne!(created.status, SUCCESS);
    assert!(!root.exists());
}

#[test]
fn read_only_refuses_every_change_at_open_and_leaves_everything_unchanged() {
    let mut drive = drives(true, 1);
    fs::create_dir(drive.root().join("folder")).unwrap();
    let before = drive.snapshot();
    let refused = [
        (
            r"\data.bin",
            DesiredAccess::GENERIC_WRITE,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::FILE_WRITE_ATTRIBUTES,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::DELETE,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::WRITE_DAC,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::MAXIMUM_ALLOWED,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::GENERIC_ALL,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_OVERWRITE,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_SUPERSEDE,
            CreateOptions::empty(),
        ),
        (
            r"\data.bin",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_OPEN,
            CreateOptions::FILE_DELETE_ON_CLOSE,
        ),
        (
            r"\new.bin",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_CREATE,
            CreateOptions::empty(),
        ),
        (
            r"\new.bin",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_OPEN_IF,
            CreateOptions::empty(),
        ),
        (
            r"\folder\new",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_CREATE,
            CreateOptions::FILE_DIRECTORY_FILE,
        ),
    ];
    for (path, access, disposition, options) in refused {
        let label = format!("{path} {access:?} {disposition:?} {options:?}");
        let reply = drive.create(path, access, disposition, options);
        assert_eq!(reply.status, MEDIA_WRITE_PROTECTED, "{label}");
    }
    assert_eq!(drive.snapshot(), before);

    // Reading still works.
    let file = drive.open(r"\data.bin", DesiredAccess::GENERIC_READ);
    let read = drive.read(file, 0, 64);
    assert_eq!(read.status, SUCCESS);
    assert_eq!(read.data(), CONTENT);
    assert_eq!(drive.close(file), SUCCESS);
}

#[test]
fn read_only_refuses_changes_through_a_read_handle() {
    let mut drive = drives(true, 1);
    let before = drive.snapshot();
    let file = drive.open(r"\data.bin", DesiredAccess::GENERIC_READ);
    assert_eq!(
        drive.write(file, 0, b"changed").status,
        MEDIA_WRITE_PROTECTED
    );
    let truncate = FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 });
    assert_eq!(
        drive.set_information(file, truncate).status,
        MEDIA_WRITE_PROTECTED
    );
    assert_eq!(
        drive.rename(file, r"\renamed.bin", false),
        MEDIA_WRITE_PROTECTED
    );
    let delete =
        FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 });
    assert_eq!(
        drive.set_information(file, delete).status,
        MEDIA_WRITE_PROTECTED
    );
    assert_eq!(drive.set_security(file).status, MEDIA_WRITE_PROTECTED);
    assert_eq!(drive.close(file), SUCCESS);
    assert_eq!(drive.snapshot(), before);
}

#[test]
fn multiply_linked_files_are_read_only_in_a_read_write_share() {
    let mut drive = drives(false, 1);
    let outside = drive.base.join("outside.bin");
    fs::hard_link(drive.root().join("data.bin"), &outside).unwrap();
    fs::write(drive.root().join("other.bin"), b"other").unwrap();

    let reply = drive.create(
        r"\data.bin",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_eq!(
        reply.status, ACCESS_DENIED,
        "no write through a second name"
    );
    let file = drive.open(r"\data.bin", DesiredAccess::GENERIC_READ);
    assert_eq!(drive.read(file, 0, 64).data(), CONTENT, "reading is fine");
    drive.close(file);

    let other = drive.open(r"\other.bin", DesiredAccess::DELETE);
    assert_eq!(
        drive.rename(other, r"\data.bin", true),
        ACCESS_DENIED,
        "replacing a linked file is refused"
    );
    drive.close(other);
    assert_eq!(fs::read(&outside).unwrap(), CONTENT);

    // Directories have several links on Unix; that does not make them read-only.
    fs::create_dir_all(drive.root().join("folder/sub")).unwrap();
    let created = drive.create(
        r"\folder\new.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_CREATE,
        CreateOptions::empty(),
    );
    assert_eq!(created.status, SUCCESS);
}

#[test]
fn a_read_write_share_creates_writes_renames_and_deletes() {
    let mut drive = drives(false, 1);
    let folder = drive.create(
        r"\work",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_CREATE,
        CreateOptions::FILE_DIRECTORY_FILE,
    );
    assert_eq!(folder.status, SUCCESS);
    drive.close(folder.word(0));

    let payload: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let created = drive.create(
        r"\work\copy.bin",
        DesiredAccess::GENERIC_WRITE | DesiredAccess::GENERIC_READ | DesiredAccess::DELETE,
        CreateDisposition::FILE_CREATE,
        CreateOptions::empty(),
    );
    assert_eq!(created.status, SUCCESS);
    let file = created.word(0);
    for (index, chunk) in payload.chunks(256 * 1024).enumerate() {
        let reply = drive.write(file, (index * 256 * 1024) as u64, chunk);
        assert_eq!(reply.status, SUCCESS);
    }
    assert_eq!(drive.rename(file, r"\work\renamed.bin", false), SUCCESS);
    assert_eq!(drive.close(file), SUCCESS);
    assert_eq!(
        fs::read(drive.root().join("work/renamed.bin")).unwrap(),
        payload
    );

    let file = drive.open(r"\work\renamed.bin", DesiredAccess::DELETE);
    let delete =
        FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 });
    assert_eq!(drive.set_information(file, delete).status, SUCCESS);
    assert_eq!(drive.close(file), SUCCESS);
    assert!(!drive.root().join("work/renamed.bin").exists());
}

#[test]
fn the_share_root_cannot_be_deleted_and_names_cannot_leave_it() {
    let mut drive = drives(false, 1);
    let reply = drive.create(
        r"\",
        DesiredAccess::DELETE,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_eq!(reply.status, ACCESS_DENIED);
    fs::write(drive.base.join("secret.txt"), b"secret").unwrap();
    let escaped = drive.create(
        r"\..\secret.txt",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_ne!(escaped.status, SUCCESS);
    std::os::unix::fs::symlink(drive.base.join("secret.txt"), drive.root().join("link")).unwrap();
    let linked = drive.create(
        r"\link",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_ne!(
        linked.status, SUCCESS,
        "links inside the share are not followed"
    );
}

#[test]
fn each_folder_is_its_own_drive() {
    let mut drive = drives(false, 2);
    drive.device = FIRST + 1;
    let created = drive.create(
        r"\notes.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_CREATE,
        CreateOptions::empty(),
    );
    assert_eq!(created.status, SUCCESS);
    assert!(drive.folders[1].join("notes.txt").is_file());
    assert!(!drive.folders[0].join("notes.txt").exists());
    let across = drive.create(
        r"\..\share0\data.bin",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_ne!(across.status, SUCCESS, "one drive cannot reach another");
}

#[test]
fn forward_slash_paths_from_xrdp_are_served() {
    let mut drive = drives(true, 1);
    let file = drive.open("/data.bin", DesiredAccess::GENERIC_READ);
    assert_eq!(drive.read(file, 0, 64).data(), CONTENT);
}

#[test]
fn link_counts_resolve_below_the_held_folder_without_following_links() {
    let (_dir, base) = real_dir();
    fs::create_dir_all(base.join("a/b")).unwrap();
    fs::write(base.join("a/file"), b"x").unwrap();
    fs::hard_link(base.join("a/file"), base.join("twin")).unwrap();
    std::os::unix::fs::symlink(base.join("a"), base.join("link")).unwrap();
    let root = open_folder(&base).unwrap();

    assert_eq!(
        link_count(&root, r"\a").unwrap(),
        Some(1),
        "directories count one"
    );
    assert_eq!(link_count(&root, r"\a\file").unwrap(), Some(2));
    assert_eq!(link_count(&root, r"\a\missing").unwrap(), None);
    assert_eq!(link_count(&root, r"\missing\file").unwrap(), None);
    assert_eq!(link_count(&root, r"\").unwrap(), None);
    assert!(
        link_count(&root, r"\link\file").is_err(),
        "no link is followed"
    );
    assert!(link_count(&root, r"\a\..\twin").is_err());
}
