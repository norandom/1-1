//! Task 4.3: in-folder operations, with the root bound as in a session.
use super::*;
use ironrdp_rdpdr::pdu::efs::{
    DeviceCloseRequest, DeviceFlushBuffersRequest, FileDispositionInformation,
};
use sha2::{Digest, Sha256};

const OBJECT_NAME_COLLISION: u32 = 0xC000_0035;
const SHARING_VIOLATION: u32 = 0xC000_0043;
const DIRECTORY_NOT_EMPTY: u32 = 0xC000_0101;
const CANNOT_DELETE: u32 = 0xC000_0121;

fn rw() -> DesiredAccess {
    DesiredAccess::GENERIC_READ | DesiredAccess::GENERIC_WRITE
}

fn junction(link: &std::path::Path, target: &std::path::Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(output.status.success(), "mklink /J failed: {output:?}");
}

fn names(folder: &std::path::Path) -> Vec<std::ffi::OsString> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

impl Drive {
    fn new_file(&mut self, path: &str, access: DesiredAccess) -> u32 {
        let reply = self.create(
            path,
            access,
            CreateDisposition::FILE_CREATE,
            CreateOptions::empty(),
        );
        assert_eq!(reply.status, SUCCESS, "create {path}");
        reply.word(0)
    }

    fn open_folder(&mut self, path: &str, access: DesiredAccess) -> Reply {
        self.create(
            path,
            access,
            CreateDisposition::FILE_OPEN,
            CreateOptions::FILE_DIRECTORY_FILE,
        )
    }

    fn close(&mut self, file_id: u32) -> u32 {
        let device_io_request = self.io(MajorFunction::Close, file_id);
        self.send(ServerDriveIoRequest::DeviceCloseRequest(
            DeviceCloseRequest { device_io_request },
        ))
        .status
    }

    fn flush(&mut self, file_id: u32) -> u32 {
        let device_io_request = self.io(MajorFunction::FlushBuffers, file_id);
        self.send(ServerDriveIoRequest::DeviceFlushBuffersRequest(
            DeviceFlushBuffersRequest { device_io_request },
        ))
        .status
    }

