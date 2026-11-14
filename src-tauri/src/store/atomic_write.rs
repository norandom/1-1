//! Atomic file replacement via same-directory staging (task 6.2).
//!
//! [`write_atomic`] / [`write_atomic_with`] stage the new content in a
//! [`tempfile::NamedTempFile`] created next to the target and rename it over
//! the target in one step, so a reader of the target path observes either
//! the complete previous file or the complete new file — never a partial
//! write. If staging, the writer callback or the final rename fails, the
//! target keeps its previous content and the staged file is deleted
//! automatically (no `.tmp` residue).
//!
//! This is ATOMIC VISIBILITY, not power-loss durability: file data is not
//! fsync'd and directory metadata is not flushed, so a crash or power loss
//! mid-write may leave the previous file or no file at all. Callers that
//! need crash consistency must layer it explicitly (Req 6.1/10.10/10.12).
//!
//! Windows note (spike-proven on NTFS): the rename-replace is blocked —
//! cleanly, with `PermissionDenied` and the prior file intact — while
//! another handle holds the target without `FILE_SHARE_DELETE`. Store
//! owners must therefore serialize reads and writes against the same file
//! (design: one store owner and serialized transactions); tasks 6.3/6.4
//! own that policy. Not yet wired into any store: 6.3 (migration) and 6.4
//! (CRUD read models) are the first production callers.

use std::io::{self, Write};
use std::path::Path;

use tempfile::NamedTempFile;

/// Atomically replace `path` with `bytes`.
///
/// The target's directory must already exist. On success `path` holds the
/// full new content; on any failure it keeps its previous content and no
/// staged file remains.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |staged| staged.write_all(bytes))
}

