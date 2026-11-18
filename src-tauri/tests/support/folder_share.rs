use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::os::windows::{fs::MetadataExt, io::AsRawHandle};
use std::path::PathBuf;

use ironrdp_core::{ReadCursor, WriteCursor, decode, encode_vec};
use ironrdp_rdpdr::Rdpdr;
use ironrdp_rdpdr::pdu::efs::{
    CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceCreateResponse,
    DeviceIoRequest, DeviceIoResponse, NtStatus, SharedAccess,
};
use ironrdp_rdpdr::pdu::{PacketId, RdpdrPdu, SharedHeader};
use ironrdp_rdpdr::server::{NoopRdpdrServerBackend, RdpdrServer};
use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackendFactory};
use ironrdp_svc::{SvcMessage, SvcProcessor};
use tempfile::TempDir;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_SPARSE_FILE;
use windows::Win32::System::{IO::DeviceIoControl, Ioctl::FSCTL_SET_SPARSE};

pub const DEVICE_ID: u32 = 27;
pub const HIGH_OFFSET: u64 = (1 << 32) + 37;
const SHARE_NAME: &str = "Fixture";

pub struct FolderShareFixture {
    sandbox: TempDir,
    pub selected: PathBuf,
}

impl FolderShareFixture {
    pub fn new() -> Self {
        let parent = std::env::var_os("DRIFT_RUST_TEST_ARTIFACT_DIR")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute() && dir.parent().is_some())
            .unwrap_or_else(|| std::env::temp_dir().join("drift-rdp"))
            .join("folder-sharing");
        fs::create_dir_all(&parent).unwrap();
        let sandbox = tempfile::Builder::new()
            .prefix("drive-")
            .tempdir_in(parent)
            .unwrap();
        let selected = sandbox.path().join("selected");
        let outside = sandbox.path().join("outside");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("readonly.txt"), b"outside read sentinel").unwrap();
        fs::write(outside.join("writable.txt"), b"outside write sentinel").unwrap();
        fs::write(selected.join("writable.txt"), b"editable").unwrap();
        fs::write(selected.join("empty"), []).unwrap();
        let readonly = selected.join("readonly.txt");
        fs::write(&readonly, b"read-only").unwrap();
        let mut permissions = fs::metadata(&readonly).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&readonly, permissions).unwrap();

        let mut sparse = File::create_new(selected.join("sparse.bin")).unwrap();
        let mut returned = 0;
        // SAFETY: The synchronous handle is borrowed from this live File. SET_SPARSE
        // takes no buffers, and returned is writable for the duration of the call.
        unsafe {
            DeviceIoControl(
                HANDLE(sparse.as_raw_handle()),
                FSCTL_SET_SPARSE,
                None,
                0,
                None,
                0,
                Some(&mut returned),
                None,
            )
            .unwrap();
        }
        assert_ne!(
            sparse.metadata().unwrap().file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE.0,
            0
        );
        sparse
            .seek(SeekFrom::Start(HIGH_OFFSET & u64::from(u32::MAX)))
            .unwrap();
        sparse.write_all(b"below").unwrap();
        sparse.seek(SeekFrom::Start(HIGH_OFFSET)).unwrap();
        sparse.write_all(b"above").unwrap();

        Self { sandbox, selected }
    }

    pub fn assert_outside_intact(&self) {
        let outside = self.sandbox.path().join("outside");
        assert_eq!(
            fs::read(outside.join("readonly.txt")).unwrap(),
            b"outside read sentinel"
        );
        assert_eq!(
            fs::read(outside.join("writable.txt")).unwrap(),
            b"outside write sentinel"
        );
    }

    pub fn read_only_drive(&self) -> TestDrive {
        let drive = RedirectedDrive::new(DEVICE_ID, SHARE_NAME, &self.selected, true).unwrap();
        let factory = WindowsRdpdrBackendFactory::new(drive);
        let mut client = Rdpdr::new(Box::new(factory.build()), "fixture".into())
            .with_drives(Some(factory.initial_drives()));
        let mut server = RdpdrServer::new(Box::new(NoopRdpdrServerBackend));
        let mut messages = server.start().unwrap();
        while !messages.is_empty() {
            let mut next = Vec::new();
            for message in messages {
                for reply in client
                    .process(&message.encode_unframed_pdu().unwrap())
                    .unwrap()
                {
                    next.extend(
                        server
                            .process(&reply.encode_unframed_pdu().unwrap())
                            .unwrap(),
                    );
                }
            }
            messages = next;
        }
        let messages = client
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap())
            .unwrap();
        let announcement = only_pdu(&messages);
        let RdpdrPdu::ClientDeviceListAnnounce(announce) = decode(&announcement).unwrap() else {
            panic!("expected a device announcement");
        };
        let [device] = announce.device_list.as_slice() else {
            panic!("expected one drive")
        };
        assert_eq!(
            (device.device_id(), device.preferred_dos_name()),
            (DEVICE_ID, SHARE_NAME)
        );
        let ack = server.process(&announcement).unwrap();
        assert!(client.process(&only_pdu(&ack)).unwrap().is_empty());
        TestDrive { client, server }
    }

    pub fn close(self) {
        self.sandbox.close().unwrap();
    }
}

