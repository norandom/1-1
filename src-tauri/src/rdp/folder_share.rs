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
pub(crate) use windows_root::{BindError, BoundRoot};

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
        FILE_ID_INFO, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_TRAVERSE,
        FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx,
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
            for name in names {
                current.push(name.as_os_str());
                guards.push(guard(&current)?);
            }
            let root = guards.last().expect("at least one component");
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
        let attributes = attributes(&file).map_err(|e| BindError::Unavailable(e.to_string()))?;
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(BindError::Link(path.to_path_buf()));
        }
        if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err(BindError::NotADirectory(path.to_path_buf()));
        }
        Ok(file)
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