/// Atomically replace `path` with content produced by `write`.
///
/// `write` fills the staged temporary file (created in the same directory
/// as `path`); it may stream serializers that are too large to buffer. If
/// `write` returns an error, or staging or the final rename fails, `path`
/// keeps its previous content and the staged file is deleted.
pub fn write_atomic_with(
    path: &Path,
    write: impl FnOnce(&mut NamedTempFile) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write target must have a parent directory",
        )
    })?;
    let mut staged = NamedTempFile::new_in(parent)?;
    write(&mut staged)?;
    staged.flush()?;
    match staged.persist(path) {
        // Persisting consumes the staged file: the rename has committed.
        Ok(_file) => Ok(()),
        // The error carries the staged file; dropping it deletes the
        // staging while the previous target stays untouched.
        Err(persist_error) => Err(persist_error.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;

    /// Windows refuses to delete or replace a read-only file
    /// (ERROR_ACCESS_DENIED): a deterministic, admin-free way to force the
    /// replacement step itself to fail.
    #[cfg(windows)]
    fn set_readonly(path: &Path, on: bool) {
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_readonly(on);
        fs::set_permissions(path, perms).unwrap();
    }

    fn dir_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn replaces_existing_target_with_full_new_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        let old = vec![b'O'; 64 * 1024];
        let new = vec![b'N'; 64 * 1024];
        fs::write(&target, &old).unwrap();

        write_atomic(&target, &new).unwrap();

        assert_eq!(fs::read(&target).unwrap(), new);
        assert_eq!(
            dir_entries(dir.path()),
            vec!["state.json".to_string()],
            "no staging residue after success"
        );
    }

    #[test]
    fn creates_target_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");

        write_atomic(&target, b"FIRST").unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"FIRST");
        assert_eq!(
            dir_entries(dir.path()),
            vec!["state.json".to_string()],
            "no staging residue after success"
        );
    }

    #[test]
    fn preserves_prior_file_when_writer_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        fs::write(&target, b"OLD").unwrap();

        let err = write_atomic_with(&target, |staged| {
            staged.write_all(b"PARTIAL")?;
            Err(io::Error::other("injected writer failure"))
        })
        .unwrap_err();

        assert_eq!(err.to_string(), "injected writer failure");
        assert_eq!(
            fs::read(&target).unwrap(),
            b"OLD",
            "prior file must stay byte-identical when the writer errors"
        );
        assert_eq!(
            dir_entries(dir.path()),
            vec!["state.json".to_string()],
            "uncommitted staging must be cleaned up"
        );
    }

    /// Injection mechanism (documented per task): the target gets the
    /// read-only attribute, so Windows refuses the replace step itself with
    /// ERROR_ACCESS_DENIED — no admin rights, no OS-global changes; the
    /// attribute is restored before the tempdir is dropped.
    #[cfg(windows)]
    #[test]
    fn preserves_prior_file_when_replacement_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        fs::write(&target, b"OLD").unwrap();
        set_readonly(&target, true);

        let err = write_atomic(&target, b"NEW").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read(&target).unwrap(),
            b"OLD",
            "prior file must be intact after a failed replace"
        );
        set_readonly(&target, false);
        assert_eq!(
            dir_entries(dir.path()),
            vec!["state.json".to_string()],
            "failed staging must be cleaned up"
        );
    }

    #[test]
    fn stages_in_target_directory() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        fs::write(&target, b"OLD").unwrap();
        let mut staged_parent: Option<PathBuf> = None;

        write_atomic_with(&target, |staged| {
            staged_parent = Some(staged.path().parent().unwrap().to_path_buf());
            staged.write_all(b"NEW")
        })
        .unwrap();

        assert_eq!(
            staged_parent.as_deref(),
            Some(dir.path()),
            "staging must land in the SAME directory as the target"
        );
        assert_eq!(fs::read(&target).unwrap(), b"NEW");
    }

    /// Windows semantics (spike run 1): a std read handle holds the target
    /// without FILE_SHARE_DELETE, so the replace is blocked — cleanly, with
    /// the complete prior content still visible through both the handle and
    /// the path. Never a partial write.
    #[cfg(windows)]
    #[test]
    fn replace_blocked_by_open_reader_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        let old = vec![b'O'; 4096];
        let new = vec![b'N'; 4096];
        fs::write(&target, &old).unwrap();
        let mut reader = fs::File::open(&target).unwrap();

        let err = write_atomic(&target, &new).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        let mut seen = Vec::new();
        reader.read_to_end(&mut seen).unwrap();
        assert_eq!(seen, old, "reader keeps the complete OLD content");
        assert_eq!(fs::read(&target).unwrap(), old);
        drop(reader);
        assert_eq!(
            dir_entries(dir.path()),
            vec!["state.json".to_string()],
            "failed staging must be cleaned up"
        );
    }

    /// Unix counterpart: rename over an open file succeeds and the existing
    /// handle keeps the old inode, so both readers see complete content.
    #[cfg(unix)]
    #[test]
    fn replace_under_open_reader_keeps_complete_old_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        let old = vec![b'O'; 4096];
        let new = vec![b'N'; 4096];
        fs::write(&target, &old).unwrap();
        let mut reader = fs::File::open(&target).unwrap();

        write_atomic(&target, &new).unwrap();

        let mut seen = Vec::new();
        reader.read_to_end(&mut seen).unwrap();
        assert_eq!(seen, old, "reader keeps the complete OLD content");
        assert_eq!(fs::read(&target).unwrap(), new);
        assert_eq!(dir_entries(dir.path()), vec!["state.json".to_string()]);
    }

    #[test]
    fn fails_when_target_is_a_directory_without_residue() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("subdir");
        fs::create_dir(&target).unwrap();

        assert!(write_atomic(&target, b"NEW").is_err());

        assert!(target.is_dir(), "prior directory entry must be intact");
        assert_eq!(
            dir_entries(dir.path()),
            vec!["subdir".to_string()],
            "failed staging must be cleaned up"
        );
    }

    /// Direct tempfile contract (spike criterion "recoverable error
    /// state"): a failed persist hands the staged file back so the caller
    /// can retry; the retry succeeds and leaks nothing.
    #[test]
    fn persist_error_retains_staged_file_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        // An existing directory blocks the first persist on every OS.
        let blocker = dir.path().join("blocker");
        fs::create_dir(&blocker).unwrap();

        let mut staged = NamedTempFile::new_in(dir.path()).unwrap();
        staged.write_all(b"NEW").unwrap();
        let persist_error = staged.persist(&blocker).unwrap_err();

        let target = dir.path().join("state.json");
        fs::write(&target, b"OLD").unwrap();
        persist_error
            .file
            .persist(&target)
            .expect("staged file survives PersistError for retry");

        assert_eq!(fs::read(&target).unwrap(), b"NEW");
        assert_eq!(
            dir_entries(dir.path()),
            vec!["blocker".to_string(), "state.json".to_string()],
            "successful retry must leave no residue"
        );
    }
}
