//! Task 6.1 (Req 10.1/10.5/10.8): v2 library document model and tree
//! invariants for the Royal TS-style connection tree; task 6.3 adds v1→v2
//! migration and write-blocked recovery; task 6.4 (Req 10.1/10.2/10.3/
//! 10.5) adds the CRUD/read facade over the single-owner [`LibraryStore`]
//! behind the IPC commands in `commands::library`.
//!
//! Pure data + validation + query layer: flat entities with parent IDs
//! (library → folders → connections), no graph crate, database or ORM.
//! Persistence uses the task 6.2 atomic writer; every mutation is one
//! serialized transaction (snapshot → change → validate → persist →
//! publish under a single lock), so a failed mutation leaves the prior
//! snapshot and file untouched. `move_node` transactions are 6.5 — they
//! build on [`LibraryDocument::validate`] so a snapshot is proven legal
//! before it is persisted or published.
//!
//! Credentials never live in this model: keyring entries are keyed by
//! connection id only (design "Data Models"), so no secret material here.

use crate::store::atomic_write::write_atomic;
use crate::store::connections::ConnectionConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

/// Discriminator of the v2 `connections.json` document (design "Logical
/// Data Model"): a bare array is v1; an object with `"version": 2` is v2.
pub const DOCUMENT_VERSION: u32 = 2;

/// Top-level tree node: a library owns folders and connections (Req 10.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Library {
    /// Create a library with a fresh UUID v4 id and current timestamps.
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            created_at: now,
            updated_at: now,
        }
    }
}

/// Structural entity inside exactly one library; nesting via `parent_id`,
/// never through nested objects (flat entities with parent IDs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub library_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub name: String,
}

impl Folder {
    /// Create a folder with a fresh UUID v4 id inside `library_id`, either
    /// at the library root (`parent_id == None`) or under another folder.
    pub fn new(
        library_id: impl Into<String>,
        parent_id: Option<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            library_id: library_id.into(),
            parent_id,
            name: name.into(),
        }
    }
}

/// Dashboard-facing projection of a connection (Req 10.5). Live status is
/// joined from the session store at read time and never persisted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSummary {
    pub id: String,
    pub name: String,
    pub host: String,
    pub library_id: String,
    pub folder_id: Option<String>,
}

/// Read model of [`LibraryStore::list_tree`] (design "Library Commands"):
/// the whole tree as flat entity lists plus dashboard connection
/// summaries, ready for the sidebar tree and folder dashboards (10.1/10.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryTree {
    pub libraries: Vec<Library>,
    pub folders: Vec<Folder>,
    pub connections: Vec<ConnectionSummary>,
}

/// Kinds of addressable tree nodes (mirrors the TS `NodeId` union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Library,
    Folder,
    Connection,
}

/// Discriminated tree-node reference used by tree queries (`descendants`)
/// and, later, `move_node` transactions (task 6.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeId {
    pub kind: NodeKind,
    pub id: String,
}

impl NodeId {
    pub fn library(id: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Library,
            id: id.into(),
        }
    }

    pub fn folder(id: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Folder,
            id: id.into(),
        }
    }

    pub fn connection(id: impl Into<String>) -> Self {
        Self {
            kind: NodeKind::Connection,
            id: id.into(),
        }
    }
}

/// Typed tree-integrity violations (design "Error Handling"): rejected by
/// validation — and later by `move_node` — before anything is persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryError {
    /// Document version is not [`DOCUMENT_VERSION`] (unsupported schema).
    UnsupportedVersion { found: u32 },
    /// Entity id is not a well-formed UUID v4.
    InvalidId { kind: &'static str, id: String },
    /// Two entities of the same kind share an id.
    DuplicateId { kind: &'static str, id: String },
    /// A folder or connection references a library that does not exist.
    UnknownLibrary { id: String },
    /// A folder parent or a connection's folder does not exist.
    UnknownFolder { id: String },
    /// A query addressed a node that does not exist.
    UnknownNode { node: NodeId },
    /// A folder's parent lives in a different library.
    CrossLibraryParent {
        folder_id: String,
        parent_id: String,
    },
    /// A folder parent chain revisits a folder (self-parenting or indirect).
    FolderCycle { folder_id: String },
    /// A connection's `library_id` disagrees with its folder's library.
    ConnectionLibraryMismatch {
        connection_id: String,
        folder_id: String,
    },
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion { found } => write!(
                f,
                "unsupported library document version {found} (expected {DOCUMENT_VERSION})"
            ),
            Self::InvalidId { kind, id } => write!(f, "{kind} id is not a UUID v4: {id}"),
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} id: {id}"),
            Self::UnknownLibrary { id } => write!(f, "unknown library id: {id}"),
            Self::UnknownFolder { id } => write!(f, "unknown folder id: {id}"),
            Self::UnknownNode { node } => write!(f, "unknown tree node: {node:?}"),
            Self::CrossLibraryParent {
                folder_id,
                parent_id,
            } => write!(
                f,
                "folder {folder_id} parents into folder {parent_id} of another library"
            ),
            Self::FolderCycle { folder_id } => {
                write!(f, "folder parent chain is cyclic at {folder_id}")
            }
            Self::ConnectionLibraryMismatch {
                connection_id,
                folder_id,
            } => write!(
                f,
                "connection {connection_id} claims a different library than its folder {folder_id}"
            ),
        }
    }
}

impl std::error::Error for LibraryError {}

/// Typed failures of the task 6.4 CRUD/read facade on [`LibraryStore`].
/// Hand-rolled (`Display` + `std::error::Error`) per the recorded
/// store-layer error convention (tasks.md Implementation Notes, 6.1/6.3
/// decision: thiserror stays unused, error types stay hand-rolled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryStoreError {
    /// The store is in write-blocked recovery (Req 10.12): every mutation
    /// and snapshot read is rejected until a successful reload.
    WriteBlocked { reason: String },
    /// A create/rename input is not a usable name (blank after trim).
    InvalidName { kind: &'static str },
    /// The operation violates tree invariants or references an unknown
    /// entity/node (wraps [`LibraryError`]).
    Invalid(LibraryError),
    /// Validation passed but persisting the new snapshot failed; the
    /// previous in-memory snapshot and on-disk file are retained.
    PersistFailed { cause: String },
}

impl std::fmt::Display for LibraryStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WriteBlocked { reason } => write!(
                f,
                "store is write-blocked until valid data is reloaded: {reason}"
            ),
            Self::InvalidName { kind } => write!(f, "{kind} name cannot be empty"),
            Self::Invalid(err) => write!(f, "library operation rejected: {err}"),
            Self::PersistFailed { cause } => write!(
                f,
                "library operation failed to persist (prior snapshot retained): {cause}"
            ),
        }
    }
}

impl std::error::Error for LibraryStoreError {}

impl From<LibraryError> for LibraryStoreError {
    fn from(err: LibraryError) -> Self {
        Self::Invalid(err)
    }
}

/// In-memory shape of `connections.json` v2 (design "Logical Data Model"):
/// flat entity lists where nesting and ownership are parent-id references.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryDocument {
    pub version: u32,
    pub libraries: Vec<Library>,
    pub folders: Vec<Folder>,
    pub connections: Vec<ConnectionConfig>,
}

impl LibraryDocument {
    /// A valid, empty v2 document.
    pub fn empty() -> Self {
        Self::new(Vec::new(), Vec::new(), Vec::new())
    }

    /// Build a v2 document from flat entity lists. `version` is pinned to
    /// [`DOCUMENT_VERSION`]; call [`LibraryDocument::validate`] before
    /// persisting or publishing any snapshot.
    pub fn new(
        libraries: Vec<Library>,
        folders: Vec<Folder>,
        connections: Vec<ConnectionConfig>,
    ) -> Self {
        Self {
            version: DOCUMENT_VERSION,
            libraries,
            folders,
            connections,
        }
    }

    /// Check every tree invariant (design "LibraryStore v2" / "Data Models"):
    ///
    /// * `version` is exactly 2 (unsupported schema fails closed);
    /// * every id is a UUID v4, unique within its entity kind;
    /// * folders reference an existing library, their parent exists and
    ///   stays in the same library, and parent chains are acyclic;
    /// * connections reference an existing library and, when filed in a
    ///   folder, that folder belongs to the same library (single-library
    ///   ancestry).
    pub fn validate(&self) -> Result<(), LibraryError> {
        if self.version != DOCUMENT_VERSION {
            return Err(LibraryError::UnsupportedVersion {
                found: self.version,
            });
        }

        let mut library_ids: HashSet<&str> = HashSet::new();
        for library in &self.libraries {
            check_entity_id("library", &library.id, &mut library_ids)?;
        }

        let mut folder_ids: HashSet<&str> = HashSet::new();
        for folder in &self.folders {
            check_entity_id("folder", &folder.id, &mut folder_ids)?;
        }

        let mut connection_ids: HashSet<&str> = HashSet::new();
        for connection in &self.connections {
            check_entity_id("connection", &connection.id, &mut connection_ids)?;
        }

        let folder_index: HashMap<&str, &Folder> =
            self.folders.iter().map(|f| (f.id.as_str(), f)).collect();

        for folder in &self.folders {
            if !library_ids.contains(folder.library_id.as_str()) {
                return Err(LibraryError::UnknownLibrary {
                    id: folder.library_id.clone(),
                });
            }
            if let Some(parent_id) = folder.parent_id.as_deref() {
                let parent =
                    folder_index
                        .get(parent_id)
                        .ok_or_else(|| LibraryError::UnknownFolder {
                            id: parent_id.to_string(),
                        })?;
                if parent.library_id != folder.library_id {
                    return Err(LibraryError::CrossLibraryParent {
                        folder_id: folder.id.clone(),
                        parent_id: parent_id.to_string(),
                    });
                }
            }
            // Ancestor walk with a visited set: catches self-parenting and
            // indirect cycles without recursion, so deep trees stay safe.
            let mut visited: HashSet<&str> = HashSet::from([folder.id.as_str()]);
            let mut cursor = folder.parent_id.as_deref();
            while let Some(ancestor_id) = cursor {
                if !visited.insert(ancestor_id) {
                    return Err(LibraryError::FolderCycle {
                        folder_id: folder.id.clone(),
                    });
                }
                let ancestor =
                    folder_index
                        .get(ancestor_id)
                        .ok_or_else(|| LibraryError::UnknownFolder {
                            id: ancestor_id.to_string(),
                        })?;
                cursor = ancestor.parent_id.as_deref();
            }
        }

        for connection in &self.connections {
            if !library_ids.contains(connection.library_id.as_str()) {
                return Err(LibraryError::UnknownLibrary {
                    id: connection.library_id.clone(),
                });
            }
            if let Some(folder_id) = connection.folder_id.as_deref() {
                let folder =
                    folder_index
                        .get(folder_id)
                        .ok_or_else(|| LibraryError::UnknownFolder {
                            id: folder_id.to_string(),
                        })?;
                if folder.library_id != connection.library_id {
                    return Err(LibraryError::ConnectionLibraryMismatch {
                        connection_id: connection.id.clone(),
                        folder_id: folder_id.to_string(),
                    });
                }
            }
        }

        Ok(())
    }