fn only_pdu(messages: &[SvcMessage]) -> Vec<u8> {
    let [message] = messages else {
        panic!("expected one RDPDR message, got {}", messages.len())
    };
    message.encode_unframed_pdu().unwrap()
}

pub struct TestDrive {
    client: Rdpdr,
    pub server: RdpdrServer,
}

impl TestDrive {
    pub fn complete(&mut self, request: Vec<SvcMessage>) -> RdpdrPdu {
        self.exchange(&only_pdu(&request))
    }

    fn exchange(&mut self, bytes: &[u8]) -> RdpdrPdu {
        let RdpdrPdu::DeviceIoRequest(header) = decode(bytes).unwrap() else {
            panic!("expected an I/O request");
        };
        let reply = only_pdu(&self.client.process(bytes).unwrap());
        let mut cursor = ReadCursor::new(&reply);
        assert_eq!(
            SharedHeader::decode(&mut cursor).unwrap().packet_id,
            PacketId::CoreDeviceIoCompletion
        );
        let response = DeviceIoResponse::decode(&mut cursor).unwrap();
        assert_eq!(response.device_id, header.device_id);
        assert_eq!(response.completion_id, header.completion_id);
        let body = RdpdrPdu::decode_io_completion(
            header.major_function,
            header.minor_function,
            None,
            None,
            response,
            &mut cursor,
        )
        .unwrap();
        assert!(cursor.is_empty());
        assert!(self.server.process(&reply).unwrap().is_empty());
        body
    }

    pub fn open_with_access(&mut self, path: &str, access: DesiredAccess) -> DeviceCreateResponse {
        let request = self
            .server
            .drive_create(
                DEVICE_ID,
                path,
                access,
                CreateDisposition::FILE_OPEN,
                CreateOptions::empty(),
            )
            .unwrap();
        // RdpdrServer 0.9.0's create convenience method sets SharedAccess::all(),
        // including reserved bits. Specify the three Windows share flags in the test peer.
        let mut bytes = only_pdu(&request);
        let mut cursor = ReadCursor::new(&bytes);
        SharedHeader::decode(&mut cursor).unwrap();
        let body_start = cursor.pos();
        let header = DeviceIoRequest::decode(&mut cursor).unwrap();
        let mut create = DeviceCreateRequest::decode(header, &mut cursor).unwrap();
        create.shared_access = SharedAccess::FILE_SHARE_READ
            | SharedAccess::FILE_SHARE_WRITE
            | SharedAccess::FILE_SHARE_DELETE;
        create
            .encode(&mut WriteCursor::new(&mut bytes[body_start..]))
            .unwrap();
        let RdpdrPdu::DeviceCreateResponse(reply) = self.exchange(&bytes) else {
            panic!("expected create completion");
        };
        reply
    }

    pub fn open(&mut self, path: &str) -> u32 {
        let reply =
            self.open_with_access(path, DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY);
        assert_eq!(reply.device_io_reply.io_status, NtStatus::SUCCESS, "{path}");
        reply.file_id
    }

    pub fn close(&mut self, file_id: u32) {
        let request = self.server.drive_close(DEVICE_ID, file_id).unwrap();
        let RdpdrPdu::DeviceCloseResponse(reply) = self.complete(request) else {
            panic!("expected close completion");
        };
        assert_eq!(reply.device_io_response.io_status, NtStatus::SUCCESS);
    }
}