    fn rename_to(&mut self, file_id: u32, to: &str, replace: bool) -> u32 {
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

    fn delete(&mut self, file_id: u32) -> u32 {
        let disposition = FileDispositionInformation { delete_pending: 1 };
        self.set_information(file_id, FileInformationClass::Disposition(disposition))
            .status
    }

    fn listed(&mut self, folder: &str, name: &str) -> bool {
        let id = self
            .open_folder(folder, DesiredAccess::GENERIC_READ)
            .word(0);
        let entries = self.list(id, &format!(r"{folder}\*"));
        self.close(id);
        let name: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
        entries.windows(name.len()).any(|w| w == name)
    }
}

#[test]
fn folders_and_files_are_created_written_renamed_and_deleted_inside_the_share() {
    let mut drive = drive_with(false, true);
    let root = drive.root.clone();
    let folder = drive.create(
        r"\work",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_CREATE,
        CreateOptions::FILE_DIRECTORY_FILE,
    );
    assert_eq!(folder.status, SUCCESS);
    assert_eq!(drive.close(folder.word(0)), SUCCESS);
    assert!(root.join("work").is_dir());

    let file = drive.new_file(r"\work\a.txt", rw());
    assert_eq!(drive.write(file, 0, b"hello ").word(0), 6);
    assert_eq!(drive.write(file, 6, b"world").word(0), 5);
    assert_eq!(drive.flush(file), SUCCESS);
    assert_eq!(drive.close(file), SUCCESS);
    assert_eq!(fs::read(root.join(r"work\a.txt")).unwrap(), b"hello world");
    assert!(drive.listed(r"\work", "a.txt"));

    let moving = drive.open(r"\work\a.txt", DesiredAccess::DELETE);
    assert_eq!(drive.rename_to(moving, r"\work\b.txt", false), SUCCESS);
    assert_eq!(drive.close(moving), SUCCESS);
    assert!(!root.join(r"work\a.txt").exists());
    assert_eq!(fs::read(root.join(r"work\b.txt")).unwrap(), b"hello world");

    // An existing name is replaced only on request.
    let other = drive.new_file(r"\work\c.txt", rw());
    drive.close(other);
    let moving = drive.open(r"\work\b.txt", DesiredAccess::DELETE);
    assert_eq!(
        drive.rename_to(moving, r"\work\c.txt", false),
        OBJECT_NAME_COLLISION
    );
    drive.close(moving);

    // A folder with content is not deleted.
    let busy = drive.open_folder(r"\work", DesiredAccess::DELETE).word(0);
    assert_eq!(drive.delete(busy), DIRECTORY_NOT_EMPTY);
    drive.close(busy);

    let doomed = drive.open(r"\work\b.txt", DesiredAccess::DELETE);
    assert_eq!(drive.delete(doomed), SUCCESS);
    assert_eq!(drive.close(doomed), SUCCESS);
    let on_close = drive.create(
        r"\work\c.txt",
        DesiredAccess::DELETE,
        CreateDisposition::FILE_OPEN,
        CreateOptions::FILE_DELETE_ON_CLOSE,
    );
    assert_eq!(on_close.status, SUCCESS);
    assert_eq!(drive.close(on_close.word(0)), SUCCESS);
    let empty = drive.open_folder(r"\work", DesiredAccess::DELETE).word(0);
    assert_eq!(drive.delete(empty), SUCCESS);
    assert_eq!(drive.close(empty), SUCCESS);
    assert!(!root.join("work").exists());
}

#[test]
fn a_remote_copy_matches_the_source_byte_for_byte() {
    let mut drive = drive_with(false, true);
    // 3 MiB of varied bytes, more than one I/O chunk.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let source: Vec<u8> = (0..3 * 1024 * 1024 + 123)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect();
    fs::write(drive.root.join("source.bin"), &source).unwrap();

    let from = drive.open(r"\source.bin", DesiredAccess::GENERIC_READ);
    let to = drive.new_file(r"\copy.bin", rw());
    let chunk = 256 * 1024;
    let mut offset = 0u64;
    loop {
        let read = drive.read(from, offset, chunk);
        if read.status == END_OF_FILE || read.data().is_empty() {
            break;
        }
        assert_eq!(read.status, SUCCESS);
        let data = read.data().to_vec();
        assert_eq!(drive.write(to, offset, &data).word(0) as usize, data.len());
        offset += data.len() as u64;
    }
    assert_eq!((drive.close(from), drive.close(to)), (SUCCESS, SUCCESS));

    let hash = |name: &str| Sha256::digest(fs::read(drive.root.join(name)).unwrap());
    assert_eq!(offset, source.len() as u64);
    assert_eq!(hash("copy.bin"), Sha256::digest(&source));
    assert_eq!(hash("copy.bin"), hash("source.bin"));
}

#[test]
fn the_shared_folder_itself_cannot_be_renamed_or_deleted() {
    let mut drive = drive_with(false, true);
    let delete = drive.open_folder(r"\", DesiredAccess::GENERIC_READ | DesiredAccess::DELETE);
    assert_eq!(delete.status, ACCESS_DENIED);
    let on_close = drive.create(
        "",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_OPEN,
        CreateOptions::FILE_DIRECTORY_FILE | CreateOptions::FILE_DELETE_ON_CLOSE,
    );
    assert_eq!(on_close.status, ACCESS_DENIED);
    // Broader access that would include DELETE either fails on the root guard
    // or still cannot rename or delete the root.
    let broad = drive.open_folder(r"\", DesiredAccess::MAXIMUM_ALLOWED);
    if broad.status == SUCCESS {
        let root = broad.word(0);
        assert_ne!(drive.rename_to(root, r"\renamed", false), SUCCESS);
        assert_ne!(drive.delete(root), SUCCESS);
        drive.close(root);
    }
    assert!(drive.root.is_dir());
    assert!(drive.root.join("data.bin").is_file());
}

#[test]
fn nothing_reaches_outside_the_share_even_through_a_replaced_parent() {
    let mut drive = drive_with(false, true);
    let (_outside_dir, outside) = real_dir();
    fs::write(outside.join("sentinel.txt"), b"outside").unwrap();
    let before = names(&outside);

    fs::create_dir(drive.root.join("sub")).unwrap();
    let file = drive.new_file(r"\a.txt", rw() | DesiredAccess::DELETE);
    drive.write(file, 0, b"inside");
    assert_ne!(
        drive.rename_to(file, r"\..\a.txt", false),
        SUCCESS,
        "parent path"
    );

    // The rename target's folder is swapped for a junction to the outside.
    fs::rename(drive.root.join("sub"), drive.root.join("sub-old")).unwrap();
    junction(&drive.root.join("sub"), &outside);
    assert_ne!(
        drive.rename_to(file, r"\sub\a.txt", false),
        SUCCESS,
        "through a junction"
    );
    let through = drive.create(
        r"\sub\new.txt",
        rw(),
        CreateDisposition::FILE_CREATE,
        CreateOptions::empty(),
    );
    assert_ne!(through.status, SUCCESS, "create through a junction");
    assert_eq!(drive.close(file), SUCCESS);

    assert_eq!(names(&outside), before, "nothing new outside");
    assert_eq!(fs::read(outside.join("sentinel.txt")).unwrap(), b"outside");
    assert_eq!(
        fs::read(drive.root.join("a.txt")).unwrap(),
        b"inside",
        "the file stayed in place"
    );
    fs::remove_dir(drive.root.join("sub")).unwrap();
}

#[test]
fn local_programs_and_permissions_are_reported_to_the_remote() {
    use std::os::windows::fs::OpenOptionsExt;
    let mut drive = drive_with(false, true);
    let busy = drive.root.join("busy.txt");
    fs::write(&busy, b"local").unwrap();
    let exclusive = fs::File::options()
        .read(true)
        .share_mode(0)
        .open(&busy)
        .unwrap();
    let reply = drive.create(
        r"\busy.txt",
        DesiredAccess::GENERIC_READ,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_eq!(reply.status, SHARING_VIOLATION, "held exclusively");
    drop(exclusive);
    // Read and write sharing without delete sharing: no rename or delete.
    let editing = fs::File::options()
        .read(true)
        .share_mode(0x1 | 0x2)
        .open(&busy)
        .unwrap();
    let reply = drive.create(
        r"\busy.txt",
        DesiredAccess::DELETE,
        CreateDisposition::FILE_OPEN,
        CreateOptions::empty(),
    );
    assert_eq!(reply.status, SHARING_VIOLATION, "open in a local editor");
    drop(editing);

    let locked = drive.root.join("locked.txt");
    fs::write(&locked, b"keep").unwrap();
    let mut permissions = fs::metadata(&locked).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&locked, permissions.clone()).unwrap();
    let file = drive.open(r"\locked.txt", DesiredAccess::DELETE);
    assert_eq!(drive.delete(file), CANNOT_DELETE, "a read-only file");
    drive.close(file);
    assert_eq!(fs::read(&locked).unwrap(), b"keep");
    #[allow(
        clippy::permissions_set_readonly_false,
        reason = "lets the temporary folder be removed"
    )]
    permissions.set_readonly(false);
    fs::set_permissions(&locked, permissions).unwrap();
}