    /// Dashboard listing (Req 10.5): every connection under `node` — a
    /// library yields its whole tree including nested folders, a folder
    /// yields its subtree, a connection yields itself. Unknown nodes are a
    /// typed error rather than a silently empty list.
    pub fn descendants(&self, node: NodeId) -> Result<Vec<ConnectionSummary>, LibraryError> {
        let summaries = match node.kind {
            NodeKind::Library => {
                if !self.libraries.iter().any(|library| library.id == node.id) {
                    return Err(LibraryError::UnknownNode { node });
                }
                self.connections
                    .iter()
                    .filter(|connection| connection.library_id == node.id)
                    .map(summary_of)
                    .collect::<Vec<_>>()
            }
            NodeKind::Folder => {
                if !self.folders.iter().any(|folder| folder.id == node.id) {
                    return Err(LibraryError::UnknownNode { node });
                }
                // Children index + breadth-first traversal: the set of
                // folders whose contents belong to this node's subtree.
                // Iterative on purpose — deep nesting cannot overflow.
                let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
                for folder in &self.folders {
                    if let Some(parent) = folder.parent_id.as_deref() {
                        children.entry(parent).or_default().push(folder.id.as_str());
                    }
                }
                let mut subtree: HashSet<&str> = HashSet::from([node.id.as_str()]);
                let mut queue: VecDeque<&str> = VecDeque::from([node.id.as_str()]);
                while let Some(current) = queue.pop_front() {
                    for child in children.get(current).map(Vec::as_slice).unwrap_or_default() {
                        if subtree.insert(child) {
                            queue.push_back(child);
                        }
                    }
                }
                self.connections
                    .iter()
                    .filter(|connection| {
                        connection
                            .folder_id
                            .as_deref()
                            .is_some_and(|folder_id| subtree.contains(folder_id))
                    })
                    .map(summary_of)
                    .collect::<Vec<_>>()
            }
            NodeKind::Connection => {
                let Some(connection) = self.connections.iter().find(|c| c.id == node.id) else {
                    return Err(LibraryError::UnknownNode { node });
                };
                vec![summary_of(connection)]
            }
        };
        Ok(summaries)
    }
}

/// Project a persisted connection into its dashboard summary.
fn summary_of(connection: &ConnectionConfig) -> ConnectionSummary {
    ConnectionSummary {
        id: connection.id.clone(),
        name: connection.name.clone(),
        host: connection.host.clone(),
        library_id: connection.library_id.clone(),
        folder_id: connection.folder_id.clone(),
    }
}

/// Enforce the id invariant for one entity kind: UUID v4 shape and
/// uniqueness within the kind (design "LibraryStore v2").
fn check_entity_id<'a>(
    kind: &'static str,
    id: &'a str,
    seen: &mut HashSet<&'a str>,
) -> Result<(), LibraryError> {
    match Uuid::parse_str(id).map(|uuid| uuid.get_version_num()) {
        Ok(4) => {}
        _ => {
            return Err(LibraryError::InvalidId {
                kind,
                id: id.to_string(),
            });
        }
    }
    if !seen.insert(id) {
        return Err(LibraryError::DuplicateId {
            kind,
            id: id.to_string(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Migration and write-blocked recovery (task 6.3; Req 6.1/6.2/10.10/10.12)
// ---------------------------------------------------------------------------

/// Name of the library every v1 connection migrates into (Req 10.10: "a
/// default library", all connections placed at its root).
pub const DEFAULT_LIBRARY_NAME: &str = "Default Library";

/// Backup suffix from the migration flow (design "Data migration"):
/// `connections.json` is backed up as `connections.json.v1.bak`.
pub const V1_BACKUP_SUFFIX: &str = ".v1.bak";

/// Human-facing recovery instructions for the write-blocked state
/// (Req 10.12); surfaced in the UI by task 8.7 and kept here so every
/// consumer shows the same text.
pub const RECOVERY_INSTRUCTIONS: &str = "The connection library could not be \
    loaded. Keep connections.json and its .v1.bak backup untouched, restore \
    valid data (for example copy the backup over connections.json), then \
    reload the library. Changes stay blocked until valid data is reloaded.";

/// Typed failure of loading/migrating `connections.json` or of a mutation
/// attempted while the store is write-blocked (Req 6.1/6.2/10.10/10.12).
///
/// Error-type convention (tasks.md "Implementation Notes", decision due at
/// 6.3): `LibraryError` is hand-rolled (`Display` + `std::error::Error`)
/// while thiserror sits unused in Cargo.toml; 6.3 keeps the hand-rolled
/// style for consistency inside the store layer rather than mixing both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationError {
    /// The store file exists but cannot be read (permissions, IO).
    ReadFailed { cause: String },
    /// The file is not valid JSON, or not valid data of its claimed shape
    /// (v1 array entries / v2 document fields).
    MalformedData { cause: String },
    /// The JSON shape is recognizable but the schema version is
    /// unsupported — fail closed, never coerce unknown versions.
    UnsupportedSchema { found: Option<u32> },
    /// Parsed fine but violates tree invariants (wraps [`LibraryError`]).
    InvalidData(LibraryError),
    /// The pre-migration backup could not be created; migration is
    /// blocked before the v2 commit (design: "backup failure blocks
    /// migration").
    BackupCreateFailed { cause: String },
    /// A backup already exists (never clobbered) but its content is not
    /// valid v1/v2 data; fail closed instead of proceeding.
    BackupExistsInvalid { cause: String },
    /// A backup already exists but cannot be examined (read error).
    BackupInspectFailed { cause: String },
    /// Internal count/id parity assertion between legacy input and
    /// migrated output failed (design: "assert count and id parity").
    ParityCheckFailed { detail: String },
    /// The validated document could not be serialized for commit.
    SerializeFailed { cause: String },
    /// The atomic v2 commit failed; the prior file and backup are intact
    /// (task 6.2 writer semantics).
    CommitFailed { cause: String },
    /// A mutation was attempted while the store is write-blocked
    /// (recovery); includes the empty-replacement case (Req 10.12: a
    /// failed load must never lead to a successful empty-store save).
    WriteBlocked { reason: String },
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadFailed { cause } => write!(f, "cannot read the library file: {cause}"),
            Self::MalformedData { cause } => write!(f, "library file is malformed: {cause}"),
            Self::UnsupportedSchema { found } => match found {
                Some(found) => write!(
                    f,
                    "unsupported library schema version {found} (expected {DOCUMENT_VERSION})"
                ),
                None => write!(
                    f,
                    "unsupported library schema (no version discriminator; \
                     expected a v1 array or version {DOCUMENT_VERSION})"
                ),
            },
            Self::InvalidData(err) => {
                write!(f, "library data violates tree invariants: {err}")
            }
            Self::BackupCreateFailed { cause } => {
                write!(f, "cannot create the migration backup: {cause}")
            }
            Self::BackupExistsInvalid { cause } => write!(
                f,
                "existing backup is not valid data, refusing to overwrite it: {cause}"
            ),
            Self::BackupInspectFailed { cause } => {
                write!(f, "cannot examine the existing backup: {cause}")
            }
            Self::ParityCheckFailed { detail } => {
                write!(f, "migration parity check failed: {detail}")
            }
            Self::SerializeFailed { cause } => {
                write!(f, "cannot serialize the library document: {cause}")
            }
            Self::CommitFailed { cause } => write!(f, "atomic v2 commit failed: {cause}"),
            Self::WriteBlocked { reason } => write!(
                f,
                "store is write-blocked until valid data is reloaded: {reason}"
            ),
        }
    }
}

impl std::error::Error for MigrationError {}

/// Result of a successful [`load_or_migrate`] / [`LibraryStore::reload`].
#[derive(Debug, Clone)]
pub enum LoadOutcome {
    /// No store file on disk: a fresh store starts empty (nothing written).
    Fresh,
    /// The file already held a valid v2 document (used as-is, no backup).
    LoadedV2(LibraryDocument),
    /// A v1 bare array was migrated: the original bytes are preserved at
    /// `backup_path` and a v2 document was atomically committed.
    MigratedV2 {
        document: LibraryDocument,
        backup_path: PathBuf,
    },
}

impl LoadOutcome {
    /// The document this outcome leaves the store in.
    pub fn document(&self) -> LibraryDocument {
        match self {
            Self::Fresh => LibraryDocument::empty(),
            Self::LoadedV2(document) => document.clone(),
            Self::MigratedV2 { document, .. } => document.clone(),
        }
    }
}

/// Wrap a parsed v1 bare-array connection list into a v2 document (design
/// "Data migration"; Req 10.10): one fresh default library, every legacy
/// connection preserved field-for-field and placed at the library root
/// (`folderId: null`). Only `libraryId`/`folderId` are assigned — the
/// documented migration defaults; ids and all other fields stay untouched,
/// and credential accounts (keyring, keyed by connection id) are unchanged
/// by construction.
///
/// Task 6.4 reconciliation (tasks.md 6.1 Implementation Note): design L348
/// sketches this as a method on the store facade
/// (`LibraryStore::migrate_v1`). The free function stays deliberately: it
/// is the pure data transform applied inside the load path
/// ([`load_or_migrate`] → [`LibraryStore::open`]/[`LibraryStore::reload`])
/// and never reads or writes store instance state, which a `&self`/`&mut
/// self` method would falsely imply. The store methods below own the
/// migration flow end-to-end; there is no separate facade shim.
pub fn migrate_v1(legacy: Vec<ConnectionConfig>) -> LibraryDocument {
    let library = Library::new(DEFAULT_LIBRARY_NAME);
    let connections = legacy
        .into_iter()
        .map(|mut connection| {
            connection.library_id = library.id.clone();
            connection.folder_id = None;
            connection
        })
        .collect();
    LibraryDocument::new(vec![library], Vec::new(), connections)
}

/// Load `path`, migrating a v1 bare array to v2 (design "Data migration"):
/// detect → wrap → backup → validate → atomic v2 commit. A v2 document is
/// used as-is. Every failure mode fails closed: the original file (and any
/// existing backup) is preserved untouched and the caller must enter
/// write-blocked recovery (see [`LibraryStore`]).
pub fn load_or_migrate(path: &Path) -> Result<LoadOutcome, MigrationError> {
    let original = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(LoadOutcome::Fresh),
        Err(e) => {
            return Err(MigrationError::ReadFailed {
                cause: e.to_string(),
            });
        }
    };
    let root: serde_json::Value =
        serde_json::from_slice(&original).map_err(|e| MigrationError::MalformedData {
            cause: e.to_string(),
        })?;
    // A numeric "version" discriminates v2 from v1 (design "Logical Data
    // Model"); unrecognized shapes fail closed below.
    let found = root
        .get("version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok());
    match root {
        serde_json::Value::Array(_) => migrate_v1_bytes(path, &original, root),
        serde_json::Value::Object(_) => match found {
            Some(DOCUMENT_VERSION) => {
                let document: LibraryDocument =
                    serde_json::from_value(root).map_err(|e| MigrationError::MalformedData {
                        cause: e.to_string(),
                    })?;
                document.validate().map_err(MigrationError::InvalidData)?;
                Ok(LoadOutcome::LoadedV2(document))
            }
            other => Err(MigrationError::UnsupportedSchema { found: other }),
        },
        _ => Err(MigrationError::UnsupportedSchema { found: None }),
    }
}

/// v1 branch of the migration flow (design "Data migration"), in flow
/// order: wrap → backup → validate → save.
fn migrate_v1_bytes(
    path: &Path,
    original: &[u8],
    root: serde_json::Value,
) -> Result<LoadOutcome, MigrationError> {
    let legacy: Vec<ConnectionConfig> =
        serde_json::from_value(root).map_err(|e| MigrationError::MalformedData {
            cause: e.to_string(),
        })?;

    // Wrap into the default library (Req 10.10).
    let migrated = migrate_v1(legacy.clone());

    // Backup the ORIGINAL bytes before anything else, without clobber: an
    // existing backup is kept when valid, and an unusable existing backup
    // fails the whole migration closed — data is never destroyed to make
    // migration proceed.
    let backup_path = backup_path_for(path);
    if backup_path.exists() {
        let existing =
            std::fs::read(&backup_path).map_err(|e| MigrationError::BackupInspectFailed {
                cause: e.to_string(),
            })?;
        validate_backup_bytes(&existing)
            .map_err(|cause| MigrationError::BackupExistsInvalid { cause })?;
    } else {
        write_atomic(&backup_path, original).map_err(|e| MigrationError::BackupCreateFailed {
            cause: e.to_string(),
        })?;
    }

    // Validate the wrapped document and assert count/id parity (design:
    // "assert count and id parity").
    migrated.validate().map_err(MigrationError::InvalidData)?;
    assert_v1_parity(&legacy, &migrated)?;

    // Atomic v2 commit; on failure the original v1 file and the backup
    // stay intact (task 6.2 writer semantics).
    let json =
        serde_json::to_string_pretty(&migrated).map_err(|e| MigrationError::SerializeFailed {
            cause: e.to_string(),
        })?;
    write_atomic(path, json.as_bytes()).map_err(|e| MigrationError::CommitFailed {
        cause: e.to_string(),
    })?;

    Ok(LoadOutcome::MigratedV2 {
        document: migrated,
        backup_path,
    })
}

/// Backup path for a store file (design "Data migration"):
/// `connections.json` → `connections.json.v1.bak`.
fn backup_path_for(path: &Path) -> PathBuf {
    match path.file_name() {
        Some(name) => {
            let mut suffixed = name.to_os_string();
            suffixed.push(V1_BACKUP_SUFFIX);
            path.with_file_name(suffixed)
        }
        None => PathBuf::from(V1_BACKUP_SUFFIX),
    }
}

/// An existing backup is usable when it parses as v1 legacy data or as an
/// invariant-valid v2 document — anything else must block migration.
fn validate_backup_bytes(bytes: &[u8]) -> Result<(), String> {
    let root: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| format!("not valid JSON: {e}"))?;
    match root {
        serde_json::Value::Array(_) => serde_json::from_value::<Vec<ConnectionConfig>>(root)
            .map(|_| ())
            .map_err(|e| format!("not a valid v1 connection array: {e}")),
        serde_json::Value::Object(_) => serde_json::from_value::<LibraryDocument>(root)
            .map_err(|e| format!("not a valid v2 library document: {e}"))
            .and_then(|document| {
                document
                    .validate()
                    .map_err(|e| format!("v2 backup violates tree invariants: {e}"))
            }),
        _ => Err("unsupported backup shape (expected v1 array or v2 object)".to_string()),
    }
}

/// Count and id parity between the legacy input and the migrated output
/// (design "Data migration": assert both before committing).
fn assert_v1_parity(
    legacy: &[ConnectionConfig],
    migrated: &LibraryDocument,
) -> Result<(), MigrationError> {
    if legacy.len() != migrated.connections.len() {
        return Err(MigrationError::ParityCheckFailed {
            detail: format!(
                "connection count changed: {} -> {}",
                legacy.len(),
                migrated.connections.len()
            ),
        });
    }
    let legacy_ids: HashSet<&str> = legacy.iter().map(|c| c.id.as_str()).collect();
    let migrated_ids: HashSet<&str> = migrated.connections.iter().map(|c| c.id.as_str()).collect();
    if legacy_ids != migrated_ids {
        return Err(MigrationError::ParityCheckFailed {
            detail: "connection id set changed during migration".to_string(),
        });
    }
    Ok(())
}

/// Current store mode for the UI (recovery state, Req 10.12). Task 6.4:
/// serialized (internally tagged, camelCase) for the `get_library_status`
/// and `reload_library_store` read commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum StoreStatus {
    /// Document loaded (v2 as-is or after v1 migration); mutations allowed.
    Ready,
    /// Load or migration failed: the original file and existing backup are
    /// preserved, every mutation is rejected until a reload succeeds.
    WriteBlocked {
        /// Display text of the blocking [`MigrationError`].
        reason: String,
        /// Recovery instructions to show until valid data is reloaded.
        instructions: &'static str,
    },
}

