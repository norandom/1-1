//! Task 5.2: a folder is offered only to the reviewed server at the granted
//! address, only while it is the chosen folder, and with the chosen mode.
use super::*;
use crate::rdp::folder_share::identity_of;
use crate::rdp::peer_identity::tests::{
    OTHER_KEY, SELF_SIGNED, SERVER, SERVER_KEY, endpoint, fixture,
};
use ironrdp_rdpdr::backend::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest,
    FileAttributes, MajorFunction, MinorFunction, ServerDriveIoRequest, SharedAccess,
};
use rustls::version;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

const ADDRESS: (&str, u16) = ("desktop.example.test", 3389);
const MEDIA_WRITE_PROTECTED: u32 = 0xC000_00A2;

fn real_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let resolved = fs::canonicalize(dir.path()).unwrap();
    let text = resolved.to_string_lossy().into_owned();
    let path = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
    (dir, path)
}

/// A folder and a TLS connection from the server the user reviewed.
struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    grant: ShareGrant,
}

async fn setup(mode: ShareMode) -> Setup {
    let (dir, base) = real_dir();
    let root = base.join("shared");
    fs::create_dir(&root).unwrap();
    let (volume_id, directory_file_id) = identity_of(&root);
    let reviewed = fixture(
        &endpoint(ADDRESS.0, ADDRESS.1),
        SELF_SIGNED,
        OTHER_KEY,
        &version::TLS13,
    )
    .await
    .unwrap();
    let grant = ShareGrant {
        profile_id: "host".into(),
        endpoint: peer_identity::endpoint_key(&endpoint(ADDRESS.0, ADDRESS.1)),
        spki_sha256: reviewed.observed().spki_sha256,
        root_path: root.clone(),
        volume_id,
        directory_file_id,
        share_name: "Work".into(),
        mode,
        revision: uuid::Uuid::new_v4(),
    };
    Setup {
        _dir: dir,
        root,
        grant,
    }
}

async fn connect(
    host: (&str, u16),
    cert: &'static [u8],
    key: &'static [u8],
    grant: ShareGrant,
) -> Result<Rdpdr, ShareState> {
    let destination = endpoint(host.0, host.1);
    let tls = fixture(&destination, cert, key, &version::TLS13)
        .await
        .unwrap();
    match drive_channel(&tls, &destination, vec![grant]).await {
        (Some(channel), _) => Ok(channel.0),
        (None, statuses) => Err(statuses[0].get().state),
    }
}

fn movable(root: &std::path::Path) -> bool {
    let moved = root.with_file_name("moved");
    let ok = fs::rename(root, &moved).is_ok();
    if ok {
        fs::rename(&moved, root).unwrap();
    }
    ok
}

