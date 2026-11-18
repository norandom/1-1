#![cfg(windows)]

#[path = "support/folder_share.rs"]
mod fixture;

use std::fs;

use fixture::{FolderShareFixture, HIGH_OFFSET};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{DesiredAccess, NtStatus};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::UI::Shell::{
    FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog,
};

#[test]
fn native_folder_picker_supports_filesystem_directories() -> windows::core::Result<()> {
    let flags = FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM;
    // SAFETY: This test initializes its own thread apartment, releases the dialog before
    // uninitializing, and never shows a window or obtains a user selection.
    let options = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let result = (|| -> windows::core::Result<_> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | flags)?;
            dialog.GetOptions()
        })();
        CoUninitialize();
        result?
    };
    assert!(options.contains(flags));
    Ok(())
}

#[test]
fn selected_read_only_drive_round_trips_native_file_io() {
    let fixture = FolderShareFixture::new();
    let mut drive = fixture.read_only_drive();
    for (name, offset, length, expected) in [
        ("readonly.txt", 1, 4, b"ead-".as_slice()),
        ("writable.txt", 0, 4, b"edit"),
        ("sparse.bin", HIGH_OFFSET, 12, b"above"),
        ("sparse.bin", HIGH_OFFSET & u64::from(u32::MAX), 5, b"below"),
    ] {
        let file_id = drive.open(name);
        let request = drive
            .server
            .drive_read(fixture::DEVICE_ID, file_id, length, offset)
            .unwrap();
        let RdpdrPdu::DeviceReadResponse(reply) = drive.complete(request) else {
            panic!("expected read completion");
        };
        assert_eq!(reply.device_io_reply.io_status, NtStatus::SUCCESS);
        assert_eq!(reply.read_data, expected, "{name} at {offset}");
        drive.close(file_id);
    }

    let empty = drive.open("empty");
    let request = drive
        .server
        .drive_read(fixture::DEVICE_ID, empty, 4, 0)
        .unwrap();
    let RdpdrPdu::DeviceReadResponse(reply) = drive.complete(request) else {
        panic!("expected empty-file read completion");
    };
    assert_eq!(reply.device_io_reply.io_status, NtStatus::END_OF_FILE);
    assert!(reply.read_data.is_empty());
    drive.close(empty);

    let reply = drive.open_with_access("writable.txt", DesiredAccess::GENERIC_WRITE);
    assert_eq!(
        reply.device_io_reply.io_status,
        NtStatus::MEDIA_WRITE_PROTECTED
    );
    let file_id = drive.open("writable.txt");
    let request = drive
        .server
        .drive_write(fixture::DEVICE_ID, file_id, b"changed".to_vec(), 0)
        .unwrap();
    let RdpdrPdu::DeviceWriteResponse(reply) = drive.complete(request) else {
        panic!("expected write completion");
    };
    assert_eq!(
        reply.device_io_reply.io_status,
        NtStatus::MEDIA_WRITE_PROTECTED
    );
    assert_eq!(reply.length, 0);
    drive.close(file_id);
    assert_eq!(
        fs::read(fixture.selected.join("writable.txt")).unwrap(),
        b"editable"
    );
    fixture.assert_outside_intact();
    drop(drive);
    fixture.close();
}
