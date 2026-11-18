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