/// Internal store mode: a readable snapshot, or write-blocked recovery.
#[derive(Debug, Clone)]
enum StoreState {
    Ready(LibraryDocument),
    WriteBlocked { cause: MigrationError },
}

/// Single-owner library store (design "Storage transactions and recovery"):
/// one `Mutex` serializes every read/write against the store file, which
/// is the discipline the task 6.2 Windows spike requires (an open
/// unshared read handle blocks the atomic replace). The backup path is
/// derived from the store path on demand ([`backup_path_for`]).
struct StoreInner {
    path: PathBuf,
    state: StoreState,
}

/// Owner of the v2 library document (transitional `libraries.json` until
/// 6.6 unifies it with the v1-owned `connections.json`): loads (migrating
/// v1 on first sight), serves the current snapshot, and gates every
/// mutation behind the
/// write-blocked recovery state (Req 10.10/10.12).
/// On any load/migration failure the store opens in
/// [`StoreStatus::WriteBlocked`]: the on-disk file and backup are left
/// untouched, every mutation is rejected — including the empty replacement
/// that would wipe unreadable data — and only a successful
/// [`LibraryStore::reload`] lifts the block.
pub struct LibraryStore {
    inner: Mutex<StoreInner>,
}

impl LibraryStore {
    /// Open (or restart) the store at `path`, migrating a v1 file per the
    /// design flow. Never panics and never writes on failure: a failed
    /// load yields a write-blocked store.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let state = match load_or_migrate(&path) {
            Ok(outcome) => {
                match &outcome {
                    LoadOutcome::Fresh => {
                        log::info!(
                            "Library store: no file at {}, starting empty",
                            path.display()
                        );
                    }
                    LoadOutcome::LoadedV2(_) => {
                        log::info!("Library store: loaded v2 document from {}", path.display());
                    }
                    LoadOutcome::MigratedV2 { backup_path, .. } => {
                        log::info!(
                            "Library store: migrated {} to v2 (backup at {})",
                            path.display(),
                            backup_path.display()
                        );
                    }
                }
                StoreState::Ready(outcome.document())
            }
            Err(cause) => {
                log::error!(
                    "Library store: loading {} failed ({cause}); entering write-blocked \
                     recovery, original file and backup preserved",
                    path.display()
                );
                StoreState::WriteBlocked { cause }
            }
        };
        Self {
            inner: Mutex::new(StoreInner { path, state }),
        }
    }

    /// Current store mode for the UI (recovery state, Req 10.12).
    pub fn status(&self) -> StoreStatus {
        match &self
            .inner
            .lock()
            .expect("library store lock poisoned")
            .state
        {
            StoreState::Ready(_) => StoreStatus::Ready,
            StoreState::WriteBlocked { cause } => StoreStatus::WriteBlocked {
                reason: cause.to_string(),
                instructions: RECOVERY_INSTRUCTIONS,
            },
        }
    }

    /// The current document snapshot, or `None` while write-blocked.
    ///
    /// Test-facing (task 6.4): production reads go through the typed read
    /// models ([`LibraryStore::list_tree`], [`LibraryStore::descendants`],
    /// [`LibraryStore::status`]) which also express the write-blocked
    /// state; tests need the raw snapshot to seed and assert documents.
    #[cfg(test)]
    pub fn document(&self) -> Option<LibraryDocument> {
        match &self
            .inner
            .lock()
            .expect("library store lock poisoned")
            .state
        {
            StoreState::Ready(document) => Some(document.clone()),
            StoreState::WriteBlocked { .. } => None,
        }
    }

    /// Re-run the load/migration from disk. On success the store becomes
    /// ready with the (possibly migrated) document; on failure it stays
    /// write-blocked. This is the only path out of recovery (Req 10.12:
    /// "until valid data is restored and reloaded").
    pub fn reload(&self) -> Result<LoadOutcome, MigrationError> {
        let mut inner = self.inner.lock().expect("library store lock poisoned");
        match load_or_migrate(&inner.path) {
            Ok(outcome) => {
                inner.state = StoreState::Ready(outcome.document());
                Ok(outcome)
            }
            Err(cause) => {
                inner.state = StoreState::WriteBlocked {
                    cause: cause.clone(),
                };
                Err(cause)
            }
        }
    }

    /// Validate, atomically persist, then publish `document` (design
    /// "Storage transactions and recovery": persist before publishing).
    ///
    /// Rejected while write-blocked — including an empty replacement
    /// (Req 10.12: a failed load must never lead to a successful
    /// empty-store save). On commit failure the previous in-memory
    /// snapshot and on-disk file are retained.
    ///
    /// Test-facing whole-document replace (task 6.4 reconciliation):
    /// production mutations flow through the single-lock CRUD facade
    /// below, whose transactions commit through the same core
    /// ([`LibraryStore::commit_publish`] via [`LibraryStore::save_locked`]);
    /// tests use this to seed and fault-inject arbitrary documents.
    #[cfg(test)]
    pub fn save(&self, document: LibraryDocument) -> Result<(), MigrationError> {
        let mut inner = self.inner.lock().expect("library store lock poisoned");
        Self::save_locked(&mut inner, document)
    }

    /// Blocked-check + transactional commit with the store lock held by
    /// the caller; shared by [`LibraryStore::save`] (tests) and
    /// [`LibraryStore::mutate`] (production CRUD transactions).
    fn save_locked(
        inner: &mut StoreInner,
        document: LibraryDocument,
    ) -> Result<(), MigrationError> {
        if let StoreState::WriteBlocked { cause } = &inner.state {
            return Err(MigrationError::WriteBlocked {
                reason: cause.to_string(),
            });
        }
        Self::commit_publish(inner, document)
    }

    // ---- Task 6.4: CRUD and read models (Req 10.1/10.2/10.3/10.5) ------
    // Commands in `commands::library` are thin marshalers around these
    // methods; all validation and transaction handling lives here.

    /// Req 10.2: create a library (fresh UUID v4 id, current timestamps)
    /// in one serialized transaction and persist it.
    pub fn create_library(&self, name: &str) -> Result<Library, LibraryStoreError> {
        check_name("library", name)?;
        self.mutate(|mut next| {
            let library = Library::new(name.trim());
            next.libraries.push(library.clone());
            Ok((next, library))
        })
    }

    /// Req 10.2: rename a library and bump its `updated_at`. Unknown ids
    /// and blank names are rejected before anything is touched.
    pub fn rename_library(&self, id: &str, name: &str) -> Result<Library, LibraryStoreError> {
        check_name("library", name)?;
        self.mutate(|mut next| {
            let renamed = {
                let library = next
                    .libraries
                    .iter_mut()
                    .find(|library| library.id == id)
                    .ok_or_else(|| LibraryError::UnknownLibrary { id: id.to_string() })?;
                library.name = name.trim().to_string();
                library.updated_at = Utc::now();
                library.clone()
            };
            Ok((next, renamed))
        })
    }

    /// Req 10.3/10.7 folder semantics: create a folder under a library
    /// root (`parent_id == None`) or under another folder of the SAME
    /// library. Unknown libraries/parents and cross-library parents are
    /// rejected by the transaction's validation step (typed
    /// [`LibraryError`]); nothing is persisted on rejection.
    pub fn create_folder(
        &self,
        library_id: &str,
        parent_id: Option<&str>,
        name: &str,
    ) -> Result<Folder, LibraryStoreError> {
        check_name("folder", name)?;
        self.mutate(|mut next| {
            let folder = Folder::new(library_id, parent_id.map(str::to_string), name.trim());
            next.folders.push(folder.clone());
            Ok((next, folder))
        })
    }

    /// Req 10.3/10.7: rename a folder in place. Unknown ids and blank
    /// names are rejected without touching the snapshot.
    pub fn rename_folder(&self, id: &str, name: &str) -> Result<Folder, LibraryStoreError> {
        check_name("folder", name)?;
        self.mutate(|mut next| {
            let renamed = {
                let folder = next
                    .folders
                    .iter_mut()
                    .find(|folder| folder.id == id)
                    .ok_or_else(|| LibraryError::UnknownFolder { id: id.to_string() })?;
                folder.name = name.trim().to_string();
                folder.clone()
            };
            Ok((next, renamed))
        })
    }

    /// Design "Library Commands" `list_tree() -> LibraryTree`: flat
    /// libraries/folders plus dashboard connection summaries (10.1).
    /// Rejected with a typed error while write-blocked (no snapshot).
    pub fn list_tree(&self) -> Result<LibraryTree, LibraryStoreError> {
        self.read(|document| {
            Ok(LibraryTree {
                libraries: document.libraries.clone(),
                folders: document.folders.clone(),
                connections: document.connections.iter().map(summary_of).collect(),
            })
        })
    }

    /// Dashboard listing incl. nested folders (Req 10.5, design L346).
    ///
    /// RECONCILIATION (tasks.md 6.1 Implementation Note, resolved in 6.4):
    /// design shows a bare `Vec<ConnectionSummary>` on the store, but the
    /// store deliberately keeps a typed error, for two reasons:
    /// 1. an unknown node must not masquerade as an empty dashboard — a
    ///    stale tree id would silently render "no connections" instead of
    ///    surfacing the drift ([`LibraryError::UnknownNode`]);
    /// 2. a write-blocked store has no readable snapshot at all, which a
    ///    bare `Vec` cannot express ([`LibraryStoreError::WriteBlocked`]).
    ///
    /// The command layer surfaces the message to the UI.
    pub fn descendants(&self, node: NodeId) -> Result<Vec<ConnectionSummary>, LibraryStoreError> {
        self.read(|document| {
            document
                .descendants(node)
                .map_err(LibraryStoreError::Invalid)
        })
    }

    /// Run a read query against the current snapshot under the store lock.
    fn read<T>(
        &self,
        query: impl FnOnce(&LibraryDocument) -> Result<T, LibraryStoreError>,
    ) -> Result<T, LibraryStoreError> {
        let inner = self.inner.lock().expect("library store lock poisoned");
        match &inner.state {
            StoreState::Ready(document) => query(document),
            StoreState::WriteBlocked { cause } => Err(LibraryStoreError::WriteBlocked {
                reason: cause.to_string(),
            }),
        }
    }

    /// One serialized transaction (design "Storage transactions and
    /// recovery"): clone the current snapshot, apply `change`, then
    /// validate/persist/publish the result while holding the SAME lock, so
    /// concurrent commands can never interleave or lose updates. On any
    /// error — blank name, invariant violation, persist failure — the
    /// previous in-memory snapshot and on-disk file are retained.
    fn mutate<T>(
        &self,
        change: impl FnOnce(LibraryDocument) -> Result<(LibraryDocument, T), LibraryStoreError>,
    ) -> Result<T, LibraryStoreError> {
        let mut inner = self.inner.lock().expect("library store lock poisoned");
        let current = match &inner.state {
            StoreState::Ready(document) => document.clone(),
            StoreState::WriteBlocked { cause } => {
                return Err(LibraryStoreError::WriteBlocked {
                    reason: cause.to_string(),
                });
            }
        };
        let (next, value) = change(current)?;
        Self::save_locked(&mut inner, next).map_err(map_commit_error)?;
        Ok(value)
    }

    /// Validate → serialize → atomically persist → publish, with the store
    /// lock held by the caller. Shared transaction core of
    /// [`LibraryStore::save`] and every task 6.4 CRUD mutation; publishing
    /// happens only after a successful commit, so a failed commit leaves
    /// the previous in-memory snapshot and on-disk file untouched.
    fn commit_publish(
        inner: &mut StoreInner,
        document: LibraryDocument,
    ) -> Result<(), MigrationError> {
        document.validate().map_err(MigrationError::InvalidData)?;
        let json = serde_json::to_string_pretty(&document).map_err(|e| {
            MigrationError::SerializeFailed {
                cause: e.to_string(),
            }
        })?;
        if let Some(parent) = inner.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| MigrationError::CommitFailed {
                cause: format!("cannot create data directory: {e}"),
            })?;
        }
        write_atomic(&inner.path, json.as_bytes()).map_err(|e| MigrationError::CommitFailed {
            cause: e.to_string(),
        })?;
        // Publish only after the commit succeeded.
        inner.state = StoreState::Ready(document);
        Ok(())
    }
}