fn eventually(mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// Open `path` for `access` on the first attached drive and return the reply status.
fn open(
    rdpdr: &mut Rdpdr,
    path: &str,
    access: DesiredAccess,
    disposition: CreateDisposition,
) -> u32 {
    open_on(rdpdr, FIRST_SHARE_DEVICE_ID, path, access, disposition)
}

/// As `open`, on a given drive device.
fn open_on(
    rdpdr: &mut Rdpdr,
    device_id: u32,
    path: &str,
    access: DesiredAccess,
    disposition: CreateDisposition,
) -> u32 {
    let worker = rdpdr.downcast_backend_mut::<WorkerBackend>().unwrap();
    let request = DeviceCreateRequest {
        device_io_request: DeviceIoRequest {
            device_id,
            file_id: 0,
            completion_id: 1,
            major_function: MajorFunction::Create,
            minor_function: MinorFunction::from(0),
        },
        desired_access: access,
        allocation_size: 0,
        file_attributes: FileAttributes::empty(),
        shared_access: SharedAccess::empty(),
        create_disposition: disposition,
        create_options: CreateOptions::empty(),
        path: path.into(),
    };
    assert!(
        worker
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(request))
            .unwrap()
            .is_empty()
    );
    let mut replies = Vec::new();
    assert!(eventually(|| {
        replies.extend(worker.poll_deferred_messages().unwrap());
        !replies.is_empty()
    }));
    let bytes = replies[0].encode_unframed_pdu().unwrap();
    u32::from_le_bytes(bytes[12..16].try_into().unwrap())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reviewed_server_gets_the_folder_and_holds_it_in_place_until_the_channel_ends() {
    let s = setup(ShareMode::ReadWrite).await;
    let mut rdpdr = connect(ADDRESS, SELF_SIGNED, OTHER_KEY, s.grant.clone())
        .await
        .unwrap();
    assert!(!movable(&s.root), "bound while the channel lives");

    // The channel's own start activates the drive; then files can be created.
    let worker = rdpdr.downcast_backend_mut::<WorkerBackend>().unwrap();
    worker.reset().unwrap();
    worker.restore_drive(FIRST_SHARE_DEVICE_ID).unwrap();
    let created = open(
        &mut rdpdr,
        r"\new.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_CREATE,
    );
    assert_eq!(created, 0);
    assert!(s.root.join("new.txt").is_file());

    drop(rdpdr);
    assert!(
        eventually(|| movable(&s.root)),
        "released after the channel ends"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_status_follows_the_drive_from_setup_to_its_end() {
    let s = setup(ShareMode::ReadOnly).await;
    let destination = endpoint(ADDRESS.0, ADDRESS.1);
    let tls = fixture(&destination, SELF_SIGNED, OTHER_KEY, &version::TLS13)
        .await
        .unwrap();
    let (rdpdr, statuses) = drive_channel(&tls, &destination, vec![s.grant.clone()]).await;
    let rdpdr = rdpdr.map(|channel| channel.0);
    let (mut rdpdr, status) = (rdpdr.unwrap(), statuses[0].clone());
    let current = status.get();
    assert_eq!(current.state, ShareState::Negotiating);
    assert_eq!(
        (current.name.as_deref(), current.read_only),
        (Some("Work"), Some(true))
    );

    let worker = rdpdr.downcast_backend_mut::<WorkerBackend>().unwrap();
    worker.reset().unwrap();
    worker.restore_drive(FIRST_SHARE_DEVICE_ID).unwrap();
    let mut raw = ironrdp_rdpdr::Rdpdr::new(
        Box::new(
            ironrdp_rdpdr_native::WindowsRdpdrBackendFactory::new(
                ironrdp_rdpdr_native::RedirectedDrive::new(9, "Other", &s.root, true).unwrap(),
            )
            .build(),
        ),
        "x".into(),
    );
    worker.handle_user_logged_on(&mut raw).unwrap();
    assert_eq!(status.get().state, ShareState::Offered);
    worker
        .handle_server_device_announce_response(
            ironrdp_rdpdr::pdu::efs::ServerDeviceAnnounceResponse {
                device_id: FIRST_SHARE_DEVICE_ID,
                result_code: ironrdp_rdpdr::pdu::efs::NtStatus::SUCCESS,
            },
        )
        .unwrap();
    assert_eq!(status.get().state, ShareState::Accepted);

    // A refused write is a failure, not access.
    fs::write(s.root.join("data.txt"), b"keep").unwrap();
    let refused = open(
        &mut rdpdr,
        r"\data.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_OPEN,
    );
    assert_eq!(refused, MEDIA_WRITE_PROTECTED);
    let current = status.get();
    assert_eq!(current.state, ShareState::Accepted);
    assert_eq!(
        current.last_failure.unwrap().description,
        "The share is read-only"
    );
    // A successful read is.
    assert_eq!(
        open(
            &mut rdpdr,
            r"\data.txt",
            DesiredAccess::GENERIC_READ,
            CreateDisposition::FILE_OPEN
        ),
        0
    );
    assert_eq!(status.get().state, ShareState::AccessObserved);

    drop(rdpdr);
    assert_eq!(status.get().state, ShareState::Ended);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_grant_attaches_a_read_only_drive() {
    let s = setup(ShareMode::ReadOnly).await;
    fs::write(s.root.join("data.txt"), b"keep").unwrap();
    let mut rdpdr = connect(ADDRESS, SELF_SIGNED, OTHER_KEY, s.grant.clone())
        .await
        .unwrap();
    let worker = rdpdr.downcast_backend_mut::<WorkerBackend>().unwrap();
    worker.reset().unwrap();
    worker.restore_drive(FIRST_SHARE_DEVICE_ID).unwrap();
    let status = open(
        &mut rdpdr,
        r"\data.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_OPEN,
    );
    assert_eq!(status, MEDIA_WRITE_PROTECTED);
    assert_eq!(fs::read(s.root.join("data.txt")).unwrap(), b"keep");
}

#[tokio::test(flavor = "multi_thread")]
async fn another_address_or_server_key_gets_no_folder() {
    let s = setup(ShareMode::ReadWrite).await;
    let moved = connect(
        ("desktop.example.test", 3390),
        SELF_SIGNED,
        OTHER_KEY,
        s.grant.clone(),
    )
    .await;
    assert_eq!(moved.unwrap_err(), NotOffered::AddressChanged.state());
    let rotated = connect(ADDRESS, SERVER, SERVER_KEY, s.grant.clone()).await;
    assert_eq!(rotated.unwrap_err(), NotOffered::IdentityChanged.state());
    assert!(movable(&s.root), "nothing was bound");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replaced_or_missing_folder_gets_no_folder() {
    let s = setup(ShareMode::ReadWrite).await;
    fs::rename(&s.root, s.root.with_file_name("original")).unwrap();
    fs::create_dir(&s.root).unwrap();
    let replaced = connect(ADDRESS, SELF_SIGNED, OTHER_KEY, s.grant.clone()).await;
    assert!(matches!(
        replaced.unwrap_err(),
        ShareState::FolderUnavailable { .. }
    ));
    fs::remove_dir(&s.root).unwrap();
    let missing = connect(ADDRESS, SELF_SIGNED, OTHER_KEY, s.grant.clone()).await;
    assert!(matches!(
        missing.unwrap_err(),
        ShareState::FolderUnavailable { .. }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_sessions_can_share_the_same_folder_independently() {
    let s = setup(ShareMode::ReadWrite).await;
    let first = connect(ADDRESS, SELF_SIGNED, OTHER_KEY, s.grant.clone())
        .await
        .unwrap();
    let second = connect(ADDRESS, SELF_SIGNED, OTHER_KEY, s.grant.clone())
        .await
        .unwrap();
    drop(first);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!movable(&s.root), "the other session still holds it");
    drop(second);
    assert!(eventually(|| movable(&s.root)));
}

#[tokio::test(flavor = "multi_thread")]
async fn several_folders_become_separate_drives_and_a_missing_one_is_left_out() {
    let s = setup(ShareMode::ReadWrite).await;
    let second = s.root.with_file_name("second");
    fs::create_dir(&second).unwrap();
    let (volume_id, directory_file_id) = identity_of(&second);
    let docs = ShareGrant {
        root_path: second.clone(),
        volume_id,
        directory_file_id,
        share_name: "Docs".into(),
        ..s.grant.clone()
    };
    let gone = ShareGrant {
        root_path: s.root.with_file_name("gone"),
        share_name: "Gone".into(),
        ..s.grant.clone()
    };
    let destination = endpoint(ADDRESS.0, ADDRESS.1);
    let tls = fixture(&destination, SELF_SIGNED, OTHER_KEY, &version::TLS13)
        .await
        .unwrap();
    let (rdpdr, statuses) =
        drive_channel(&tls, &destination, vec![s.grant.clone(), docs, gone]).await;
    let rdpdr = rdpdr.map(|channel| channel.0);
    let states: Vec<_> = statuses.iter().map(|status| status.get()).collect();
    assert_eq!(states[0].state, ShareState::Negotiating);
    assert_eq!(states[1].state, ShareState::Negotiating);
    assert!(matches!(
        states[2].state,
        ShareState::FolderUnavailable { .. }
    ));
    assert_eq!(states[1].name.as_deref(), Some("Docs"));
    assert!(
        !movable(&s.root) && !movable(&second),
        "both folders are bound"
    );

    // The second folder is the next device ID, and its files land in it.
    let mut rdpdr = rdpdr.unwrap();
    let worker = rdpdr.downcast_backend_mut::<WorkerBackend>().unwrap();
    worker.reset().unwrap();
    worker.restore_drive(FIRST_SHARE_DEVICE_ID).unwrap();
    worker.restore_drive(FIRST_SHARE_DEVICE_ID + 1).unwrap();
    let created = open_on(
        &mut rdpdr,
        FIRST_SHARE_DEVICE_ID + 1,
        r"\notes.txt",
        DesiredAccess::GENERIC_WRITE,
        CreateDisposition::FILE_CREATE,
    );
    assert_eq!(created, 0);
    assert!(second.join("notes.txt").is_file());
    assert!(!s.root.join("notes.txt").exists());

    drop(rdpdr);
    assert!(eventually(|| movable(&s.root) && movable(&second)));
}