/// Store-layer input validation for entity names, so the IPC commands stay
/// thin marshalers (task 6.4): a name must be non-blank after trimming.
fn check_name(kind: &'static str, name: &str) -> Result<(), LibraryStoreError> {
    if name.trim().is_empty() {
        return Err(LibraryStoreError::InvalidName { kind });
    }
    Ok(())
}

/// Fold a commit-path [`MigrationError`] into the CRUD error surface.
fn map_commit_error(error: MigrationError) -> LibraryStoreError {
    match error {
        MigrationError::WriteBlocked { reason } => LibraryStoreError::WriteBlocked { reason },
        MigrationError::InvalidData(err) => LibraryStoreError::Invalid(err),
        MigrationError::SerializeFailed { cause } | MigrationError::CommitFailed { cause } => {
            LibraryStoreError::PersistFailed { cause }
        }
        // commit_publish produces no other variants; keep the mapping
        // total anyway so a future variant cannot panic.
        other => LibraryStoreError::PersistFailed {
            cause: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    //! Task 6.1 (Req 10.1/10.5/10.8): v2 library document model and tree
    //! invariants. These tests are the specification for the pure data layer:
    //! flat entities with parent IDs, UUID v4 uniqueness per entity kind,
    //! single-library ancestry, acyclic folder graphs and descendant queries
    //! that reach nested folders. Persistence (6.3/6.4), CRUD commands (6.4)
    //! and moves (6.5) build on this model.

    use super::*;
    use crate::store::connections::ConnectionConfig;
    use uuid::Uuid;

    fn ts() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    fn conn(library_id: &str, folder_id: Option<&str>, name: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            host: format!("{name}.invalid"),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            mock: false,
            library_id: library_id.to_string(),
            folder_id: folder_id.map(str::to_string),
            created_at: ts(),
            updated_at: ts(),
            last_connected_at: None,
        }
    }

    fn sorted_names(summaries: Vec<ConnectionSummary>) -> String {
        let mut names: Vec<String> = summaries.into_iter().map(|s| s.name).collect();
        names.sort();
        names.join(",")
    }

    // ---- v2 document shape -------------------------------------------------

    #[test]
    fn empty_document_is_valid() {
        let doc = LibraryDocument::empty();
        assert_eq!(doc.validate(), Ok(()));
    }

    #[test]
    fn v2_document_serializes_with_camel_case_v2_shape() {
        let lib = Library::new("Work");
        let root = Folder::new(&lib.id, None, "Prod");
        let nested = Folder::new(&lib.id, Some(root.id.clone()), "Europe");
        let c1 = conn(&lib.id, None, "root-conn");
        let c2 = conn(&lib.id, Some(&nested.id), "nested-conn");
        let doc = LibraryDocument::new(vec![lib], vec![root, nested], vec![c1, c2]);

        assert_eq!(doc.validate(), Ok(()));

        let json = serde_json::to_string_pretty(&doc).unwrap();
        assert!(json.contains(r#""version": 2"#), "v2 discriminator: {json}");
        assert!(
            json.contains(r#""libraryId""#),
            "camelCase folder.libraryId: {json}"
        );
        assert!(
            json.contains(r#""parentId""#),
            "camelCase folder.parentId: {json}"
        );
        assert!(
            json.contains(r#""folderId""#),
            "camelCase connection.folderId: {json}"
        );
        assert!(
            json.contains(r#""createdAt""#),
            "camelCase library.createdAt: {json}"
        );

        let reparsed: LibraryDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_string_pretty(&reparsed).unwrap(),
            json,
            "v2 document round-trips losslessly"
        );
    }

    #[test]
    fn v1_bare_array_does_not_parse_as_v2_document() {
        let v1 = r#"[{"id":"legacy","name":"Legacy","host":"legacy.invalid","port":3389,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]"#;
        assert!(
            serde_json::from_str::<LibraryDocument>(v1).is_err(),
            "a v1 bare array is not a v2 document object"
        );
    }

    #[test]
    fn unsupported_document_version_is_rejected() {
        let doc = LibraryDocument {
            version: 3,
            ..LibraryDocument::empty()
        };
        assert_eq!(
            doc.validate(),
            Err(LibraryError::UnsupportedVersion { found: 3 })
        );
    }

    // ---- id invariants: UUID v4, unique per entity kind --------------------

    #[test]
    fn duplicate_ids_are_rejected_per_entity_kind() {
        let a = Library::new("A");
        let b = Library::new("B");
        let mut doc = LibraryDocument::new(vec![a.clone(), b], vec![], vec![]);
        doc.libraries[1].id = a.id.clone();
        assert_eq!(
            doc.validate(),
            Err(LibraryError::DuplicateId {
                kind: "library",
                id: a.id.clone()
            })
        );

        let lib = Library::new("L");
        let f1 = Folder::new(&lib.id, None, "F1");
        let f2 = Folder::new(&lib.id, None, "F2");
        let mut doc = LibraryDocument::new(vec![lib.clone()], vec![f1.clone(), f2], vec![]);
        doc.folders[1].id = f1.id.clone();
        assert_eq!(
            doc.validate(),
            Err(LibraryError::DuplicateId {
                kind: "folder",
                id: f1.id.clone()
            })
        );

        let c1 = conn(&lib.id, None, "C1");
        let c2 = conn(&lib.id, None, "C2");
        let mut doc = LibraryDocument::new(vec![lib], vec![], vec![c1.clone(), c2]);
        doc.connections[1].id = c1.id.clone();
        assert_eq!(
            doc.validate(),
            Err(LibraryError::DuplicateId {
                kind: "connection",
                id: c1.id.clone()
            })
        );
    }

    #[test]
    fn non_uuid_and_non_v4_entity_ids_are_rejected() {
        let lib = Library {
            id: "not-a-uuid".to_string(),
            ..Library::new("L")
        };
        let doc = LibraryDocument::new(vec![lib.clone()], vec![], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::InvalidId {
                kind: "library",
                id: lib.id.clone()
            })
        );

        // Well-formed UUID, wrong version (v1): still invalid.
        let lib = Library {
            id: "00000000-0000-1000-8000-000000000000".to_string(),
            ..Library::new("L")
        };
        let doc = LibraryDocument::new(vec![lib.clone()], vec![], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::InvalidId {
                kind: "library",
                id: lib.id.clone()
            })
        );
    }

    // ---- folder invariants: ownership + acyclicity -------------------------

    #[test]
    fn folder_with_missing_parent_is_rejected() {
        let lib = Library::new("L");
        let missing = Uuid::new_v4().to_string();
        let orphan = Folder::new(&lib.id, Some(missing.clone()), "Orphan");
        let doc = LibraryDocument::new(vec![lib], vec![orphan], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::UnknownFolder { id: missing })
        );
    }

    #[test]
    fn folder_referencing_unknown_library_is_rejected() {
        let present = Library::new("Present");
        let absent = Library::new("Absent");
        let misplaced = Folder::new(&absent.id, None, "Misplaced");
        let doc = LibraryDocument::new(vec![present], vec![misplaced], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::UnknownLibrary { id: absent.id })
        );
    }

    #[test]
    fn folder_parent_in_a_different_library_is_rejected() {
        let a = Library::new("A");
        let b = Library::new("B");
        let parent = Folder::new(&a.id, None, "Parent");
        let child = Folder::new(&b.id, Some(parent.id.clone()), "Child");
        let doc = LibraryDocument::new(vec![a, b], vec![parent.clone(), child.clone()], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::CrossLibraryParent {
                folder_id: child.id.clone(),
                parent_id: parent.id.clone()
            })
        );
    }

    #[test]
    fn self_parent_cycle_is_rejected() {
        let lib = Library::new("L");
        let mut f = Folder::new(&lib.id, None, "Self");
        f.parent_id = Some(f.id.clone());
        let doc = LibraryDocument::new(vec![lib], vec![f.clone()], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::FolderCycle { folder_id: f.id })
        );
    }

    #[test]
    fn indirect_parent_cycles_are_rejected() {
        let lib = Library::new("L");

        // Two-node cycle: A -> B -> A.
        let mut a = Folder::new(&lib.id, None, "A");
        let mut b = Folder::new(&lib.id, None, "B");
        a.parent_id = Some(b.id.clone());
        b.parent_id = Some(a.id.clone());
        let doc = LibraryDocument::new(vec![lib.clone()], vec![a.clone(), b], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::FolderCycle { folder_id: a.id })
        );

        // Three-node cycle: C -> D -> E -> C (document order shuffled so the
        // check cannot rely on order). The error names the first folder in
        // document order whose ancestor walk revisits a node.
        let mut c = Folder::new(&lib.id, None, "C");
        let mut d = Folder::new(&lib.id, None, "D");
        let mut e = Folder::new(&lib.id, None, "E");
        c.parent_id = Some(d.id.clone());
        d.parent_id = Some(e.id.clone());
        e.parent_id = Some(c.id.clone());
        let doc = LibraryDocument::new(vec![lib], vec![e.clone(), c, d], vec![]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::FolderCycle { folder_id: e.id })
        );
    }

    // ---- connection ownership invariants -----------------------------------

    #[test]
    fn connection_referencing_unknown_library_is_rejected() {
        let lib = Library::new("L");
        let absent = Library::new("Absent");
        let c = conn(&absent.id, None, "C");
        let doc = LibraryDocument::new(vec![lib], vec![], vec![c]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::UnknownLibrary { id: absent.id })
        );
    }

    #[test]
    fn connection_referencing_unknown_folder_is_rejected() {
        let lib = Library::new("L");
        let missing = Uuid::new_v4().to_string();
        let c = conn(&lib.id, Some(&missing), "C");
        let doc = LibraryDocument::new(vec![lib], vec![], vec![c]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::UnknownFolder { id: missing })
        );
    }

    #[test]
    fn connection_folder_in_a_different_library_is_rejected() {
        let a = Library::new("A");
        let b = Library::new("B");
        let f = Folder::new(&a.id, None, "FolderInA");
        let c = conn(&b.id, Some(&f.id), "ConnInB");
        let doc = LibraryDocument::new(vec![a, b], vec![f.clone()], vec![c.clone()]);
        assert_eq!(
            doc.validate(),
            Err(LibraryError::ConnectionLibraryMismatch {
                connection_id: c.id.clone(),
                folder_id: f.id.clone()
            })
        );
    }

    // ---- descendant queries (10.5) -----------------------------------------

    #[test]
    fn deep_nesting_validates_and_descendant_queries_reach_nested_connections() {
        const DEPTH: usize = 50;
        let lib = Library::new("Deep");

        let mut folders = Vec::with_capacity(DEPTH);
        let mut parent: Option<String> = None;
        for depth in 0..DEPTH {
            let f = Folder::new(&lib.id, parent.clone(), format!("depth-{depth}"));
            parent = Some(f.id.clone());
            folders.push(f);
        }
        let root_folder = folders[0].id.clone();
        let mid_folder = folders[DEPTH / 2].id.clone();
        let deepest_folder = folders[DEPTH - 1].id.clone();

        let at_root = conn(&lib.id, None, "at-library-root");
        let under_root = conn(&lib.id, Some(&root_folder), "under-root");
        let under_mid = conn(&lib.id, Some(&mid_folder), "under-mid");
        let under_deepest = conn(&lib.id, Some(&deepest_folder), "under-deepest");
        let doc = LibraryDocument::new(
            vec![lib.clone()],
            folders,
            vec![at_root, under_root, under_mid, under_deepest],
        );

        // A 50-level chain is valid: acyclic, single-library, unique v4 ids.
        assert_eq!(doc.validate(), Ok(()));

        // Library dashboard: everything in the library, nested included (10.5).
        assert_eq!(
            sorted_names(doc.descendants(NodeId::library(&lib.id)).unwrap()),
            "at-library-root,under-deepest,under-mid,under-root"
        );
        // Root folder dashboard: everything at depth >= 1.
        assert_eq!(
            sorted_names(doc.descendants(NodeId::folder(&root_folder)).unwrap()),
            "under-deepest,under-mid,under-root"
        );
        // Mid folder dashboard: only its own subtree.
        assert_eq!(
            sorted_names(doc.descendants(NodeId::folder(&mid_folder)).unwrap()),
            "under-deepest,under-mid"
        );
        // Deepest folder dashboard: the leaf only.
        assert_eq!(
            sorted_names(doc.descendants(NodeId::folder(&deepest_folder)).unwrap()),
            "under-deepest"
        );
    }

    #[test]
    fn descendants_are_scoped_to_the_requested_library_tree() {
        let a = Library::new("A");
        let b = Library::new("B");
        let f_a = Folder::new(&a.id, None, "InA");
        let f_b = Folder::new(&b.id, None, "InB");
        let c_a = conn(&a.id, Some(&f_a.id), "belongs-to-a");
        let c_b = conn(&b.id, Some(&f_b.id), "belongs-to-b");
        let doc = LibraryDocument::new(
            vec![a.clone(), b.clone()],
            vec![f_a.clone(), f_b],
            vec![c_a, c_b],
        );
        assert_eq!(doc.validate(), Ok(()));

        assert_eq!(
            sorted_names(doc.descendants(NodeId::library(&a.id)).unwrap()),
            "belongs-to-a"
        );
        assert_eq!(
            sorted_names(doc.descendants(NodeId::library(&b.id)).unwrap()),
            "belongs-to-b"
        );
        assert_eq!(
            sorted_names(doc.descendants(NodeId::folder(&f_a.id)).unwrap()),
            "belongs-to-a"
        );
    }

    #[test]
    fn descendants_of_a_connection_node_returns_that_connection() {
        let lib = Library::new("L");
        let c = conn(&lib.id, None, "solo");
        let doc = LibraryDocument::new(vec![lib], vec![], vec![c.clone()]);
        assert_eq!(doc.validate(), Ok(()));

        let summaries = doc.descendants(NodeId::connection(&c.id)).unwrap();
        assert_eq!(summaries.len(), 1);
        let s = &summaries[0];
        assert_eq!(s.id, c.id);
        assert_eq!(s.name, "solo");
        assert_eq!(s.host, c.host);
        assert_eq!(s.library_id, c.library_id);
        assert_eq!(s.folder_id, None);
    }

    #[test]
    fn descendants_of_unknown_node_is_a_typed_error() {
        let doc = LibraryDocument::empty();
        let missing = Uuid::new_v4().to_string();
        assert_eq!(
            doc.descendants(NodeId::library(&missing)),
            Err(LibraryError::UnknownNode {
                node: NodeId::library(&missing)
            })
        );
        assert_eq!(
            doc.descendants(NodeId::folder(&missing)),
            Err(LibraryError::UnknownNode {
                node: NodeId::folder(&missing)
            })
        );
        assert_eq!(
            doc.descendants(NodeId::connection(&missing)),
            Err(LibraryError::UnknownNode {
                node: NodeId::connection(&missing)
            })
        );
    }
}

#[cfg(test)]
mod migration_tests {
    //! Task 6.3 (Req 6.1/6.2/10.10/10.12): v1→v2 migration and
    //! write-blocked recovery. Versioned serde fixtures drive every case:
    //! a golden v1 bare array (the pre-2.0 `connections.json` shape) with
    //! expected v2 output asserted field-for-field, plus fault fixtures
    //! for malformed data, unsupported schemas, invalid backups and
    //! interrupted migration. Fault injection is admin-free: pre-created
    //! invalid/unreadable backups, read-only attributes (Windows), a
    //! read-only data directory (Unix) and simulated staging residue.

    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Golden v1 fixture (bare array, camelCase, serde-defaulted optional
    /// fields): two records so field preservation is proven across one
    /// fully-populated and one minimal record.
    const V1_GOLDEN: &str = r##"[
        {
            "id": "11111111-1111-4111-8111-111111111111",
            "name": "Alpha DC",
            "host": "alpha.invalid",
            "port": 3389,
            "username": "admin",
            "domain": "CORP",
            "colorAccent": "#ff0000",
            "tags": ["prod", "dc"],
            "displayWidth": 2560,
            "displayHeight": 1440,
            "createdAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-02T03:04:05Z",
            "lastConnectedAt": "2026-01-03T06:07:08Z"
        },
        {
            "id": "22222222-2222-4222-8222-222222222222",
            "name": "Beta APP",
            "host": "beta.invalid",
            "port": 33890,
            "mock": true,
            "createdAt": "2026-02-01T00:00:00Z",
            "updatedAt": "2026-02-01T00:00:00Z"
        }
    ]"##;

    /// A second, older v1 capture: used as a pre-existing valid backup to
    /// prove backups are never clobbered.
    const V1_OLDER_BACKUP: &str = r#"[
        {
            "id": "33333333-3333-4333-8333-333333333333",
            "name": "Old Host",
            "host": "old.invalid",
            "port": 3389,
            "createdAt": "2025-01-01T00:00:00Z",
            "updatedAt": "2025-01-01T00:00:00Z"
        }
    ]"#;

    /// Not JSON at all: the store must fail closed and keep these bytes.
    const MALFORMED_JSON: &str = "{ not valid json ";

    /// Recognizable object shape but an unsupported schema version.
    const V3_SCHEMA: &str = r#"{"version":3,"libraries":[],"folders":[],"connections":[]}"#;

    /// Valid JSON, claims v2, but the connection references a library that
    /// does not exist: a tree-invariant violation at load time.
    const V2_UNKNOWN_LIBRARY: &str = r#"{"version":2,"libraries":[],"folders":[],"connections":[
        {"id":"11111111-1111-4111-8111-111111111111","name":"X","host":"x.invalid","port":3389,
         "createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z",
         "libraryId":"99999999-9999-4999-8999-999999999999"}
    ]}"#;

    /// v1 shape with a non-UUID id: migration verifies every legacy ID and
    /// must fail closed instead of coercing it.
    const V1_INVALID_ID: &str = r#"[{"id":"legacy","name":"Legacy","host":"legacy.invalid","port":3389,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]"#;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn read_file(path: &Path) -> String {
        String::from_utf8(fs::read(path).unwrap()).unwrap()
    }

    fn parse_v1(json: &str) -> Vec<ConnectionConfig> {
        serde_json::from_str(json).unwrap()
    }

    fn backup_of(path: &Path) -> PathBuf {
        super::backup_path_for(path)
    }

    fn value_of(doc: &LibraryDocument) -> serde_json::Value {
        serde_json::to_value(doc).unwrap()
    }

    fn sample_v2_document() -> LibraryDocument {
        let library = Library::new("Work");
        let folder = Folder::new(&library.id, None, "Prod");
        let root_conn = ConnectionConfig {
            id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            name: "root-conn".to_string(),
            host: "root.invalid".to_string(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            mock: false,
            library_id: library.id.clone(),
            folder_id: None,
            created_at: Library::new("x").created_at,
            updated_at: Library::new("x").updated_at,
            last_connected_at: None,
        };
        let folder_conn = ConnectionConfig {
            id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
            name: "folder-conn".to_string(),
            host: "folder.invalid".to_string(),
            ..root_conn.clone()
        };
        LibraryDocument::new(vec![library], vec![folder], vec![root_conn, folder_conn])
    }

    /// Req 10.10 "without data loss": every field of every legacy record
    /// must survive migration untouched; only libraryId (default library)
    /// and folderId (root placement) are assigned.
    fn assert_preserved(
        original: &ConnectionConfig,
        migrated: &ConnectionConfig,
        library_id: &str,
    ) {
        assert_eq!(migrated.id, original.id, "id must survive migration");
        assert_eq!(migrated.name, original.name, "name must survive");
        assert_eq!(migrated.host, original.host, "host must survive");
        assert_eq!(migrated.port, original.port, "port must survive");
        assert_eq!(
            migrated.username, original.username,
            "username must survive"
        );
        assert_eq!(migrated.domain, original.domain, "domain must survive");
        assert_eq!(
            migrated.color_accent, original.color_accent,
            "colorAccent must survive"
        );
        assert_eq!(migrated.tags, original.tags, "tags must survive");
        assert_eq!(
            migrated.display_width, original.display_width,
            "displayWidth must survive"
        );
        assert_eq!(
            migrated.display_height, original.display_height,
            "displayHeight must survive"
        );
        assert_eq!(
            migrated.created_at, original.created_at,
            "createdAt must survive"
        );
        assert_eq!(
            migrated.updated_at, original.updated_at,
            "updatedAt must survive"
        );
        assert_eq!(
            migrated.last_connected_at, original.last_connected_at,
            "lastConnectedAt must survive"
        );
        assert_eq!(migrated.mock, original.mock, "mock must survive");
        assert_eq!(
            migrated.library_id, library_id,
            "migration assigns exactly the default library id"
        );
        assert_eq!(
            migrated.folder_id, None,
            "v1 connections are placed at the library root"
        );
    }

    /// Windows refuses to replace a read-only file: the deterministic,
    /// admin-free way to fail the v2 commit step itself.
    #[cfg(windows)]
    fn set_readonly(path: &Path, on: bool) {
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_readonly(on);
        fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn migrates_golden_v1_fixture_preserving_every_field_and_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);

        let outcome = load_or_migrate(&path).unwrap();

        let LoadOutcome::MigratedV2 {
            document,
            backup_path,
        } = outcome
        else {
            panic!("golden v1 input must migrate to v2, got {outcome:?}");
        };

        // Expected v2 output: version discriminator, exactly one default
        // library, no folders, every connection at the library root.
        document.validate().unwrap();
        assert_eq!(document.version, DOCUMENT_VERSION);
        let [library] = &document.libraries[..] else {
            panic!("expected exactly one default library");
        };
        assert_eq!(library.name, DEFAULT_LIBRARY_NAME);
        assert!(document.folders.is_empty(), "v1 had no folders");

        // Count and id parity (design "Data migration": assert both).
        let legacy = parse_v1(V1_GOLDEN);
        assert_eq!(document.connections.len(), legacy.len(), "count parity");
        let expected_ids: HashSet<&str> = legacy.iter().map(|c| c.id.as_str()).collect();
        let migrated_ids: HashSet<&str> =
            document.connections.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(migrated_ids, expected_ids, "id parity");

        for original in &legacy {
            let migrated = document
                .connections
                .iter()
                .find(|c| c.id == original.id)
                .expect("connection present by id");
            assert_preserved(original, migrated, &library.id);
        }

        // The backup holds the ORIGINAL v1 bytes, byte-for-byte.
        assert_eq!(backup_path, backup_of(&path));
        assert_eq!(
            fs::read(&backup_path).unwrap(),
            V1_GOLDEN.as_bytes(),
            "backup must preserve the original v1 bytes"
        );

        // The committed file is exactly the v2 document.
        let on_disk: LibraryDocument = serde_json::from_str(&read_file(&path)).unwrap();
        assert_eq!(value_of(&on_disk), value_of(&document));
    }

    #[test]
    fn restart_after_migration_loads_v2_as_is_and_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);

        let first = load_or_migrate(&path).unwrap();
        let migrated_doc = first.document();
        assert!(matches!(first, LoadOutcome::MigratedV2 { .. }));

        // Restart: the v2 file is used as-is — no second migration, no
        // backup rewrite, no data change.
        let second = load_or_migrate(&path).unwrap();
        assert!(
            matches!(second, LoadOutcome::LoadedV2(_)),
            "restart must load the committed v2 file as-is"
        );
        assert_eq!(value_of(&second.document()), value_of(&migrated_doc));
        assert_eq!(
            fs::read(backup_of(&path)).unwrap(),
            V1_GOLDEN.as_bytes(),
            "backup must stay untouched across restart"
        );
    }

    #[test]
    fn missing_file_starts_empty_ready_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");

        let store = LibraryStore::open(&path);

        assert_eq!(store.status(), StoreStatus::Ready);
        assert_eq!(
            value_of(&store.document().expect("ready store has a document")),
            value_of(&LibraryDocument::empty())
        );
        assert!(!path.exists(), "open must not create the store file");
        assert!(!backup_of(&path).exists(), "no backup without migration");
    }

    #[test]
    fn malformed_data_fails_closed_preserving_original_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, MALFORMED_JSON);

        let err = load_or_migrate(&path).unwrap_err();
        assert!(
            matches!(err, MigrationError::MalformedData { .. }),
            "typed error expected, got {err:?}"
        );

        // Fail closed: original bytes preserved byte-for-byte, no backup,
        // and the store opens write-blocked (Req 10.12).
        assert_eq!(
            fs::read(&path).unwrap(),
            MALFORMED_JSON.as_bytes(),
            "malformed original must be preserved byte-for-byte"
        );
        assert!(!backup_of(&path).exists(), "no backup without migration");
        let store = LibraryStore::open(&path);
        assert!(
            matches!(store.status(), StoreStatus::WriteBlocked { .. }),
            "store must open write-blocked on malformed data"
        );
    }

    #[test]
    fn unsupported_schema_fails_closed_for_every_unrecognized_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        for (label, fixture, found) in [
            ("v3 object", V3_SCHEMA, Some(3)),
            ("object without version", r#"{"libraries":[]}"#, None),
            ("scalar root", "42", None),
        ] {
            write_file(&path, fixture);
            let err = load_or_migrate(&path).unwrap_err();
            assert_eq!(
                err,
                MigrationError::UnsupportedSchema { found },
                "fail closed: {label}"
            );
            assert_eq!(
                fs::read(&path).unwrap(),
                fixture.as_bytes(),
                "unsupported schema must preserve the original bytes ({label})"
            );
        }
    }

    #[test]
    fn v2_document_with_invalid_tree_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V2_UNKNOWN_LIBRARY);

        let err = load_or_migrate(&path).unwrap_err();
        assert_eq!(
            err,
            MigrationError::InvalidData(LibraryError::UnknownLibrary {
                id: "99999999-9999-4999-8999-999999999999".to_string()
            }),
            "tree-invariant violations must surface the typed LibraryError"
        );
        assert_eq!(fs::read(&path).unwrap(), V2_UNKNOWN_LIBRARY.as_bytes());
    }

    #[test]
    fn v1_record_with_invalid_id_fails_closed_after_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_INVALID_ID);

        let err = load_or_migrate(&path).unwrap_err();
        assert_eq!(
            err,
            MigrationError::InvalidData(LibraryError::InvalidId {
                kind: "connection",
                id: "legacy".to_string()
            }),
            "migration verifies every legacy ID (design: verify every legacy field and ID)"
        );
        // Design flow order is Backup → Validate, so the backup of the
        // un migrated original exists — and the original is untouched.
        assert_eq!(fs::read(&path).unwrap(), V1_INVALID_ID.as_bytes());
        assert_eq!(
            fs::read(backup_of(&path)).unwrap(),
            V1_INVALID_ID.as_bytes(),
            "backup (written before validation) preserves the original"
        );
    }

    #[test]
    fn existing_invalid_backup_blocks_migration_preserving_everything() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);
        write_file(&backup_of(&path), "definitely not json");

        let err = load_or_migrate(&path).unwrap_err();
        assert!(
            matches!(err, MigrationError::BackupExistsInvalid { .. }),
            "typed error expected, got {err:?}"
        );
        // Fail closed: neither the original nor the suspicious backup was
        // touched, and no v2 was committed.
        assert_eq!(fs::read(&path).unwrap(), V1_GOLDEN.as_bytes());
        assert_eq!(fs::read(backup_of(&path)).unwrap(), b"definitely not json");
    }

    #[test]
    fn unreadable_existing_backup_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);
        // A directory at the backup path makes examining it fail on every
        // OS (EISDIR on Unix, access denied on Windows).
        fs::create_dir(backup_of(&path)).unwrap();

        let err = load_or_migrate(&path).unwrap_err();
        assert!(
            matches!(err, MigrationError::BackupInspectFailed { .. }),
            "typed error expected, got {err:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), V1_GOLDEN.as_bytes());
    }

    #[test]
    fn existing_valid_backup_is_never_clobbered_and_migration_retries() {
        // Restart after a migration interrupted between backup and commit:
        // a valid backup from the earlier attempt already exists while the
        // store file is still v1. The retry must keep the existing backup
        // untouched and complete the migration.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);
        write_file(&backup_of(&path), V1_OLDER_BACKUP);

        let outcome = load_or_migrate(&path).unwrap();

        assert!(matches!(outcome, LoadOutcome::MigratedV2 { .. }));
        assert_eq!(
            fs::read(backup_of(&path)).unwrap(),
            V1_OLDER_BACKUP.as_bytes(),
            "an existing valid backup must never be clobbered"
        );
        let on_disk: LibraryDocument = serde_json::from_str(&read_file(&path)).unwrap();
        let golden_ids: HashSet<String> = parse_v1(V1_GOLDEN).into_iter().map(|c| c.id).collect();
        let disk_ids: HashSet<String> = on_disk.connections.into_iter().map(|c| c.id).collect();
        assert_eq!(
            disk_ids, golden_ids,
            "ids preserved by the retried migration"
        );
    }

    /// Unix-only injection: a read-only data directory (chmod, no admin
    /// rights) fails backup creation; migration must block BEFORE any v2
    /// commit. Windows lacks an equivalent admin-free create-failure, so
    /// backup faults there are covered by the invalid/unreadable-backup
    /// tests above.
    #[cfg(unix)]
    #[test]
    fn backup_creation_failure_blocks_migration_before_commit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = load_or_migrate(&path);
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        let err = result.unwrap_err();
        assert!(
            matches!(err, MigrationError::BackupCreateFailed { .. }),
            "typed error expected, got {err:?}"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            V1_GOLDEN.as_bytes(),
            "original v1 must stay intact when the backup cannot be created"
        );
        assert!(
            !backup_of(&path).exists(),
            "no backup residue after failed creation"
        );
    }

    /// Windows-only injection: a read-only store file makes the atomic
    /// replace step itself fail (ERROR_ACCESS_DENIED, no admin rights).
    #[cfg(windows)]
    #[test]
    fn commit_failure_preserves_original_and_backup_then_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);
        set_readonly(&path, true);

        let store = LibraryStore::open(&path);
        assert!(
            matches!(store.status(), StoreStatus::WriteBlocked { .. }),
            "failed commit must open the store write-blocked"
        );
        let err = store.reload().unwrap_err();
        assert!(
            matches!(err, MigrationError::CommitFailed { .. }),
            "typed error expected, got {err:?}"
        );

        set_readonly(&path, false);
        assert_eq!(
            fs::read(&path).unwrap(),
            V1_GOLDEN.as_bytes(),
            "original v1 must survive the failed commit byte-for-byte"
        );
        assert_eq!(
            fs::read(backup_of(&path)).unwrap(),
            V1_GOLDEN.as_bytes(),
            "backup must survive the failed commit"
        );

        // Retry (restart-equivalent): the existing valid backup is kept
        // and the migration now completes.
        let outcome = store.reload().unwrap();
        assert!(matches!(outcome, LoadOutcome::MigratedV2 { .. }));
        assert_eq!(store.status(), StoreStatus::Ready);
        let on_disk: LibraryDocument = serde_json::from_str(&read_file(&path)).unwrap();
        let golden_ids: HashSet<String> = parse_v1(V1_GOLDEN).into_iter().map(|c| c.id).collect();
        let disk_ids: HashSet<String> = on_disk.connections.into_iter().map(|c| c.id).collect();
        assert_eq!(disk_ids, golden_ids);
    }

    #[test]
    fn write_blocked_recovery_rejects_mutations_and_cannot_save_empty_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, MALFORMED_JSON);

        let store = LibraryStore::open(&path);
        let blocked_status = store.status();
        let StoreStatus::WriteBlocked {
            reason,
            instructions,
        } = &blocked_status
        else {
            panic!("store must be write-blocked after a failed load");
        };
        assert!(!reason.is_empty(), "blocked status carries the cause");
        assert!(
            !instructions.is_empty(),
            "blocked status carries recovery instructions for the UI (task 8.7)"
        );

        // THE empty-replacement guard: saving an empty document over the
        // unreadable original would destroy the only copy of the data.
        let err = store.save(LibraryDocument::empty()).unwrap_err();
        assert!(
            matches!(err, MigrationError::WriteBlocked { .. }),
            "empty replacement must be rejected in recovery, got {err:?}"
        );
        // Every other mutation is blocked too (Req 10.12).
        let err = store.save(sample_v2_document()).unwrap_err();
        assert!(
            matches!(err, MigrationError::WriteBlocked { .. }),
            "all store mutations must be blocked in recovery"
        );
        assert!(
            store.document().is_none(),
            "no readable snapshot may be exposed while blocked"
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            MALFORMED_JSON.as_bytes(),
            "nothing may be written while blocked"
        );

        // Reload keeps blocking while the data is still invalid.
        assert!(store.reload().is_err());
        assert_eq!(store.status(), blocked_status);

        // Repair on disk, then reload lifts the block (Req 10.12: blocked
        // "until valid data is restored and reloaded").
        let repaired = sample_v2_document();
        write_file(&path, &serde_json::to_string_pretty(&repaired).unwrap());
        assert!(matches!(store.reload().unwrap(), LoadOutcome::LoadedV2(_)));
        assert_eq!(store.status(), StoreStatus::Ready);

        // Mutations work again and round-trip through the file.
        store.save(repaired.clone()).unwrap();
        let on_disk: LibraryDocument = serde_json::from_str(&read_file(&path)).unwrap();
        assert_eq!(value_of(&on_disk), value_of(&repaired));
    }

    #[test]
    fn leftover_staging_files_do_not_corrupt_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let doc = sample_v2_document();
        write_file(&path, &serde_json::to_string_pretty(&doc).unwrap());
        // Simulate residue from an interrupted staging round (tempfile's
        // ".tmp" prefix).
        let residue = dir.path().join(".tmpAbandonedStaging");
        fs::write(&residue, b"abandoned staging bytes").unwrap();

        let store = LibraryStore::open(&path);

        assert_eq!(store.status(), StoreStatus::Ready);
        assert_eq!(
            value_of(&store.document().unwrap()),
            value_of(&doc),
            "restart must load the prior data despite staging residue"
        );
        assert_eq!(
            fs::read(&residue).unwrap(),
            b"abandoned staging bytes",
            "unknown files are never deleted blindly"
        );

        // The store remains fully functional alongside the residue.
        store.save(doc.clone()).unwrap();
        let on_disk: LibraryDocument = serde_json::from_str(&read_file(&path)).unwrap();
        assert_eq!(value_of(&on_disk), value_of(&doc));
    }
}

#[cfg(test)]
mod crud_tests {
    //! Task 6.4 (Req 10.1/10.2/10.3/10.5): the CRUD and read-model facade
    //! behind the IPC commands in `commands::library`. The commands are
    //! thin marshalers; these tests specify the store they call: one
    //! serialized transaction per mutation, nested-descendant read models,
    //! snapshot preservation on every error path, and write-blocked
    //! rejection of CRUD and reads (Req 10.12).

    use super::*;
    use crate::store::connections::ConnectionConfig;
    use std::collections::HashSet;
    use std::sync::Arc;
    use uuid::Uuid;

    const MALFORMED: &str = "{ not a library document";
    const MISSING_ID: &str = "00000000-0000-4000-8000-000000000000";

    fn store_at(root: &std::path::Path, file: &str) -> LibraryStore {
        LibraryStore::open(root.join(file))
    }

    fn conn(library_id: &str, folder_id: Option<&str>, name: &str) -> ConnectionConfig {
        let now = Utc::now();
        ConnectionConfig {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            host: format!("{name}.invalid"),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            mock: false,
            library_id: library_id.to_string(),
            folder_id: folder_id.map(str::to_string),
            created_at: now,
            updated_at: now,
            last_connected_at: None,
        }
    }

    fn sorted(summaries: Vec<ConnectionSummary>) -> String {
        let mut names: Vec<String> = summaries.into_iter().map(|s| s.name).collect();
        names.sort();
        names.join(",")
    }

    #[test]
    fn create_and_rename_library_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");

        let created = store.create_library("Work").unwrap();
        assert_eq!(created.name, "Work");
        assert_eq!(
            Uuid::parse_str(&created.id).unwrap().get_version_num(),
            4,
            "fresh UUID v4 per library"
        );

        let renamed = store.rename_library(&created.id, "Personal").unwrap();
        assert_eq!(renamed.name, "Personal");
        assert_eq!(renamed.id, created.id);
        assert!(
            renamed.updated_at >= created.updated_at,
            "rename bumps updatedAt"
        );

        // Persistence round-trip: a fresh store over the same file sees
        // the rename (Req 10.2 persist).
        let reopened = store_at(dir.path(), "connections.json");
        let tree = reopened.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(tree.libraries[0].name, "Personal");
        assert_eq!(tree.libraries[0].id, created.id);
    }

    #[test]
    fn create_and_rename_folder_round_trip_under_root_and_nested() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let library = store.create_library("Work").unwrap();

        let root = store.create_folder(&library.id, None, "Prod").unwrap();
        assert_eq!(root.library_id, library.id);
        assert_eq!(root.parent_id, None, "no parent = library root");

        let nested = store
            .create_folder(&library.id, Some(&root.id), "Europe")
            .unwrap();
        assert_eq!(nested.parent_id.as_deref(), Some(root.id.as_str()));

        let renamed = store.rename_folder(&nested.id, "EMEA").unwrap();
        assert_eq!(renamed.name, "EMEA");

        let reopened = store_at(dir.path(), "connections.json");
        let tree = reopened.list_tree().unwrap();
        assert_eq!(tree.folders.len(), 2);
        let saved = tree
            .folders
            .iter()
            .find(|f| f.id == nested.id)
            .expect("renamed folder persists");
        assert_eq!(saved.name, "EMEA");
        assert_eq!(saved.parent_id.as_deref(), Some(root.id.as_str()));
    }

    #[test]
    fn descendants_lists_connections_in_nested_folders() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let a = store.create_library("A").unwrap();
        let b = store.create_library("B").unwrap();
        let root = store.create_folder(&a.id, None, "Prod").unwrap();
        let mid = store
            .create_folder(&a.id, Some(&root.id), "Europe")
            .unwrap();
        let deep = store.create_folder(&a.id, Some(&mid.id), "Berlin").unwrap();

        // Connection CRUD is task 6.6; seed the read model through the
        // store's validated whole-document save.
        store
            .save(LibraryDocument::new(
                vec![a.clone(), b.clone()],
                vec![root, mid, deep],
                vec![
                    conn(&a.id, None, "a-root"),
                    conn(&a.id, None, "a-deepless"),
                    conn(&b.id, None, "b-root"),
                ],
            ))
            .unwrap();
        // Re-file a connection into the deepest folder by saving the
        // adjusted document (nested placement for the query below).
        let mut document = store.document().expect("ready store has a snapshot");
        document.connections[1].folder_id = Some(document.folders[2].id.clone());
        store.save(document).unwrap();

        assert_eq!(
            sorted(store.descendants(NodeId::library(&a.id)).unwrap()),
            "a-deepless,a-root",
            "library node reaches the whole tree incl. nested folders"
        );
        let tree = store.list_tree().unwrap();
        let deep_conn = tree
            .connections
            .iter()
            .find(|c| c.name == "a-deepless")
            .unwrap();
        assert_eq!(
            sorted(
                store
                    .descendants(NodeId::folder(&tree.folders[1].id))
                    .unwrap()
            ),
            "a-deepless",
            "folder node reaches its subtree"
        );
        assert_eq!(
            store
                .descendants(NodeId::connection(&deep_conn.id))
                .unwrap()
                .len(),
            1,
            "connection node yields itself"
        );

        // Unknown node is a typed error, never a silent empty dashboard.
        let missing = NodeId::folder(MISSING_ID);
        assert!(matches!(
            store.descendants(missing.clone()),
            Err(LibraryStoreError::Invalid(LibraryError::UnknownNode { node })) if node == missing
        ));
    }

    #[test]
    fn list_tree_returns_flat_entities_and_connection_summaries() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let library = store.create_library("Work").unwrap();
        let folder = store.create_folder(&library.id, None, "Prod").unwrap();
        store
            .save(LibraryDocument::new(
                vec![library.clone()],
                vec![folder.clone()],
                vec![conn(&library.id, Some(&folder.id), "web-1")],
            ))
            .unwrap();

        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(tree.folders.len(), 1);
        assert_eq!(tree.connections.len(), 1);
        let summary = &tree.connections[0];
        assert_eq!(summary.name, "web-1");
        assert_eq!(summary.host, "web-1.invalid");
        assert_eq!(summary.library_id, library.id);
        assert_eq!(summary.folder_id.as_deref(), Some(folder.id.as_str()));
    }

    #[test]
    fn rejected_mutations_preserve_prior_snapshot_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = store_at(dir.path(), "connections.json");
        let library = store.create_library("Work").unwrap();
        let other = store.create_library("Other").unwrap();
        let foreign = store.create_folder(&other.id, None, "Foreign").unwrap();

        let before_tree = store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        // Blank names are rejected before anything is touched.
        assert!(matches!(
            store.create_library("   "),
            Err(LibraryStoreError::InvalidName { kind: "library" })
        ));
        assert!(matches!(
            store.rename_folder(&foreign.id, ""),
            Err(LibraryStoreError::InvalidName { kind: "folder" })
        ));
        // Unknown entities.
        assert!(matches!(
            store.rename_library(MISSING_ID, "X"),
            Err(LibraryStoreError::Invalid(
                LibraryError::UnknownLibrary { .. }
            ))
        ));
        assert!(matches!(
            store.rename_folder(MISSING_ID, "X"),
            Err(LibraryStoreError::Invalid(
                LibraryError::UnknownFolder { .. }
            ))
        ));
        assert!(matches!(
            store.create_folder(MISSING_ID, None, "X"),
            Err(LibraryStoreError::Invalid(
                LibraryError::UnknownLibrary { .. }
            ))
        ));
        assert!(matches!(
            store.create_folder(&library.id, Some(MISSING_ID), "X"),
            Err(LibraryStoreError::Invalid(
                LibraryError::UnknownFolder { .. }
            ))
        ));
        // Cross-library parent.
        assert!(matches!(
            store.create_folder(&library.id, Some(&foreign.id), "X"),
            Err(LibraryStoreError::Invalid(
                LibraryError::CrossLibraryParent { .. }
            ))
        ));

        // Prior snapshot (in memory) and file (on disk) are preserved.
        assert_eq!(store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
    }

    #[test]
    fn persist_failure_preserves_prior_snapshot_and_file() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file where the data directory should be makes every
        // commit fail after validation.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"not a directory").unwrap();
        let path = blocker.join("connections.json");
        let store = LibraryStore::open(&path);
        assert_eq!(
            store.status(),
            StoreStatus::Ready,
            "open on a missing file starts empty and ready"
        );

        let err = store.create_library("Work").unwrap_err();
        assert!(
            matches!(err, LibraryStoreError::PersistFailed { .. }),
            "got {err:?}"
        );

        // In-memory snapshot and disk are untouched by the failed mutation.
        assert!(store.list_tree().unwrap().libraries.is_empty());
        assert_eq!(std::fs::read(&blocker).unwrap(), b"not a directory");
        assert!(!path.exists());
    }

    #[test]
    fn write_blocked_store_rejects_every_crud_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, MALFORMED).unwrap();
        let store = LibraryStore::open(&path);
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));

        let results = [
            store.create_library("X").map(|_| ()),
            store.rename_library(MISSING_ID, "X").map(|_| ()),
            store.create_folder(MISSING_ID, None, "X").map(|_| ()),
            store.rename_folder(MISSING_ID, "X").map(|_| ()),
        ];
        for result in results {
            assert!(matches!(
                result,
                Err(LibraryStoreError::WriteBlocked { .. })
            ));
        }
        assert!(matches!(
            store.list_tree(),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.descendants(NodeId::library(MISSING_ID)),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));

        // Reloading still-invalid data keeps the block; the file is
        // preserved byte-for-byte throughout.
        assert!(store.reload().is_err());
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));
        assert_eq!(std::fs::read(&path).unwrap(), MALFORMED.as_bytes());
    }

    #[test]
    fn successful_reload_lifts_the_block_for_crud_and_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, MALFORMED).unwrap();
        let store = LibraryStore::open(&path);
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));

        let library = Library::new("Work");
        let valid = serde_json::to_string_pretty(&LibraryDocument::new(
            vec![library.clone()],
            Vec::new(),
            Vec::new(),
        ))
        .unwrap();
        std::fs::write(&path, valid).unwrap();

        store.reload().unwrap();
        assert_eq!(store.status(), StoreStatus::Ready);
        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(tree.libraries[0].id, library.id);

        // Mutations are allowed again once valid data is reloaded.
        store.create_library("Second").unwrap();
        assert_eq!(store.list_tree().unwrap().libraries.len(), 2);
    }

    #[test]
    fn concurrent_mutations_serialize_without_lost_updates() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(store_at(dir.path(), "connections.json"));

        let handles: Vec<_> = (0..8)
            .map(|t| {
                let store = Arc::clone(&store);
                std::thread::spawn(move || {
                    for i in 0..5 {
                        store.create_library(&format!("lib-{t}-{i}")).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 40, "every transaction lands");
        let ids: HashSet<&str> = tree.libraries.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids.len(), 40, "fresh UUID v4 per library, no duplicates");

        // The persisted file agrees with the published snapshot.
        let reopened = store_at(dir.path(), "connections.json");
        assert_eq!(reopened.list_tree().unwrap(), tree);
    }
}
