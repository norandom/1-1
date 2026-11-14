//! Task 6.1 (Req 10.1/10.5/10.8): v2 library document model and tree
//! invariants for the Royal TS-style connection tree; task 6.3 adds v1→v2
//! migration and write-blocked recovery; task 6.4 (Req 10.1/10.2/10.3/
//! 10.5) adds the CRUD/read facade over the single-owner [`LibraryStore`]
//! behind the IPC commands in `commands::library`; task 6.6 UNIFIES
//! connection storage onto this store: `connections.json` (v2 document)
//! is the one store file, the transitional `libraries.json` is merged in
//! or adopted at open (see [`load_unified`]), and connection
//! CRUD/import/duplication run through the same single-lock transactions
//! (the v1 `ConnectionStore` is retired).
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
use crate::store::connections::{ConnectionConfig, ConnectionInput};
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

/// Exact removal/move outcome of a destructive tree operation (Req 10.18,
/// design "Destructive actions and identity"): the deletion surface returns
/// precise counts — each list's length — plus the affected IDs, so the
/// confirmation UI (group 8) can state them verbatim. Folder deletion
/// (task 7.2) fills `deleted_folder_ids` and `moved_connection_ids` and
/// always leaves `deleted_connection_ids` empty: descendants move to the
/// library root instead of being deleted (Req 10.16). Library deletion
/// (task 7.3) will reuse the shape with deleted connections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CascadedIds {
    /// Every folder removed by the operation, in document order (the
    /// selected folder plus all nested folders for a folder deletion).
    pub deleted_folder_ids: Vec<String>,
    /// Connections removed by the operation. Empty for folder deletion.
    pub deleted_connection_ids: Vec<String>,
    /// Connections re-filed at a library root. Their IDs — and therefore
    /// their keyring accounts and active-session references — are
    /// unchanged (Req 10.16).
    pub moved_connection_ids: Vec<String>,
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
    /// A move addressed a library node as its source: libraries are tree
    /// roots, not movable subtrees (design "LibraryStore v2").
    MoveSourceIsLibrary { id: String },
    /// A move addressed a connection node as its target: only folders and
    /// libraries can receive nodes.
    MoveTargetIsConnection { id: String },
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
            Self::MoveSourceIsLibrary { id } => {
                write!(f, "library {id} cannot be moved (libraries are tree roots)")
            }
            Self::MoveTargetIsConnection { id } => write!(
                f,
                "connection {id} cannot receive a move (only folders and libraries can)"
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
    /// A connection create/update input failed validation (Req 1.1:
    /// blank name, blank/over-long host, port 0) or requested a mock
    /// flag in a production build (Req 12.10).
    InvalidConnection { reason: String },
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
            Self::InvalidConnection { reason } => {
                write!(f, "connection rejected: {reason}")
            }
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

/// Result of a successful [`load_or_migrate`] / [`load_unified`] /
/// [`LibraryStore::reload`].
#[derive(Debug, Clone)]
pub enum LoadOutcome {
    /// No store file on disk: a fresh store starts empty (nothing written).
    Fresh,
    /// The file already held a valid v2 document (used as-is, no backup).
    LoadedV2(LibraryDocument),
    /// A v1 bare array was migrated: the original bytes are preserved at
    /// `backup_path` and a v2 document was atomically committed. Task 6.6:
    /// with a transitional `libraries.json` present this is a MERGE — the
    /// legacy connections were placed in the existing v2 default library
    /// and the transitional file stays untouched (strict subset).
    MigratedV2 {
        document: LibraryDocument,
        backup_path: PathBuf,
    },
    /// Task 6.6 unification: no store file existed but a valid transitional
    /// `libraries.json` did — its document was adopted as the unified store
    /// (atomically committed to the store path). The transitional file at
    /// `source_path` is left untouched as the preserved original; no v1
    /// backup is involved (there were no v1 bytes).
    AdoptedV2 {
        document: LibraryDocument,
        source_path: PathBuf,
    },
}

impl LoadOutcome {
    /// The document this outcome leaves the store in.
    pub fn document(&self) -> LibraryDocument {
        match self {
            Self::Fresh => LibraryDocument::empty(),
            Self::LoadedV2(document) => document.clone(),
            Self::MigratedV2 { document, .. } => document.clone(),
            Self::AdoptedV2 { document, .. } => document.clone(),
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

    // Backup the ORIGINAL bytes before anything else; then validate the
    // wrapped document, assert count/id parity, and commit atomically.
    let backup_path = backup_v1_original(path, original)?;
    migrated.validate().map_err(MigrationError::InvalidData)?;
    assert_v1_parity(&legacy, &migrated)?;
    commit_v2(path, &migrated)?;

    Ok(LoadOutcome::MigratedV2 {
        document: migrated,
        backup_path,
    })
}

/// Backup tail shared by the 6.3 migration and the 6.6 unification merge
/// (design "Data migration" / "Storage transactions and recovery"):
/// preserve the ORIGINAL v1 bytes without clobber — an existing backup is
/// kept when valid, and an unusable existing backup fails the whole flow
/// closed. Data is never destroyed to make migration proceed.
#[cfg(test)]
static FAIL_NEXT_BACKUP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn backup_v1_original(path: &Path, original: &[u8]) -> Result<PathBuf, MigrationError> {
    let backup_path = backup_path_for(path);
    if backup_path.exists() {
        let existing =
            std::fs::read(&backup_path).map_err(|e| MigrationError::BackupInspectFailed {
                cause: e.to_string(),
            })?;
        validate_backup_bytes(&existing)
            .map_err(|cause| MigrationError::BackupExistsInvalid { cause })?;
    } else {
        #[cfg(test)]
        if FAIL_NEXT_BACKUP.swap(false, std::sync::atomic::Ordering::SeqCst) {
            return Err(MigrationError::BackupCreateFailed {
                cause: "injected for tests (root-proof, OS-independent)".into(),
            });
        }
        write_atomic(&backup_path, original).map_err(|e| MigrationError::BackupCreateFailed {
            cause: e.to_string(),
        })?;
    }
    Ok(backup_path)
}

/// Serialize and atomically commit a (caller-validated) v2 document to
/// `path`; on failure the prior file stays intact (task 6.2 semantics).
fn commit_v2(path: &Path, document: &LibraryDocument) -> Result<(), MigrationError> {
    let json =
        serde_json::to_string_pretty(document).map_err(|e| MigrationError::SerializeFailed {
            cause: e.to_string(),
        })?;
    write_atomic(path, json.as_bytes()).map_err(|e| MigrationError::CommitFailed {
        cause: e.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Task 6.6 unification: one store file (`connections.json` v2), the
// transitional `libraries.json` merged in or adopted once (Req 10.10/10.12,
// tasks.md Implementation Notes obligation)
// ---------------------------------------------------------------------------

/// The unified-store open/migration flow (task 6.6). `store_path` is THE
/// connection store file (`connections.json`, v2 document — design
/// "Logical Data Model", option A′); `transitional_path` is the v2 file a
/// 6.4/6.5 build persisted while the live `connections.json` was still
/// v1-owned. Branches, every one data-preserving:
///
/// * store file v2 → used as-is; a still-present transitional file is a
///   post-unification leftover and is deliberately ignored (and never
///   written) — the store file is authoritative;
/// * store file v1 → [`migrate_v1`] wrap; when a valid transitional
///   document exists the legacy connections are MERGED into its default
///   library ([`merge_v1_into_document`]) instead of overwriting it;
/// * store file missing → the transitional document is ADOPTED (committed
///   to the store path, original left untouched); no transitional means a
///   fresh empty store;
/// * any malformed/invalid file among the ones consulted fails closed
///   ([`MigrationError`]) with every byte preserved — the caller enters
///   write-blocked recovery, and the merge is restart-safe on both sides
///   of its atomic commit (pre-commit: inputs unchanged, deterministic
///   retry; post-commit: the store file is v2, so the first branch wins).
pub fn load_unified(
    store_path: &Path,
    transitional_path: &Path,
) -> Result<LoadOutcome, MigrationError> {
    let original = match std::fs::read(store_path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return adopt_or_fresh(store_path, transitional_path);
        }
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
    let found = root
        .get("version")
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok());
    match root {
        serde_json::Value::Array(_) => {
            let legacy: Vec<ConnectionConfig> =
                serde_json::from_value(root).map_err(|e| MigrationError::MalformedData {
                    cause: e.to_string(),
                })?;
            let base = read_transitional(transitional_path)?;
            let migrated = match &base {
                Some(base) => merge_v1_into_document(base.clone(), &legacy)?,
                None => migrate_v1(legacy.clone()),
            };
            let backup_path = backup_v1_original(store_path, &original)?;
            migrated.validate().map_err(MigrationError::InvalidData)?;
            match &base {
                Some(base) => assert_merge_parity(base, &legacy, &migrated)?,
                None => assert_v1_parity(&legacy, &migrated)?,
            }
            commit_v2(store_path, &migrated)?;
            Ok(LoadOutcome::MigratedV2 {
                document: migrated,
                backup_path,
            })
        }
        serde_json::Value::Object(_) => match found {
            Some(DOCUMENT_VERSION) => {
                let document: LibraryDocument =
                    serde_json::from_value(root).map_err(|e| MigrationError::MalformedData {
                        cause: e.to_string(),
                    })?;
                document.validate().map_err(MigrationError::InvalidData)?;
                // A leftover transitional file is intentionally ignored:
                // the unified store file is authoritative from here on.
                Ok(LoadOutcome::LoadedV2(document))
            }
            other => Err(MigrationError::UnsupportedSchema { found: other }),
        },
        _ => Err(MigrationError::UnsupportedSchema { found: None }),
    }
}

/// Read and validate the transitional `libraries.json` (task 6.6).
/// Missing → `None`; anything unreadable/malformed/invalid fails closed.
fn read_transitional(path: &Path) -> Result<Option<LibraryDocument>, MigrationError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(MigrationError::ReadFailed {
                cause: format!("cannot read the transitional libraries.json: {e}"),
            });
        }
    };
    let document: LibraryDocument =
        serde_json::from_slice(&bytes).map_err(|e| MigrationError::MalformedData {
            cause: format!("transitional libraries.json is not a v2 document: {e}"),
        })?;
    document.validate().map_err(MigrationError::InvalidData)?;
    Ok(Some(document))
}

/// Merge v1 legacy connections INTO an existing (possibly user-populated)
/// v2 document (tasks.md 6.6 obligation): every legacy connection lands in
/// the default library's root ([`default_library_in`]), field-for-field
/// untouched apart from the documented ownership defaults. An id that
/// already exists in the base fails closed — re-iding would orphan the
/// keyring entry keyed by that connection id.
fn merge_v1_into_document(
    mut base: LibraryDocument,
    legacy: &[ConnectionConfig],
) -> Result<LibraryDocument, MigrationError> {
    let existing_ids: HashSet<&str> = base.connections.iter().map(|c| c.id.as_str()).collect();
    if let Some(clash) = legacy.iter().find(|c| existing_ids.contains(c.id.as_str())) {
        return Err(MigrationError::ParityCheckFailed {
            detail: format!(
                "legacy connection id {} already exists in the v2 document",
                clash.id
            ),
        });
    }
    let default_library_id = default_library_in(&mut base);
    let mut merged = legacy.to_vec();
    for connection in &mut merged {
        connection.library_id = default_library_id.clone();
        connection.folder_id = None;
    }
    base.connections.extend(merged);
    Ok(base)
}

/// Resolve the document's default library — the merge target for v1
/// connections (Req 10.10) and the destination of unspecified creates:
/// a library NAMED `Default Library` wins, else the first library
/// (creation order), else a fresh `Default Library` is created.
fn default_library_in(document: &mut LibraryDocument) -> String {
    if let Some(existing) = document
        .libraries
        .iter()
        .find(|library| library.name == DEFAULT_LIBRARY_NAME)
    {
        return existing.id.clone();
    }
    if let Some(first) = document.libraries.first() {
        return first.id.clone();
    }
    let library = Library::new(DEFAULT_LIBRARY_NAME);
    let id = library.id.clone();
    document.libraries.push(library);
    id
}

/// Merge parity (design "assert count and id parity", extended to the
/// union): every legacy connection survives and nothing else is dropped.
fn assert_merge_parity(
    base: &LibraryDocument,
    legacy: &[ConnectionConfig],
    merged: &LibraryDocument,
) -> Result<(), MigrationError> {
    let expected = base.connections.len() + legacy.len();
    if merged.connections.len() != expected {
        return Err(MigrationError::ParityCheckFailed {
            detail: format!(
                "merge changed the connection count: {expected} expected, {} found",
                merged.connections.len()
            ),
        });
    }
    let legacy_ids: HashSet<&str> = legacy.iter().map(|c| c.id.as_str()).collect();
    let merged_ids: HashSet<&str> = merged.connections.iter().map(|c| c.id.as_str()).collect();
    if !legacy_ids.is_subset(&merged_ids) {
        return Err(MigrationError::ParityCheckFailed {
            detail: "legacy connection id set did not survive the merge".to_string(),
        });
    }
    Ok(())
}

/// No store file yet: adopt a valid transitional document (commit it to
/// the store path; the transitional original stays untouched), or start
/// fresh when neither file exists.
fn adopt_or_fresh(
    store_path: &Path,
    transitional_path: &Path,
) -> Result<LoadOutcome, MigrationError> {
    match read_transitional(transitional_path)? {
        Some(document) => {
            commit_v2(store_path, &document)?;
            Ok(LoadOutcome::AdoptedV2 {
                document,
                source_path: transitional_path.to_path_buf(),
            })
        }
        None => Ok(LoadOutcome::Fresh),
    }
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
    /// Task 6.6: the transitional `libraries.json` consulted once by
    /// [`load_unified`] (merge/adopt) while the store opens or reloads;
    /// `None` for single-file stores ([`LibraryStore::open`]).
    transitional_path: Option<PathBuf>,
    state: StoreState,
    /// Test-only commit-failure injection (root-proof, OS-independent —
    /// CI run 43587595668 lesson: permission-based injections cannot fail
    /// inside the root-run Linux container). Checked in `commit_publish`
    /// between serialization and the persist stage; fires once.
    #[cfg(test)]
    fail_next_commit: bool,
    /// Test-only race-window injection (task 7.3): connections a test
    /// stages to JOIN the document inside the next `delete_library`
    /// transaction — the deterministic model of a connection that was
    /// filed into the library between the coordination layer's
    /// `library_connection_ids` preview and the commit (the post-commit
    /// "extras" sweep window). Drained by `delete_library`; never
    /// present in production builds.
    #[cfg(test)]
    library_delete_joins: Vec<ConnectionConfig>,
}

/// Owner of the unified v2 connection library document (task 6.6): the
/// store file IS `connections.json` (design "Logical Data Model", option
/// A′). [`LibraryStore::open_unified`] additionally consults the
/// transitional `libraries.json` a 6.4/6.5 build may have left behind and
/// merges/adopts it once without data loss ([`load_unified`]). The store
/// loads (migrating v1 on first sight), serves the current snapshot, and
/// gates every mutation behind the write-blocked recovery state
/// (Req 10.10/10.12).
/// On any load/migration failure the store opens in
/// [`StoreStatus::WriteBlocked`]: the on-disk file and backup are left
/// untouched, every mutation is rejected — including the empty replacement
/// that would wipe unreadable data — and only a successful
/// [`LibraryStore::reload`] lifts the block.
pub struct LibraryStore {
    inner: Mutex<StoreInner>,
}

impl LibraryStore {
    /// Open (or restart) a single-file store at `path`, migrating a v1
    /// file per the design flow. Never panics and never writes on
    /// failure: a failed load yields a write-blocked store.
    ///
    /// Test-facing (task 6.6, same policy as [`LibraryStore::document`]/
    /// [`LibraryStore::save`]): production opens the UNIFIED store
    /// ([`LibraryStore::open_unified`] in `lib.rs`); the single-file
    /// variant remains for the 6.3–6.5 store test suites.
    #[cfg(test)]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::open_with(path, None)
    }

    /// Open (or restart) the UNIFIED store (task 6.6): `path` is the
    /// `connections.json` store file and `transitional_path` the legacy
    /// `libraries.json` (see [`load_unified`] for the merge/adopt/fresh
    /// semantics). Like [`LibraryStore::open`], never panics and never
    /// writes on failure.
    pub fn open_unified(path: impl Into<PathBuf>, transitional_path: impl Into<PathBuf>) -> Self {
        Self::open_with(path, Some(transitional_path.into()))
    }

    /// Test-only: make the NEXT `commit_publish` fail deterministically
    /// after validation, on every OS — including the root-run CI Linux
    /// container where permission-based injections are ineffective
    /// (CI run 43587595668).
    #[cfg(test)]
    pub fn fail_next_commit(&self) {
        self.inner
            .lock()
            .expect("library store lock poisoned")
            .fail_next_commit = true;
    }

    /// Test-only (task 7.3): stage connections that JOIN the library
    /// inside the next [`LibraryStore::delete_library`] transaction —
    /// the deterministic stand-in for a connection filed into the
    /// library between the caller's [`LibraryStore::library_connection_ids`]
    /// preview and the commit. The staged connections are drained by
    /// that one transaction (before its cascade is computed), so they
    /// end up deleted-but-unpreviewed — exactly the "extras" the
    /// coordination layer's post-commit sweep must handle.
    #[cfg(test)]
    pub fn join_connections_inside_next_library_delete(&self, joined: Vec<ConnectionConfig>) {
        self.inner
            .lock()
            .expect("library store lock poisoned")
            .library_delete_joins = joined;
    }

    fn open_with(path: impl Into<PathBuf>, transitional_path: Option<PathBuf>) -> Self {
        let path = path.into();
        let state = match Self::load_for(&path, &transitional_path) {
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
                    LoadOutcome::AdoptedV2 { source_path, .. } => {
                        log::info!(
                            "Library store: adopted transitional {} into {} (original untouched)",
                            source_path.display(),
                            path.display()
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
            inner: Mutex::new(StoreInner {
                path,
                transitional_path,
                state,
                #[cfg(test)]
                fail_next_commit: false,
                #[cfg(test)]
                library_delete_joins: Vec::new(),
            }),
        }
    }

    /// Run the load/migration flow for the configured paths: the unified
    /// loader when a transitional file participates, else the 6.3 flow.
    fn load_for(
        path: &Path,
        transitional_path: &Option<PathBuf>,
    ) -> Result<LoadOutcome, MigrationError> {
        match transitional_path {
            Some(transitional_path) => load_unified(path, transitional_path),
            None => load_or_migrate(path),
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

    /// Re-run the load/migration from disk (the unified flow when a
    /// transitional path is configured). On success the store becomes
    /// ready with the (possibly migrated/merged) document; on failure it
    /// stays write-blocked. This is the only path out of recovery
    /// (Req 10.12: "until valid data is restored and reloaded").
    pub fn reload(&self) -> Result<LoadOutcome, MigrationError> {
        let mut inner = self.inner.lock().expect("library store lock poisoned");
        match Self::load_for(&inner.path, &inner.transitional_path) {
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

    // ---- Task 6.5: atomic cross-library moves (Req 10.8/10.15) -------

    /// Req 10.8/10.15 (design "Storage transactions and recovery",
    /// "LibraryStore v2" and "Library Commands"): atomically move a folder
    /// or connection — including across libraries — in ONE serialized
    /// transaction: snapshot → [`apply_move`] → validate → persist →
    /// publish. On any rejection or failed persist the previous
    /// in-memory snapshot and on-disk file are retained (Req 10.15).
    ///
    /// * folder source: re-parents the folder (a library target means
    ///   that library's root) and, when crossing libraries, rewrites
    ///   `library_id` on the folder, every descendant folder and every
    ///   connection filed inside the subtree, so single-library ancestry
    ///   keeps holding;
    /// * connection source: re-files the connection (a library target
    ///   means its root); only `library_id`/`folder_id` change — ids,
    ///   credentials (moves never touch the keyring), settings and
    ///   active-session references are preserved;
    /// * typed rejections (design L336): library nodes as sources,
    ///   connection nodes as targets, unknown node/target, and folders
    ///   moved into their own subtree or onto themselves (cycles).
    ///
    /// Signature reconciliation (6.4 precedent, design contract block):
    /// design lists `&mut self` + `LibraryError`, but the store serializes
    /// every transaction through its interior lock behind `&self`
    /// (shared `State` in commands) and a move can also fail `WriteBlocked`
    /// or `PersistFailed`, which a bare `LibraryError` cannot express —
    /// the same amendment recorded for `descendants`.
    pub fn move_node(&self, node: NodeId, target: NodeId) -> Result<(), LibraryStoreError> {
        self.mutate(|mut next| {
            apply_move(&mut next, node, target)?;
            Ok((next, ()))
        })
    }

    /// Req 10.16/10.18 (task 7.2): delete a folder subtree in ONE serialized
    /// store transaction. The folder and every nested folder are removed;
    /// every descendant connection — at any depth — is re-filed at the SAME
    /// library's root (`folder_id -> None`, `library_id` unchanged), never
    /// deleted. Connection IDs are unchanged, so keyring accounts (service +
    /// connection ID) and active-session references keep resolving; the
    /// deletion path performs no keyring cleanup for moved records (design
    /// "Destructive actions and identity"). Returns the exact cascaded
    /// counts/IDs ([`CascadedIds`]) so the confirmation surface can state
    /// them (Req 10.18). Unknown folder IDs are typed rejections; a
    /// write-blocked store refuses the deletion; a failed persist retains
    /// the previous snapshot and on-disk file.
    pub fn delete_folder(&self, id: &str) -> Result<CascadedIds, LibraryStoreError> {
        self.mutate(|mut next| {
            let cascaded = apply_folder_deletion(&mut next, id)?;
            Ok((next, cascaded))
        })
    }

    /// Req 10.2/10.17/10.18 (task 7.3): delete a library together with
    /// every folder and every connection filed in it — root-level and
    /// nested, at any depth — in ONE serialized store transaction. The
    /// command coordination layer (`commands::deletion`) stops the
    /// affected sessions first and performs keyring cleanup from the
    /// returned [`CascadedIds`] only after this transaction committed;
    /// a failed persist retains the previous snapshot and on-disk file
    /// (the library deletion then aborts with saved data retained).
    pub fn delete_library(&self, id: &str) -> Result<CascadedIds, LibraryStoreError> {
        // Test-only race-window injection (see `library_delete_joins`):
        // the staged connections join the document HERE — after any
        // earlier `library_connection_ids` preview, before the cascade —
        // so the transaction deletes them unpreviewed.
        #[cfg(test)]
        let joined = std::mem::take(
            &mut self
                .inner
                .lock()
                .expect("library store lock poisoned")
                .library_delete_joins,
        );
        self.mutate(|mut next| {
            #[cfg(test)]
            next.connections.extend(joined);
            let cascaded = apply_library_deletion(&mut next, id)?;
            Ok((next, cascaded))
        })
    }

    /// Req 10.17 (task 7.3): the connection IDs a library deletion will
    /// affect, in document order — the preview the coordination layer
    /// stops session actors for (and tombstones against Connect) before
    /// [`LibraryStore::delete_library`] commits. An unknown library is
    /// the same typed rejection the deletion itself would return.
    pub fn library_connection_ids(&self, id: &str) -> Result<Vec<String>, LibraryStoreError> {
        self.read(|document| {
            if !document.libraries.iter().any(|library| library.id == id) {
                return Err(LibraryError::UnknownNode {
                    node: NodeId::library(id),
                }
                .into());
            }
            Ok(document
                .connections
                .iter()
                .filter(|connection| connection.library_id == id)
                .map(|connection| connection.id.clone())
                .collect())
        })
    }

    // ---- Task 6.6: connection CRUD, reads, duplication (Req 1.1-1.5,
    // 10.7, 10.19, 12.10) — the retired v1 ConnectionStore behavior
    // unified onto the single-lock transaction core. Commands in
    // `commands::connection` stay thin marshalers; keyring work stays at
    // the command layer because credentials never enter the document.

    /// Req 1.1/1.2: create a connection in one serialized transaction.
    /// Input validation (name/host/port + mock policy) happens BEFORE the
    /// transaction; the destination resolves from the payload's ownership
    /// fields — explicit `library_id`, else the `folder_id`'s library,
    /// else the store's default library (created lazily) — and unknown or
    /// cross-library placements are typed rejections from the
    /// transaction's validation step. The payload password is never
    /// persisted (the command layer routes it to the keyring).
    pub fn create_connection(
        &self,
        input: ConnectionInput,
    ) -> Result<ConnectionConfig, LibraryStoreError> {
        validate_connection_input(&input)?;
        let mock = resolve_mock_flag(input.mock)?;
        self.mutate(|mut next| {
            let library_id = if !input.library_id.is_empty() {
                input.library_id.clone()
            } else if let Some(folder_id) = input.folder_id.as_deref() {
                next.folders
                    .iter()
                    .find(|folder| folder.id == folder_id)
                    .ok_or_else(|| LibraryError::UnknownFolder {
                        id: folder_id.to_string(),
                    })?
                    .library_id
                    .clone()
            } else {
                default_library_in(&mut next)
            };
            if !next
                .libraries
                .iter()
                .any(|library| library.id == library_id)
            {
                return Err(LibraryError::UnknownLibrary { id: library_id }.into());
            }
            let now = Utc::now();
            let config = ConnectionConfig {
                id: Uuid::new_v4().to_string(),
                name: input.name,
                host: input.host,
                port: input.port,
                username: input.username,
                domain: input.domain,
                color_accent: input.color_accent,
                tags: input.tags,
                display_width: input.display_width,
                display_height: input.display_height,
                mock,
                library_id,
                folder_id: input.folder_id,
                created_at: now,
                updated_at: now,
                last_connected_at: None,
            };
            next.connections.push(config.clone());
            Ok((next, config))
        })
    }

    /// Req 1.1: update a connection's editable fields (full replacement
    /// of the input-covered fields, v1 semantics). Identity, ownership
    /// (`library_id`/`folder_id` — re-filing is [`LibraryStore::move_node`])
    /// and history (`created_at`, `last_connected_at`) are preserved;
    /// `Ok(None)` mirrors the v1 contract for unknown ids. The payload
    /// password is ignored here (the command layer routes it to the
    /// keyring; the file stays secret-free either way).
    pub fn update_connection(
        &self,
        id: &str,
        input: ConnectionInput,
    ) -> Result<Option<ConnectionConfig>, LibraryStoreError> {
        validate_connection_input(&input)?;
        self.mutate(|mut next| {
            let Some(existing) = next.connections.iter_mut().find(|c| c.id == id) else {
                return Ok((next, None));
            };
            existing.name = input.name;
            existing.host = input.host;
            existing.port = input.port;
            existing.username = input.username;
            existing.domain = input.domain;
            existing.color_accent = input.color_accent;
            existing.tags = input.tags;
            existing.display_width = input.display_width;
            existing.display_height = input.display_height;
            existing.mock = merge_update_mock_flag(existing.mock, input.mock);
            existing.updated_at = Utc::now();
            let updated = existing.clone();
            Ok((next, Some(updated)))
        })
    }

    /// Req 1.5 (store-level semantics): remove the connection from the
    /// library. Best-effort keyring deletion stays at the command layer.
    pub fn delete_connection(&self, id: &str) -> Result<bool, LibraryStoreError> {
        self.mutate(|mut next| {
            let before = next.connections.len();
            next.connections.retain(|connection| connection.id != id);
            let removed = before != next.connections.len();
            Ok((next, removed))
        })
    }

    /// Stamp `last_connected_at` after a successful connect (v1 behavior:
    /// a missing id is a silent no-op).
    pub fn update_last_connected(&self, id: &str) -> Result<(), LibraryStoreError> {
        self.mutate(|mut next| {
            if let Some(connection) = next.connections.iter_mut().find(|c| c.id == id) {
                connection.last_connected_at = Some(Utc::now());
            }
            Ok((next, ()))
        })
    }

    /// Req 10.19 (design "Destructive actions and identity"): duplicate a
    /// connection — copy NON-SECRET fields into the SAME folder with a
    /// fresh UUID v4, a deterministic available name ("Name copy",
    /// "Name copy 2", … among the destination scope's siblings) and fresh
    /// timestamps/history. The keyring is never touched: the store model
    /// holds no secret material and the command layer performs no
    /// credential call on this path, so the duplicate requires password
    /// re-entry by construction.
    pub fn duplicate_connection(&self, id: &str) -> Result<ConnectionConfig, LibraryStoreError> {
        self.mutate(|mut next| {
            let source = next
                .connections
                .iter()
                .find(|connection| connection.id == id)
                .ok_or_else(|| LibraryError::UnknownNode {
                    node: NodeId::connection(id),
                })?
                .clone();
            let name = available_copy_name(
                &source.name,
                &next.connections,
                &source.library_id,
                source.folder_id.as_deref(),
            );
            let now = Utc::now();
            let duplicate = ConnectionConfig {
                id: Uuid::new_v4().to_string(),
                name,
                mock: duplicate_mock_flag(source.mock),
                created_at: now,
                updated_at: now,
                last_connected_at: None,
                ..source
            };
            next.connections.push(duplicate.clone());
            Ok((next, duplicate))
        })
    }

    /// Fetch one connection (v1 `get` contract, plus the v2 typed
    /// write-blocked read error).
    pub fn get_connection(&self, id: &str) -> Result<Option<ConnectionConfig>, LibraryStoreError> {
        self.read(|document| {
            Ok(document
                .connections
                .iter()
                .find(|connection| connection.id == id)
                .cloned())
        })
    }

    /// Every connection, name-sorted (v1 `list` contract).
    pub fn list_connections(&self) -> Result<Vec<ConnectionConfig>, LibraryStoreError> {
        self.read(|document| {
            let mut list = document.connections.clone();
            list.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(list)
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
        #[cfg(test)]
        if inner.fail_next_commit {
            inner.fail_next_commit = false;
            return Err(MigrationError::CommitFailed {
                cause: "injected for tests (root-proof, OS-independent)".into(),
            });
        }
        write_atomic(&inner.path, json.as_bytes()).map_err(|e| MigrationError::CommitFailed {
            cause: e.to_string(),
        })?;
        // Publish only after the commit succeeded.
        inner.state = StoreState::Ready(document);
        Ok(())
    }
}

/// Folder ids of the whole subtree rooted at `root_id` (inclusive), via a
/// children index and iterative BFS so deep nesting cannot overflow the
/// stack. Owned ids so callers can keep the set while mutating the
/// document. Shared by tree moves ([`apply_move`]) and folder deletion
/// ([`apply_folder_deletion`]).
fn folder_subtree(folders: &[Folder], root_id: &str) -> HashSet<String> {
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    for folder in folders {
        if let Some(parent) = folder.parent_id.as_deref() {
            children.entry(parent).or_default().push(folder.id.as_str());
        }
    }
    let mut subtree: HashSet<String> = HashSet::from([root_id.to_string()]);
    let mut queue: VecDeque<String> = VecDeque::from([root_id.to_string()]);
    while let Some(current) = queue.pop_front() {
        for child in children
            .get(current.as_str())
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if subtree.insert((*child).to_string()) {
                queue.push_back((*child).to_string());
            }
        }
    }
    subtree
}

/// Pure tree-move core of [`LibraryStore::move_node`] (task 6.5): apply
/// the move to the proposed snapshot in place; the surrounding
/// transaction validates the result before persisting it.
///
/// `node` must be a folder or connection (libraries are tree roots);
/// `target` must be a library (re-file at its root) or a folder (move
/// into it). Folder moves re-parent the subtree and, when crossing
/// libraries, rewrite `library_id` on every descendant folder and every
/// connection filed inside the subtree, so single-library ancestry keeps
/// holding. Connection moves change only `library_id`/`folder_id` —
/// ids, settings and timestamps (and the keyring) are untouched.
fn apply_move(
    document: &mut LibraryDocument,
    node: NodeId,
    target: NodeId,
) -> Result<(), LibraryError> {
    if node.kind == NodeKind::Library {
        return Err(LibraryError::MoveSourceIsLibrary { id: node.id });
    }
    if target.kind == NodeKind::Connection {
        return Err(LibraryError::MoveTargetIsConnection { id: target.id });
    }

    // Resolve the receiving library before touching anything: a library
    // target owns the drop point itself, a folder target owns its folder.
    let target_library_id = match target.kind {
        NodeKind::Library => document
            .libraries
            .iter()
            .find(|library| library.id == target.id)
            .map(|library| library.id.clone())
            .ok_or_else(|| LibraryError::UnknownNode {
                node: target.clone(),
            })?,
        _ => document
            .folders
            .iter()
            .find(|folder| folder.id == target.id)
            .map(|folder| folder.library_id.clone())
            .ok_or_else(|| LibraryError::UnknownNode {
                node: target.clone(),
            })?,
    };

    match node.kind {
        NodeKind::Folder => {
            if !document.folders.iter().any(|folder| folder.id == node.id) {
                return Err(LibraryError::UnknownNode { node: node.clone() });
            }
            // Subtree of the PRE-MOVE tree (children index + BFS,
            // iterative so deep nesting cannot overflow): the moved
            // folder plus every descendant that follows it across
            // libraries.
            let subtree = folder_subtree(&document.folders, &node.id);
            // A folder cannot move into its own subtree — this also covers
            // moving onto itself: either would form a parent cycle
            // (design "LibraryStore v2" / "Storage transactions").
            if target.kind == NodeKind::Folder && subtree.contains(target.id.as_str()) {
                return Err(LibraryError::FolderCycle { folder_id: node.id });
            }

            let new_parent = (target.kind == NodeKind::Folder).then(|| target.id.clone());
            for folder in &mut document.folders {
                if folder.id == node.id {
                    folder.parent_id = new_parent.clone();
                    folder.library_id = target_library_id.clone();
                } else if subtree.contains(folder.id.as_str()) {
                    folder.library_id = target_library_id.clone();
                }
            }
            for connection in &mut document.connections {
                if connection
                    .folder_id
                    .as_deref()
                    .is_some_and(|folder_id| subtree.contains(folder_id))
                {
                    connection.library_id = target_library_id.clone();
                }
            }
        }
        NodeKind::Connection => {
            let new_folder = (target.kind == NodeKind::Folder).then(|| target.id.clone());
            let Some(connection) = document
                .connections
                .iter_mut()
                .find(|connection| connection.id == node.id)
            else {
                return Err(LibraryError::UnknownNode { node: node.clone() });
            };
            connection.library_id = target_library_id;
            connection.folder_id = new_folder;
        }
        // Rejected above: libraries are not valid move sources.
        NodeKind::Library => unreachable!("library move sources are rejected up front"),
    }
    Ok(())
}

/// Pure deletion core of [`LibraryStore::delete_folder`] (task 7.2, Req
/// 10.16): remove the folder subtree and re-file every descendant
/// connection at the same library's root, in the proposed snapshot in
/// place. The surrounding transaction validates the result before
/// persisting it.
///
/// Single-library ancestry (the document invariant) guarantees the whole
/// subtree and every connection filed inside it belong to the selected
/// folder's one library, so the survivors keep their `library_id` — the
/// ONLY field that changes is the re-filed connections' `folder_id`.
fn apply_folder_deletion(
    document: &mut LibraryDocument,
    folder_id: &str,
) -> Result<CascadedIds, LibraryError> {
    // Resolve the folder before touching anything; an unknown id is a
    // typed rejection, never a silent no-op (same stance as moves).
    if !document.folders.iter().any(|folder| folder.id == folder_id) {
        return Err(LibraryError::UnknownNode {
            node: NodeId::folder(folder_id),
        });
    }
    let subtree = folder_subtree(&document.folders, folder_id);

    // Capture the exact outcome (Req 10.18) in document order BEFORE
    // mutating, so the returned counts and IDs describe precisely what
    // the transaction below applies.
    let deleted_folder_ids: Vec<String> = document
        .folders
        .iter()
        .filter(|folder| subtree.contains(folder.id.as_str()))
        .map(|folder| folder.id.clone())
        .collect();
    let moved_connection_ids: Vec<String> = document
        .connections
        .iter()
        .filter(|connection| {
            connection
                .folder_id
                .as_deref()
                .is_some_and(|filed_under| subtree.contains(filed_under))
        })
        .map(|connection| connection.id.clone())
        .collect();

    // Remove the folder and every nested folder…
    document
        .folders
        .retain(|folder| !subtree.contains(folder.id.as_str()));
    // …and re-file every descendant connection — at any depth — at the
    // library root. IDs, `library_id`, settings and timestamps are
    // untouched, so keyring accounts (service + connection ID) and
    // active-session references keep resolving (Req 10.16); no keyring
    // cleanup exists for moved records (design "Destructive actions and
    // identity").
    for connection in &mut document.connections {
        if connection
            .folder_id
            .as_deref()
            .is_some_and(|filed_under| subtree.contains(filed_under))
        {
            connection.folder_id = None;
        }
    }

    Ok(CascadedIds {
        deleted_folder_ids,
        // Folder deletion never deletes connections (Req 10.16) — the
        // field stays available for library deletion (task 7.3).
        deleted_connection_ids: Vec::new(),
        moved_connection_ids,
    })
}

/// Pure deletion core of [`LibraryStore::delete_library`] (task 7.3, Req
/// 10.2/10.17/10.18): remove the library and every folder and connection
/// filed in it, in the proposed snapshot in place. The surrounding
/// transaction validates the result before persisting it.
///
/// Folders carry their owning `library_id` directly and connections
/// reference theirs through the single-library ancestry invariant, so
/// "filed in the library" is a flat filter — no subtree walk. Nothing is
/// re-filed and nothing survives inside a deleted library, so
/// `moved_connection_ids` is always empty here: library deletion
/// deletes connections, it never moves them.
fn apply_library_deletion(
    document: &mut LibraryDocument,
    library_id: &str,
) -> Result<CascadedIds, LibraryError> {
    // Resolve the library before touching anything; an unknown id is a
    // typed rejection, never a silent no-op (same stance as moves and
    // folder deletion).
    if !document
        .libraries
        .iter()
        .any(|library| library.id == library_id)
    {
        return Err(LibraryError::UnknownNode {
            node: NodeId::library(library_id),
        });
    }

    // Capture the exact outcome (Req 10.18) in document order BEFORE
    // mutating, so the returned counts and IDs describe precisely what
    // the transaction below applies.
    let deleted_folder_ids: Vec<String> = document
        .folders
        .iter()
        .filter(|folder| folder.library_id == library_id)
        .map(|folder| folder.id.clone())
        .collect();
    let deleted_connection_ids: Vec<String> = document
        .connections
        .iter()
        .filter(|connection| connection.library_id == library_id)
        .map(|connection| connection.id.clone())
        .collect();

    document
        .libraries
        .retain(|library| library.id != library_id);
    document
        .folders
        .retain(|folder| folder.library_id != library_id);
    document
        .connections
        .retain(|connection| connection.library_id != library_id);

    Ok(CascadedIds {
        deleted_folder_ids,
        deleted_connection_ids,
        moved_connection_ids: Vec::new(),
    })
}

/// Store-layer input validation for entity names, so the IPC commands stay
/// thin marshalers (task 6.4): a name must be non-blank after trimming.
fn check_name(kind: &'static str, name: &str) -> Result<(), LibraryStoreError> {
    if name.trim().is_empty() {
        return Err(LibraryStoreError::InvalidName { kind });
    }
    Ok(())
}

/// Store-layer validation for connection create/update payloads (task 6.6,
/// Req 1.1): the v1 command-layer rules (host required, ≤253 chars, port
/// 1-65535) moved behind the IPC boundary — now enforced on UPDATE too,
/// flipping baseline defect B3-1 — plus a non-blank name. Username stays
/// UI-validated (baseline row 1.1 classification: backend parity kept).
fn validate_connection_input(input: &ConnectionInput) -> Result<(), LibraryStoreError> {
    if input.name.trim().is_empty() {
        return Err(LibraryStoreError::InvalidConnection {
            reason: "connection name cannot be empty".to_string(),
        });
    }
    if input.host.trim().is_empty() {
        return Err(LibraryStoreError::InvalidConnection {
            reason: "Host cannot be empty".to_string(),
        });
    }
    if input.host.len() > 253 {
        return Err(LibraryStoreError::InvalidConnection {
            reason: "Host exceeds maximum length of 253 characters".to_string(),
        });
    }
    if input.port == 0 {
        return Err(LibraryStoreError::InvalidConnection {
            reason: "Port must be in range 1-65535".to_string(),
        });
    }
    Ok(())
}

/// Mock policy for NEW records (Req 12.10): `false` passes through;
/// `true` is admitted ONLY in test-support builds (harness seeding). In
/// production the allow-path is not compiled in — mirroring
/// `rdp::session::admit_mock_session` — so no production write path can
/// set the flag, regardless of the payload.
fn resolve_mock_flag(requested: bool) -> Result<bool, LibraryStoreError> {
    if !requested {
        return Ok(false);
    }
    #[cfg(feature = "test-support")]
    {
        // Test-support build: synthetic saved connections are seedable.
        Ok(true)
    }
    #[cfg(not(feature = "test-support"))]
    {
        // Production build: reject the write before any transaction runs.
        Err(LibraryStoreError::InvalidConnection {
            reason: "mock connections can only be created in test-support builds; this \
                     production build rejects the request"
                .to_string(),
        })
    }
}

/// Mock policy for UPDATE (Req 12.10): test-support builds keep the v1
/// replace semantics (the payload flag applies); production builds never
/// rewrite the persisted flag through update — persisted mock data stays
/// data-only (inert at the connect boundary) until it is deleted.
fn merge_update_mock_flag(existing: bool, requested: bool) -> bool {
    if cfg!(feature = "test-support") {
        requested
    } else {
        existing
    }
}

/// Mock policy for DUPLICATE (Req 12.10): test-support builds copy the
/// source flag; production builds never mint a NEW mock record, so a
/// duplicate of legacy mock data lands as inert non-mock data.
fn duplicate_mock_flag(source: bool) -> bool {
    if cfg!(feature = "test-support") {
        source
    } else {
        false
    }
}

/// Deterministic available duplicate name (design "Destructive actions and
/// identity", Req 10.19): "Name copy", then "Name copy 2", "Name copy 3",
/// … — the first candidate not taken among the destination scope's
/// siblings (same library AND same folder as the source).
fn available_copy_name(
    base: &str,
    connections: &[ConnectionConfig],
    library_id: &str,
    folder_id: Option<&str>,
) -> String {
    let taken: HashSet<&str> = connections
        .iter()
        .filter(|connection| {
            connection.library_id == library_id && connection.folder_id.as_deref() == folder_id
        })
        .map(|connection| connection.name.as_str())
        .collect();
    let first = format!("{base} copy");
    if !taken.contains(first.as_str()) {
        return first;
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base} copy {suffix}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
        suffix += 1;
    }
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

    /// OS-independent backup-creation failure injection (CI run
    /// 43587595668 lesson): the CI Linux container runs as root, so the
    /// previous chmod read-only-directory injection could NOT fail backup
    /// creation there; the FAIL_NEXT_BACKUP seam fails it deterministically
    /// on every OS. Migration must block BEFORE any v2 commit.
    #[test]
    fn backup_creation_failure_blocks_migration_before_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        write_file(&path, V1_GOLDEN);

        FAIL_NEXT_BACKUP.store(true, std::sync::atomic::Ordering::SeqCst);
        let result = load_or_migrate(&path);

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
        let path = dir.path().join("connections.json");

        // Phase 1: a successful mutation establishes the prior snapshot
        // in memory and on disk (also proves a missing file opens Ready
        // and commits through the normal path).
        let store = LibraryStore::open(&path);
        store.create_library("First").unwrap();
        let before_tree = store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        // Phase 2: the NEXT commit fails deterministically after
        // validation. The seam is OS-independent and root-proof: the CI
        // Linux container runs as root, so the previous ENOTDIR /
        // permission injections did not fail there (CI run 43587595668).
        store.fail_next_commit();
        let err = store.create_library("Work").unwrap_err();
        assert!(
            matches!(err, LibraryStoreError::PersistFailed { .. }),
            "got {err:?}"
        );

        // In-memory snapshot and disk are untouched by the failed mutation.
        assert_eq!(store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
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

#[cfg(test)]
mod move_tests {
    //! Task 6.5 (Req 10.8/10.15): atomic cross-library tree moves through
    //! [`LibraryStore::move_node`] — one serialized snapshot → change →
    //! validate → persist → publish transaction (design "Storage
    //! transactions and recovery"). Valid folder moves re-parent the
    //! subtree and rewrite every descendant `library_id` when crossing
    //! libraries; connection moves change only `library_id`/`folder_id`,
    //! preserving ids, settings and timestamps (credentials live in the
    //! keyring, which the move path never touches — structural: no
    //! keyring call exists between `move_node` and `mutate`). Every
    //! rejection — cycles (own subtree/self), missing node/target,
    //! library sources, connection targets, write-blocked recovery and
    //! failed persists — retains the previous in-memory snapshot and
    //! on-disk file (Req 10.15).

    use super::*;
    use crate::store::connections::ConnectionConfig;
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

    fn folder_of<'a>(tree: &'a LibraryTree, id: &str) -> &'a Folder {
        tree.folders
            .iter()
            .find(|folder| folder.id == id)
            .unwrap_or_else(|| panic!("folder {id} missing from tree"))
    }

    fn connection_of<'a>(tree: &'a LibraryTree, id: &str) -> &'a ConnectionSummary {
        tree.connections
            .iter()
            .find(|connection| connection.id == id)
            .unwrap_or_else(|| panic!("connection {id} missing from tree"))
    }

    fn connection_entity(store: &LibraryStore, id: &str) -> ConnectionConfig {
        store
            .document()
            .unwrap()
            .connections
            .into_iter()
            .find(|connection| connection.id == id)
            .unwrap_or_else(|| panic!("connection {id} missing from snapshot"))
    }

    #[test]
    fn same_library_folder_move_reparents_subtree_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let work = Library::new("Work");
        let p1 = Folder::new(&work.id, None, "P1");
        let p2 = Folder::new(&work.id, None, "P2");
        let nested = Folder::new(&work.id, Some(p1.id.clone()), "Nested");
        let c_nested = conn(&work.id, Some(&nested.id), "nested-conn");
        let c_root = conn(&work.id, None, "root-conn");
        store
            .save(LibraryDocument::new(
                vec![work.clone()],
                vec![p1.clone(), p2.clone(), nested.clone()],
                vec![c_nested.clone(), c_root.clone()],
            ))
            .unwrap();

        store
            .move_node(NodeId::folder(p1.id.clone()), NodeId::folder(p2.id.clone()))
            .unwrap();

        let tree = store.list_tree().unwrap();
        let moved = folder_of(&tree, &p1.id);
        assert_eq!(moved.parent_id.as_deref(), Some(p2.id.as_str()));
        assert_eq!(
            moved.library_id, work.id,
            "same-library move keeps the library"
        );
        let child = folder_of(&tree, &nested.id);
        assert_eq!(
            child.parent_id.as_deref(),
            Some(p1.id.as_str()),
            "descendant folder keeps its parent inside the moved subtree"
        );
        let summary = connection_of(&tree, &c_nested.id);
        assert_eq!(summary.library_id, work.id);
        assert_eq!(summary.folder_id.as_deref(), Some(nested.id.as_str()));
        // Nothing was created or lost: same ids, same counts.
        assert_eq!(tree.folders.len(), 3);
        assert_eq!(tree.connections.len(), 2);

        // Persisted atomically: a fresh store over the same file sees the move.
        let reopened = store_at(dir.path(), "connections.json");
        assert_eq!(reopened.list_tree().unwrap(), tree);
    }

    #[test]
    fn cross_library_folder_move_rewrites_all_descendant_library_ids() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let b = Library::new("B");
        // Depth-3 subtree rooted at f1 (f1 > f2 > f3) with connections at
        // the top and bottom of the subtree, plus library-A-root and
        // library-B entities that must NOT move.
        let f1 = Folder::new(&a.id, None, "F1");
        let f2 = Folder::new(&a.id, Some(f1.id.clone()), "F2");
        let f3 = Folder::new(&a.id, Some(f2.id.clone()), "F3");
        let c_f1 = conn(&a.id, Some(&f1.id), "direct-in-f1");
        let c_f3 = conn(&a.id, Some(&f3.id), "deep-in-f3");
        let c_root = conn(&a.id, None, "a-root-conn");
        let target = Folder::new(&b.id, None, "Target");
        let unrelated = Folder::new(&b.id, None, "Unrelated");
        let c_other = conn(&b.id, Some(&unrelated.id), "b-conn");
        store
            .save(LibraryDocument::new(
                vec![a.clone(), b.clone()],
                vec![
                    f1.clone(),
                    f2.clone(),
                    f3.clone(),
                    target.clone(),
                    unrelated.clone(),
                ],
                vec![c_f1.clone(), c_f3.clone(), c_root.clone(), c_other.clone()],
            ))
            .unwrap();

        store
            .move_node(
                NodeId::folder(f1.id.clone()),
                NodeId::folder(target.id.clone()),
            )
            .unwrap();

        let tree = store.list_tree().unwrap();
        // Every descendant folder (depth >= 3) follows into library B with
        // the subtree shape intact.
        for folder in [&f1, &f2, &f3] {
            assert_eq!(
                folder_of(&tree, &folder.id).library_id,
                b.id,
                "subtree folder {} now belongs to library B",
                folder.name
            );
        }
        assert_eq!(
            folder_of(&tree, &f1.id).parent_id.as_deref(),
            Some(target.id.as_str())
        );
        assert_eq!(
            folder_of(&tree, &f2.id).parent_id.as_deref(),
            Some(f1.id.as_str())
        );
        assert_eq!(
            folder_of(&tree, &f3.id).parent_id.as_deref(),
            Some(f2.id.as_str())
        );
        // Connections inside the subtree change library but keep folder ids.
        let s_top = connection_of(&tree, &c_f1.id);
        assert_eq!(s_top.library_id, b.id);
        assert_eq!(s_top.folder_id.as_deref(), Some(f1.id.as_str()));
        let s_deep = connection_of(&tree, &c_f3.id);
        assert_eq!(s_deep.library_id, b.id);
        assert_eq!(s_deep.folder_id.as_deref(), Some(f3.id.as_str()));
        // Ancestors-only scoping: library A's root connection and library
        // B's own entities are untouched.
        assert_eq!(connection_of(&tree, &c_root.id).library_id, a.id);
        assert_eq!(connection_of(&tree, &c_root.id).folder_id, None);
        assert_eq!(connection_of(&tree, &c_other.id).library_id, b.id);
        assert_eq!(folder_of(&tree, &unrelated.id).library_id, b.id);

        // Single-library ancestry still holds: B's dashboard now serves
        // the moved connections alongside its own.
        let mut b_names: Vec<String> = store
            .descendants(NodeId::library(b.id.clone()))
            .unwrap()
            .into_iter()
            .map(|summary| summary.name)
            .collect();
        b_names.sort();
        assert_eq!(b_names.join(","), "b-conn,deep-in-f3,direct-in-f1");

        // The moved document is the one persisted.
        let reopened = store_at(dir.path(), "connections.json");
        assert_eq!(reopened.list_tree().unwrap(), tree);
    }

    #[test]
    fn cross_library_connection_move_preserves_ids_and_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let b = Library::new("B");
        let folder = Folder::new(&a.id, None, "F");
        let target = Folder::new(&b.id, None, "T");
        let mut moved = conn(&a.id, Some(&folder.id), "guarded");
        moved.username = "operator".to_string();
        moved.domain = "CORP".to_string();
        moved.port = 3390;
        moved.tags = vec!["prod".to_string(), "tier-2".to_string()];
        moved.display_width = Some(2560);
        moved.display_height = Some(1440);
        store
            .save(LibraryDocument::new(
                vec![a.clone(), b.clone()],
                vec![folder.clone(), target.clone()],
                vec![moved.clone()],
            ))
            .unwrap();

        store
            .move_node(
                NodeId::connection(moved.id.clone()),
                NodeId::folder(target.id.clone()),
            )
            .unwrap();

        // Only libraryId/folderId change: the whole entity (id, name, host,
        // port, username, domain, tags, display settings, timestamps) is
        // preserved — settings and active-session references survive
        // because the connection id does (Req 10.8). Credentials live in
        // the keyring; the move path never touches them.
        let after = connection_entity(&store, &moved.id);
        let mut expected = serde_json::to_value(&moved).unwrap();
        if let serde_json::Value::Object(map) = &mut expected {
            map.insert("libraryId".to_string(), serde_json::json!(b.id));
            map.insert("folderId".to_string(), serde_json::json!(target.id));
        }
        assert_eq!(
            serde_json::to_value(&after).unwrap(),
            expected,
            "connection entity identical except libraryId/folderId"
        );

        // Moving onto a library re-files the connection at its root.
        store
            .move_node(
                NodeId::connection(moved.id.clone()),
                NodeId::library(b.id.clone()),
            )
            .unwrap();
        let at_root = connection_entity(&store, &moved.id);
        assert_eq!(at_root.library_id, b.id);
        assert_eq!(at_root.folder_id, None);

        let reopened = store_at(dir.path(), "connections.json");
        assert_eq!(
            connection_entity(&reopened, &moved.id).folder_id,
            None,
            "root re-filing persisted"
        );
    }

    #[test]
    fn folder_move_into_own_subtree_or_onto_itself_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let f1 = Folder::new(&a.id, None, "F1");
        let f2 = Folder::new(&a.id, Some(f1.id.clone()), "F2");
        let f3 = Folder::new(&a.id, Some(f2.id.clone()), "F3");
        let deep = conn(&a.id, Some(&f3.id), "deep");
        store
            .save(LibraryDocument::new(
                vec![a.clone()],
                vec![f1.clone(), f2.clone(), f3.clone()],
                vec![deep.clone()],
            ))
            .unwrap();

        let before_tree = store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        // Into its own descendant (three levels down) and onto itself.
        for target in [NodeId::folder(f3.id.clone()), NodeId::folder(f1.id.clone())] {
            assert!(
                matches!(
                    store.move_node(NodeId::folder(f1.id.clone()), target.clone()),
                    Err(LibraryStoreError::Invalid(LibraryError::FolderCycle { .. }))
                ),
                "moving F1 under {target:?} must be rejected as a cycle"
            );
        }

        // Rejections preserved the snapshot (memory) and the file (disk).
        assert_eq!(store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
    }

    #[test]
    fn missing_node_or_target_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let folder = Folder::new(&a.id, None, "F");
        let connection = conn(&a.id, Some(&folder.id), "c");
        store
            .save(LibraryDocument::new(
                vec![a.clone()],
                vec![folder.clone()],
                vec![connection.clone()],
            ))
            .unwrap();

        let before_tree = store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        let cases = [
            // Missing source folder / connection.
            (
                NodeId::folder(MISSING_ID),
                NodeId::folder(folder.id.clone()),
            ),
            (
                NodeId::connection(MISSING_ID),
                NodeId::library(a.id.clone()),
            ),
            // Missing folder / library target.
            (
                NodeId::folder(folder.id.clone()),
                NodeId::folder(MISSING_ID),
            ),
            (
                NodeId::connection(connection.id.clone()),
                NodeId::library(MISSING_ID),
            ),
        ];
        for (node, target) in cases {
            assert!(
                matches!(
                    store.move_node(node.clone(), target.clone()),
                    Err(LibraryStoreError::Invalid(LibraryError::UnknownNode { .. }))
                ),
                "move {node:?} -> {target:?} must reject the unknown node"
            );
        }

        assert_eq!(store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
    }

    #[test]
    fn library_sources_and_connection_targets_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let b = Library::new("B");
        let folder = Folder::new(&a.id, None, "F");
        let connection = conn(&a.id, Some(&folder.id), "c");
        store
            .save(LibraryDocument::new(
                vec![a.clone(), b.clone()],
                vec![folder.clone()],
                vec![connection.clone()],
            ))
            .unwrap();

        let before_tree = store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        // Libraries are tree roots, not movable sources (design L336).
        assert!(matches!(
            store.move_node(
                NodeId::library(a.id.clone()),
                NodeId::folder(folder.id.clone())
            ),
            Err(LibraryStoreError::Invalid(
                LibraryError::MoveSourceIsLibrary { .. }
            ))
        ));
        // Connections cannot receive moves.
        assert!(matches!(
            store.move_node(
                NodeId::folder(folder.id.clone()),
                NodeId::connection(connection.id.clone())
            ),
            Err(LibraryStoreError::Invalid(
                LibraryError::MoveTargetIsConnection { .. }
            ))
        ));

        assert_eq!(store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
    }

    #[test]
    fn persist_failure_rolls_back_memory_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let b = Library::new("B");
        let subtree = Folder::new(&a.id, None, "Subtree");
        let nested = Folder::new(&a.id, Some(subtree.id.clone()), "Nested");
        let moved_conn = conn(&a.id, Some(&nested.id), "c");
        let target = Folder::new(&b.id, None, "Target");
        store
            .save(LibraryDocument::new(
                vec![a.clone(), b.clone()],
                vec![subtree.clone(), nested.clone(), target.clone()],
                vec![moved_conn.clone()],
            ))
            .unwrap();

        let before_tree = store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        // Force the atomic persist to fail after validation. The seam is
        // OS-independent and root-proof (CI run 43587595668): permission
        // based injections cannot fail inside the root-run CI container.
        store.fail_next_commit();
        let err = store
            .move_node(
                NodeId::folder(subtree.id.clone()),
                NodeId::folder(target.id.clone()),
            )
            .unwrap_err();

        assert!(
            matches!(err, LibraryStoreError::PersistFailed { .. }),
            "got {err:?}"
        );
        // Rollback: the previous snapshot is retained in memory AND the
        // file keeps its exact bytes (Req 10.15 — previous persisted tree).
        assert_eq!(store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
    }

    #[test]
    fn write_blocked_store_rejects_moves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, MALFORMED).unwrap();
        let store = LibraryStore::open(&path);
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));

        assert!(matches!(
            store.move_node(NodeId::folder(MISSING_ID), NodeId::library(MISSING_ID)),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        // Nothing was written over the invalid file.
        assert_eq!(std::fs::read(&path).unwrap(), MALFORMED.as_bytes());
    }

    #[test]
    fn folder_move_to_library_root_refiles_subtree_parent_none() {
        // tasks.md Implementation Note (task 7.2): the direct folder →
        // library-root move (parent -> None + subtree `library_id` rewrite)
        // was the one unpinned branch of `apply_move` — folder deletion
        // (7.2) shares exactly this re-filing code path, so the pin lands
        // with it.
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let a = Library::new("A");
        let b = Library::new("B");
        let holder = Folder::new(&a.id, None, "Holder");
        let moving = Folder::new(&a.id, Some(holder.id.clone()), "Moving");
        let nested = Folder::new(&a.id, Some(moving.id.clone()), "Nested");
        let c_moving = conn(&a.id, Some(&moving.id), "mid");
        let c_nested = conn(&a.id, Some(&nested.id), "deep");
        store
            .save(LibraryDocument::new(
                vec![a.clone(), b.clone()],
                vec![holder.clone(), moving.clone(), nested.clone()],
                vec![c_moving.clone(), c_nested.clone()],
            ))
            .unwrap();

        // Same library: re-file `moving` at A's root.
        store
            .move_node(
                NodeId::folder(moving.id.clone()),
                NodeId::library(a.id.clone()),
            )
            .unwrap();
        let tree = store.list_tree().unwrap();
        let moved = folder_of(&tree, &moving.id);
        assert_eq!(
            moved.parent_id, None,
            "a library target means that library's root"
        );
        assert_eq!(moved.library_id, a.id);
        let child = folder_of(&tree, &nested.id);
        assert_eq!(
            child.parent_id.as_deref(),
            Some(moving.id.as_str()),
            "descendant folders keep their parent inside the moved subtree"
        );
        assert_eq!(child.library_id, a.id);
        // Connections stay filed INSIDE the moved subtree — only deletion
        // (7.2) re-files them at the root.
        let mid = connection_of(&tree, &c_moving.id);
        assert_eq!(mid.folder_id.as_deref(), Some(moving.id.as_str()));
        assert_eq!(mid.library_id, a.id);
        let deep = connection_of(&tree, &c_nested.id);
        assert_eq!(deep.folder_id.as_deref(), Some(nested.id.as_str()));
        assert_eq!(deep.library_id, a.id);

        // Cross library: the same direct-to-root move rewrites the whole
        // subtree's library ancestry onto B.
        store
            .move_node(
                NodeId::folder(moving.id.clone()),
                NodeId::library(b.id.clone()),
            )
            .unwrap();
        let tree = store.list_tree().unwrap();
        let moved = folder_of(&tree, &moving.id);
        assert_eq!(moved.parent_id, None);
        assert_eq!(moved.library_id, b.id);
        let child = folder_of(&tree, &nested.id);
        assert_eq!(
            child.library_id, b.id,
            "descendant folder follows across libraries"
        );
        assert_eq!(
            connection_of(&tree, &c_moving.id).library_id,
            b.id,
            "subtree connection follows its folder across libraries"
        );
        assert_eq!(connection_of(&tree, &c_nested.id).library_id, b.id);
        // The untouched folder stays filed in A.
        assert_eq!(folder_of(&tree, &holder.id).library_id, a.id);

        // Persisted atomically: a fresh store over the same file agrees.
        let reopened = store_at(dir.path(), "connections.json");
        assert_eq!(reopened.list_tree().unwrap(), tree);
    }
}

#[cfg(test)]
mod folder_deletion_tests {
    //! Task 7.2 (Req 10.16/10.18): folder deletion through
    //! [`LibraryStore::delete_folder`] — ONE serialized snapshot → change →
    //! validate → persist → publish transaction (design "Storage
    //! transactions and recovery" + "Destructive actions and identity").
    //! The folder and every nested folder disappear; every descendant
    //! connection — at any depth — is re-filed at the SAME library's root
    //! with its ID (and therefore its keyring account and active-session
    //! references) unchanged; nothing but folders is deleted. The returned
    //! [`CascadedIds`] states the exact counts and IDs for the 10.18
    //! confirmation surface. Credentials and sessions are structurally
    //! untouched on this path (no keyring call exists between
    //! `delete_folder` and `mutate`; the session manager is never
    //! addressed) — the cross-subsystem survival guarantees are pinned in
    //! `store::credentials::folder_deletion_tests` (keyring) and
    //! `commands::library::tests` (live sessions). Every rejection —
    //! unknown folder, write-blocked recovery, failed persist — retains
    //! the previous in-memory snapshot and on-disk file.

    use super::*;
    use crate::store::connections::ConnectionConfig;
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

    fn connection_entity(store: &LibraryStore, id: &str) -> ConnectionConfig {
        store
            .document()
            .unwrap()
            .connections
            .into_iter()
            .find(|connection| connection.id == id)
            .unwrap_or_else(|| panic!("connection {id} missing from snapshot"))
    }

    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    /// Shared fixture: library W with a three-level subtree
    /// (`f1 > f1a > f1a1`, connections at every level), an unrelated
    /// surviving folder `keep` in W, and a second library O with one
    /// folder and connection.
    struct Fixture {
        dir: tempfile::TempDir,
        store: LibraryStore,
        work: Library,
        f1: Folder,
        f1a: Folder,
        f1a1: Folder,
        keep: Folder,
        other: Library,
        fo: Folder,
        c_f1: ConnectionConfig,
        c_f1a: ConnectionConfig,
        c_f1a1: ConnectionConfig,
        c_root: ConnectionConfig,
        c_keep: ConnectionConfig,
        c_o: ConnectionConfig,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let work = Library::new("Work");
        let other = Library::new("Other");
        let f1 = Folder::new(&work.id, None, "F1");
        let f1a = Folder::new(&work.id, Some(f1.id.clone()), "F1a");
        let f1a1 = Folder::new(&work.id, Some(f1a.id.clone()), "F1a1");
        let keep = Folder::new(&work.id, None, "Keep");
        let fo = Folder::new(&other.id, None, "OtherRoot");
        let c_f1 = conn(&work.id, Some(&f1.id), "depth-1");
        let c_f1a = conn(&work.id, Some(&f1a.id), "depth-2");
        let c_f1a1 = conn(&work.id, Some(&f1a1.id), "depth-3");
        let c_root = conn(&work.id, None, "already-root");
        let c_keep = conn(&work.id, Some(&keep.id), "keeper");
        let c_o = conn(&other.id, Some(&fo.id), "other-lib");
        store
            .save(LibraryDocument::new(
                vec![work.clone(), other.clone()],
                vec![
                    f1.clone(),
                    f1a.clone(),
                    f1a1.clone(),
                    keep.clone(),
                    fo.clone(),
                ],
                vec![
                    c_f1.clone(),
                    c_f1a.clone(),
                    c_f1a1.clone(),
                    c_root.clone(),
                    c_keep.clone(),
                    c_o.clone(),
                ],
            ))
            .unwrap();
        Fixture {
            dir,
            store,
            work,
            f1,
            f1a,
            f1a1,
            keep,
            other,
            fo,
            c_f1,
            c_f1a,
            c_f1a1,
            c_root,
            c_keep,
            c_o,
        }
    }

    #[test]
    fn folder_deletion_removes_subtree_and_moves_connections_to_library_root() {
        let f = fixture();
        let cascaded = f.store.delete_folder(&f.f1.id).unwrap();

        // Exact counts and separated IDs (Req 10.18): three folders gone,
        // three connections moved, zero connections deleted.
        assert_eq!(cascaded.deleted_folder_ids.len(), 3);
        assert_eq!(
            sorted(cascaded.deleted_folder_ids),
            sorted(vec![f.f1.id.clone(), f.f1a.id.clone(), f.f1a1.id.clone()]),
            "the folder and every nested folder is removed"
        );
        assert_eq!(
            sorted(cascaded.moved_connection_ids),
            sorted(vec![
                f.c_f1.id.clone(),
                f.c_f1a.id.clone(),
                f.c_f1a1.id.clone()
            ]),
            "descendants at every depth move"
        );
        assert!(
            cascaded.deleted_connection_ids.is_empty(),
            "folder deletion never deletes connections (Req 10.16)"
        );

        // Tree shape: only the untouched folders survive, and no connection
        // was lost.
        let tree = f.store.list_tree().unwrap();
        assert_eq!(
            tree.folders.len(),
            2,
            "subtree folders are gone: {:?}",
            tree.folders
        );
        assert_eq!(tree.connections.len(), 6, "no connection was deleted");
        assert!(tree.folders.iter().any(|folder| folder.id == f.keep.id));
        assert!(tree.folders.iter().any(|folder| folder.id == f.fo.id));

        // Every descendant connection now sits at W's root — same library.
        for id in [&f.c_f1.id, &f.c_f1a.id, &f.c_f1a1.id] {
            let summary = tree
                .connections
                .iter()
                .find(|connection| &connection.id == id)
                .unwrap_or_else(|| panic!("connection {id} missing after deletion"));
            assert_eq!(summary.folder_id, None, "{id} re-filed at the library root");
            assert_eq!(
                summary.library_id, f.work.id,
                "{id} stays in the same library"
            );
        }

        // Untouched placements: an already-root connection, an unrelated
        // folder's connection, and the other library's subtree.
        let c_root = tree
            .connections
            .iter()
            .find(|connection| connection.id == f.c_root.id)
            .unwrap();
        assert_eq!(c_root.folder_id, None);
        let c_keep = tree
            .connections
            .iter()
            .find(|connection| connection.id == f.c_keep.id)
            .unwrap();
        assert_eq!(c_keep.folder_id.as_deref(), Some(f.keep.id.as_str()));
        let c_o = tree
            .connections
            .iter()
            .find(|connection| connection.id == f.c_o.id)
            .unwrap();
        assert_eq!(c_o.library_id, f.other.id);
        assert_eq!(c_o.folder_id.as_deref(), Some(f.fo.id.as_str()));

        // ONE transaction: the persisted file already reflects exactly the
        // post-deletion tree (a fresh store over the same file agrees).
        let reopened = store_at(f.dir.path(), "connections.json");
        assert_eq!(reopened.list_tree().unwrap(), tree);
    }

    #[test]
    fn moved_connections_keep_their_exact_entity_and_id() {
        let f = fixture();
        f.store.delete_folder(&f.f1.id).unwrap();

        for original in [&f.c_f1, &f.c_f1a, &f.c_f1a1] {
            let moved = connection_entity(&f.store, &original.id);
            assert_eq!(
                moved.id, original.id,
                "keyring accounts (service + connection id) and session references keep resolving"
            );
            let mut expected = original.clone();
            expected.folder_id = None; // the ONLY change
            assert_eq!(
                moved, expected,
                "ids, settings and timestamps are untouched by the move to the root"
            );
        }
    }

    #[test]
    fn deleting_an_empty_folder_reports_zero_moves() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let work = Library::new("Work");
        let empty = Folder::new(&work.id, None, "Empty");
        store
            .save(LibraryDocument::new(
                vec![work],
                vec![empty.clone()],
                vec![],
            ))
            .unwrap();

        let cascaded = store.delete_folder(&empty.id).unwrap();

        assert_eq!(cascaded.deleted_folder_ids, vec![empty.id.clone()]);
        assert!(
            cascaded.moved_connection_ids.is_empty(),
            "no descendant connections exist to move"
        );
        assert!(cascaded.deleted_connection_ids.is_empty());
        assert_eq!(store.list_tree().unwrap().folders.len(), 0);
    }

    #[test]
    fn failed_persist_retains_snapshot_and_file_exactly() {
        let f = fixture();
        let path = f.dir.path().join("connections.json");
        let before_tree = f.store.list_tree().unwrap();
        let before_bytes = std::fs::read(&path).unwrap();

        // Force the atomic persist to fail after validation. The seam is
        // OS-independent and root-proof (CI runs as root; permission-based
        // injections are banned).
        f.store.fail_next_commit();
        let err = f.store.delete_folder(&f.f1.id).unwrap_err();

        assert!(
            matches!(err, LibraryStoreError::PersistFailed { .. }),
            "got {err:?}"
        );
        // Rollback: the previous snapshot is retained in memory AND the
        // file keeps its exact bytes — same folder and connection counts,
        // same IDs, same placements.
        assert_eq!(f.store.list_tree().unwrap(), before_tree);
        assert_eq!(std::fs::read(&path).unwrap(), before_bytes);
    }

    #[test]
    fn write_blocked_store_rejects_folder_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, MALFORMED).unwrap();
        let store = LibraryStore::open(&path);
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));

        assert!(matches!(
            store.delete_folder(MISSING_ID),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        // Nothing was written over the invalid file.
        assert_eq!(std::fs::read(&path).unwrap(), MALFORMED.as_bytes());
    }

    #[test]
    fn deleting_an_unknown_folder_is_a_typed_error() {
        let f = fixture();
        let before_tree = f.store.list_tree().unwrap();

        assert_eq!(
            f.store.delete_folder(MISSING_ID),
            Err(LibraryStoreError::Invalid(LibraryError::UnknownNode {
                node: NodeId::folder(MISSING_ID)
            }))
        );
        assert_eq!(
            f.store.list_tree().unwrap(),
            before_tree,
            "a rejected deletion changes nothing"
        );
    }
}

#[cfg(test)]
mod library_deletion_tests {
    //! Task 7.3 (Req 10.2/10.17/10.18): library deletion through
    //! [`LibraryStore::delete_library`] — ONE serialized snapshot →
    //! change → validate → persist → publish transaction (design
    //! "Storage transactions and recovery" + "Destructive actions and
    //! identity"). The library, every folder filed in it (at any depth)
    //! and every connection filed in it (root or nested) disappear in
    //! that single transaction; other libraries survive untouched. The
    //! returned [`CascadedIds`] states the exact counts/IDs for the
    //! 10.18 confirmation surface, and
    //! [`LibraryStore::library_connection_ids`] is the preview the
    //! command coordination layer stops sessions against. Session-actor
    //! and keyring coordination over these primitives is pinned in
    //! `commands::deletion` tests. Every rejection — unknown library,
    //! write-blocked recovery, failed persist — retains the previous
    //! in-memory snapshot and on-disk file.

    use super::*;
    use crate::store::connections::ConnectionConfig;
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

    fn sorted(mut ids: Vec<String>) -> Vec<String> {
        ids.sort();
        ids
    }

    /// Shared fixture: library W with a nested subtree (`f1 > f1a`, a
    /// root connection and a nested connection) and a second library O
    /// with one folder and one connection that must survive.
    struct Fixture {
        dir: tempfile::TempDir,
        store: LibraryStore,
        work: Library,
        other: Library,
        f1: Folder,
        f1a: Folder,
        fo: Folder,
        c_root: ConnectionConfig,
        c_nested: ConnectionConfig,
        c_o: ConnectionConfig,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let work = Library::new("Work");
        let other = Library::new("Other");
        let f1 = Folder::new(&work.id, None, "F1");
        let f1a = Folder::new(&work.id, Some(f1.id.clone()), "F1a");
        let fo = Folder::new(&other.id, None, "OtherRoot");
        let c_root = conn(&work.id, None, "at-work-root");
        let c_nested = conn(&work.id, Some(&f1a.id), "nested");
        let c_o = conn(&other.id, Some(&fo.id), "other-lib");
        store
            .save(LibraryDocument::new(
                vec![work.clone(), other.clone()],
                vec![f1.clone(), f1a.clone(), fo.clone()],
                vec![c_root.clone(), c_nested.clone(), c_o.clone()],
            ))
            .unwrap();
        Fixture {
            dir,
            store,
            work,
            other,
            f1,
            f1a,
            fo,
            c_root,
            c_nested,
            c_o,
        }
    }

    fn reopened(f: &Fixture) -> LibraryStore {
        store_at(f.dir.path(), "connections.json")
    }

    #[test]
    fn library_deletion_cascades_folders_and_connections_in_one_transaction() {
        let f = fixture();

        let cascaded = f.store.delete_library(&f.work.id).unwrap();
        assert_eq!(
            sorted(cascaded.deleted_folder_ids),
            sorted(vec![f.f1.id.clone(), f.f1a.id.clone()])
        );
        assert_eq!(
            sorted(cascaded.deleted_connection_ids),
            sorted(vec![f.c_root.id.clone(), f.c_nested.id.clone()])
        );
        assert!(
            cascaded.moved_connection_ids.is_empty(),
            "library deletion never re-files connections: they are deleted"
        );

        // Only the surviving library remains, in the snapshot and on disk.
        let document = f.store.document().unwrap();
        let library_ids: Vec<String> = document
            .libraries
            .iter()
            .map(|library| library.id.clone())
            .collect();
        assert_eq!(library_ids, vec![f.other.id.clone()]);
        assert_eq!(document.folders, vec![f.fo.clone()]);
        assert_eq!(document.connections, vec![f.c_o.clone()]);

        let disk = reopened(&f).document().unwrap();
        assert_eq!(disk.libraries.len(), 1);
        assert_eq!(disk.connections, vec![f.c_o.clone()]);
    }

    #[test]
    fn preview_lists_every_connection_of_the_library_only() {
        let f = fixture();

        let preview = f.store.library_connection_ids(&f.work.id).unwrap();
        assert_eq!(
            sorted(preview),
            sorted(vec![f.c_root.id.clone(), f.c_nested.id.clone()])
        );
        assert_eq!(
            sorted(f.store.library_connection_ids(&f.other.id).unwrap()),
            vec![f.c_o.id.clone()]
        );
    }

    #[test]
    fn deleting_the_last_library_yields_a_valid_empty_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path(), "connections.json");
        let only = Library::new("Only");
        store
            .save(LibraryDocument::new(
                vec![only.clone()],
                Vec::new(),
                Vec::new(),
            ))
            .unwrap();

        let cascaded = store.delete_library(&only.id).unwrap();
        assert!(cascaded.deleted_folder_ids.is_empty());
        assert!(cascaded.deleted_connection_ids.is_empty());
        let document = store.document().unwrap();
        assert!(document.libraries.is_empty());
        assert!(
            store_at(dir.path(), "connections.json")
                .document()
                .unwrap()
                .libraries
                .is_empty()
        );
    }

    #[test]
    fn deleting_or_previewing_an_unknown_library_is_a_typed_error() {
        let f = fixture();
        let before_tree = f.store.list_tree().unwrap();

        assert_eq!(
            f.store.delete_library(MISSING_ID),
            Err(LibraryStoreError::Invalid(LibraryError::UnknownNode {
                node: NodeId::library(MISSING_ID)
            }))
        );
        assert_eq!(
            f.store.library_connection_ids(MISSING_ID),
            Err(LibraryStoreError::Invalid(LibraryError::UnknownNode {
                node: NodeId::library(MISSING_ID)
            }))
        );
        assert_eq!(
            f.store.list_tree().unwrap(),
            before_tree,
            "a rejected deletion changes nothing"
        );
    }

    #[test]
    fn failed_persist_retains_the_snapshot_and_the_on_disk_file() {
        let f = fixture();
        f.store.fail_next_commit();

        let error = f.store.delete_library(&f.work.id).unwrap_err();
        assert!(
            matches!(error, LibraryStoreError::PersistFailed { .. }),
            "injected commit failure surfaces as PersistFailed: {error:?}"
        );

        // In-memory snapshot retained…
        let document = f.store.document().unwrap();
        assert_eq!(document.libraries.len(), 2);
        assert_eq!(document.connections.len(), 3);
        // …and the on-disk file with it (restart validity after failure).
        let disk = reopened(&f).document().unwrap();
        assert_eq!(disk.libraries.len(), 2);
        assert_eq!(disk.connections.len(), 3);
    }

    #[test]
    fn write_blocked_store_rejects_the_deletion_and_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        std::fs::write(&path, MALFORMED).unwrap();
        let store = LibraryStore::open(&path);
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));

        assert!(matches!(
            store.delete_library(MISSING_ID),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.library_connection_ids(MISSING_ID),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        // Nothing was written over the invalid file.
        assert_eq!(std::fs::read(&path).unwrap(), MALFORMED.as_bytes());
    }
}

#[cfg(test)]
mod unification_tests {
    //! Task 6.6 (Req 10.10/10.12 + tasks.md Implementation Notes obligation):
    //! unify the transitional `libraries.json` (v2, tasks 6.4/6.5) with the
    //! live v1 `connections.json` onto ONE v2 `connections.json` store file
    //! (design "Logical Data Model", option A′). Every branch must preserve
    //! data: v1 connections merge INTO the existing v2 default library, the
    //! transitional file is never destroyed, and every failure fails closed
    //! into write-blocked recovery.

    use super::*;
    use std::fs;
    use std::path::Path;

    const V1_TWO_CONNECTIONS: &str = r##"[
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

    const V1_COLLIDING: &str = r#"[{"id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa","name":"Clash","host":"clash.invalid","port":3389,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]"#;

    const MALFORMED_JSON: &str = "{ not valid json ";

    /// A transitional `libraries.json` a 6.4/6.5 build could have written:
    /// one user library with a folder and one root connection.
    fn transitional_document() -> LibraryDocument {
        let library = Library::new("Work");
        let folder = Folder::new(&library.id, None, "Prod");
        let existing = ConnectionConfig {
            id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            name: "V2 native".to_string(),
            host: "v2-native.invalid".to_string(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_connected_at: None,
        };
        LibraryDocument::new(vec![library], vec![folder], vec![existing])
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn open_unified_at(dir: &Path) -> LibraryStore {
        LibraryStore::open_unified(dir.join("connections.json"), dir.join("libraries.json"))
    }

    #[test]
    fn fresh_install_with_neither_file_starts_ready_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_unified_at(dir.path());
        assert_eq!(store.status(), StoreStatus::Ready);
        assert!(!dir.path().join("connections.json").exists());
        assert!(!dir.path().join("libraries.json").exists());
        assert!(store.list_connections().unwrap().is_empty());

        // Ready for CRUD: the first mutation persists the v2 document and
        // lazily creates the default library (fresh stores have none).
        let created = store
            .create_connection(input("First", "first.invalid"))
            .unwrap();
        assert!(dir.path().join("connections.json").exists());
        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(created.library_id, tree.libraries[0].id);
        assert_eq!(created.folder_id, None);
    }

    #[test]
    fn v1_only_migrates_into_default_library_with_field_and_id_parity() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        write_file(&store_path, V1_TWO_CONNECTIONS);

        let store = open_unified_at(dir.path());
        assert_eq!(store.status(), StoreStatus::Ready);

        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(tree.libraries[0].name, DEFAULT_LIBRARY_NAME);
        assert_eq!(tree.connections.len(), 2);

        let alpha = store
            .get_connection("11111111-1111-4111-8111-111111111111")
            .unwrap()
            .expect("legacy id preserved");
        assert_eq!(alpha.name, "Alpha DC");
        assert_eq!(alpha.host, "alpha.invalid");
        assert_eq!(alpha.port, 3389);
        assert_eq!(alpha.username, "admin");
        assert_eq!(alpha.domain, "CORP");
        assert_eq!(alpha.color_accent, Some("#ff0000".to_string()));
        assert_eq!(alpha.tags, vec!["prod".to_string(), "dc".to_string()]);
        assert_eq!(alpha.display_width, Some(2560));
        assert_eq!(alpha.display_height, Some(1440));
        assert!(alpha.last_connected_at.is_some());
        assert_eq!(alpha.library_id, tree.libraries[0].id);
        assert_eq!(alpha.folder_id, None, "legacy connections land at the root");

        // Backup semantics are the 6.3 flow: original v1 bytes preserved.
        assert_eq!(
            fs::read_to_string(dir.path().join("connections.json.v1.bak")).unwrap(),
            V1_TWO_CONNECTIONS
        );
    }

    #[test]
    fn v1_plus_populated_transitional_merges_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, V1_TWO_CONNECTIONS);
        let base = transitional_document();
        let base_library_id = base.libraries[0].id.clone();
        let base_folder_id = base.folders[0].id.clone();
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&base).unwrap(),
        );

        let store = open_unified_at(dir.path());
        assert_eq!(store.status(), StoreStatus::Ready);

        // Both populations survive; ids stay unique.
        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1, "user library kept");
        assert_eq!(tree.libraries[0].id, base_library_id);
        assert_eq!(tree.folders.len(), 1, "user folder kept");
        assert_eq!(tree.folders[0].id, base_folder_id);
        let mut ids: Vec<&str> = tree.connections.iter().map(|c| c.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "11111111-1111-4111-8111-111111111111",
                "22222222-2222-4222-8222-222222222222",
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
            ]
        );

        // v1 connections merged into the (first, since unnamed) default
        // library root; the v2-native connection keeps its placement.
        let alpha = store
            .get_connection("11111111-1111-4111-8111-111111111111")
            .unwrap()
            .unwrap();
        assert_eq!(alpha.library_id, base_library_id);
        assert_eq!(alpha.folder_id, None);
        let native = store
            .get_connection("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
            .unwrap()
            .unwrap();
        assert_eq!(native.library_id, base_library_id);

        // The v1 bytes are backed up; the transitional file is untouched.
        assert_eq!(
            fs::read_to_string(dir.path().join("connections.json.v1.bak")).unwrap(),
            V1_TWO_CONNECTIONS
        );
        let merged_json = fs::read_to_string(&store_path).unwrap();
        assert!(merged_json.contains("\"version\": 2"));

        // Restart is deterministic: the now-v2 store file is authoritative
        // and the leftover transitional file no longer duplicates anything.
        let reopened = open_unified_at(dir.path());
        assert_eq!(reopened.status(), StoreStatus::Ready);
        assert_eq!(reopened.list_tree().unwrap(), tree);
    }

    #[test]
    fn merge_prefers_a_library_named_default_library() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, V1_TWO_CONNECTIONS);

        let named = Library::new(DEFAULT_LIBRARY_NAME);
        let other = Library::new("Work");
        let named_id = named.id.clone();
        let doc = LibraryDocument::new(vec![other, named], Vec::new(), Vec::new());
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&doc).unwrap(),
        );

        let store = open_unified_at(dir.path());
        let alpha = store
            .get_connection("11111111-1111-4111-8111-111111111111")
            .unwrap()
            .unwrap();
        assert_eq!(
            alpha.library_id, named_id,
            "a library named Default Library wins over library order"
        );
    }

    #[test]
    fn merge_with_an_empty_transitional_creates_the_default_library() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, V1_TWO_CONNECTIONS);
        write_file(
            &transitional_path,
            r#"{"version":2,"libraries":[],"folders":[],"connections":[]}"#,
        );

        let store = open_unified_at(dir.path());
        assert_eq!(store.status(), StoreStatus::Ready);
        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(tree.libraries[0].name, DEFAULT_LIBRARY_NAME);
        assert_eq!(tree.connections.len(), 2);
    }

    #[test]
    fn missing_store_file_with_populated_transitional_adopts_it() {
        let dir = tempfile::tempdir().unwrap();
        let transitional_path = dir.path().join("libraries.json");
        let base = transitional_document();
        let expected_tree = LibraryTree {
            libraries: base.libraries.clone(),
            folders: base.folders.clone(),
            connections: base.connections.iter().map(summary_of).collect(),
        };
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&base).unwrap(),
        );

        let store = open_unified_at(dir.path());
        assert_eq!(store.status(), StoreStatus::Ready);
        assert_eq!(store.list_tree().unwrap(), expected_tree);

        // Adoption writes the unified v2 store file; the transitional file
        // stays untouched as the preserved original (least destructive).
        let store_json = fs::read_to_string(dir.path().join("connections.json")).unwrap();
        assert!(store_json.contains("\"version\": 2"));
        assert_eq!(
            fs::read_to_string(&transitional_path).unwrap(),
            serde_json::to_string_pretty(&base).unwrap()
        );

        // Restart: the store file now exists as v2 and stays authoritative.
        let reopened = open_unified_at(dir.path());
        assert_eq!(reopened.status(), StoreStatus::Ready);
        assert_eq!(reopened.list_tree().unwrap(), expected_tree);
    }

    #[test]
    fn existing_v2_store_file_is_authoritative_over_a_leftover_transitional() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        let unified = transitional_document();
        write_file(
            &store_path,
            &serde_json::to_string_pretty(&unified).unwrap(),
        );
        // A stale transitional file (post-unification leftover) holding an
        // entity the unified store does not know: never re-merged, never
        // destroyed; the v2 store file wins.
        let stale = LibraryDocument::new(vec![Library::new("Stale")], Vec::new(), Vec::new());
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&stale).unwrap(),
        );
        let before = fs::read_to_string(&transitional_path).unwrap();

        let store = open_unified_at(dir.path());
        assert_eq!(store.status(), StoreStatus::Ready);
        let tree = store.list_tree().unwrap();
        assert_eq!(tree.libraries.len(), 1);
        assert_eq!(tree.libraries[0].id, unified.libraries[0].id);
        assert_eq!(
            fs::read_to_string(&transitional_path).unwrap(),
            before,
            "leftover transitional file untouched"
        );
    }

    #[test]
    fn malformed_store_file_fails_closed_into_write_blocked_even_with_transitional() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, MALFORMED_JSON);
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&transitional_document()).unwrap(),
        );

        let store = open_unified_at(dir.path());
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));
        // Nothing was written anywhere; the corrupt bytes are intact.
        assert_eq!(fs::read_to_string(&store_path).unwrap(), MALFORMED_JSON);
        assert!(!dir.path().join("connections.json.v1.bak").exists());
        // Every connection mutation is rejected (Req 10.12).
        assert!(matches!(
            store.create_connection(input("Blocked", "blocked.invalid")),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
    }

    #[test]
    fn v1_invalid_id_with_transitional_fails_closed_after_backup() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        let invalid_v1 = r#"[{"id":"legacy","name":"Legacy","host":"legacy.invalid","port":3389,"createdAt":"2026-01-01T00:00:00Z","updatedAt":"2026-01-01T00:00:00Z"}]"#;
        write_file(&store_path, invalid_v1);
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&transitional_document()).unwrap(),
        );

        let store = open_unified_at(dir.path());
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));
        // Fail-closed AFTER the no-clobber backup, before any store write.
        assert_eq!(
            fs::read_to_string(dir.path().join("connections.json.v1.bak")).unwrap(),
            invalid_v1
        );
        assert_eq!(
            fs::read_to_string(&store_path).unwrap(),
            invalid_v1,
            "store file untouched"
        );
    }

    #[test]
    fn id_collision_between_v1_and_transitional_fails_closed_without_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, V1_COLLIDING);
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&transitional_document()).unwrap(),
        );

        let store = open_unified_at(dir.path());
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));
        assert_eq!(
            fs::read_to_string(&store_path).unwrap(),
            V1_COLLIDING,
            "no clobbering write on collision"
        );
        assert!(
            !dir.path().join("connections.json.v1.bak").exists(),
            "collision is detected before the backup/commit flow"
        );
    }

    #[test]
    fn malformed_transitional_fails_closed_preserving_everything() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, V1_TWO_CONNECTIONS);
        write_file(&transitional_path, MALFORMED_JSON);

        let store = open_unified_at(dir.path());
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));
        assert_eq!(fs::read_to_string(&store_path).unwrap(), V1_TWO_CONNECTIONS);
        assert_eq!(
            fs::read_to_string(&transitional_path).unwrap(),
            MALFORMED_JSON
        );
        assert!(!dir.path().join("connections.json.v1.bak").exists());
    }

    #[test]
    fn interrupted_unification_is_restart_safe_on_both_sides_of_the_commit() {
        // Post-commit interruption: the merged v2 store file exists while
        // the transitional file is still in place (nothing else ran yet).
        // Restart must not re-merge or duplicate.
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("connections.json");
        let transitional_path = dir.path().join("libraries.json");
        write_file(&store_path, V1_TWO_CONNECTIONS);
        write_file(
            &transitional_path,
            &serde_json::to_string_pretty(&transitional_document()).unwrap(),
        );
        {
            let store = open_unified_at(dir.path());
            assert_eq!(store.status(), StoreStatus::Ready);
        } // "crash" after the commit
        let reopened = open_unified_at(dir.path());
        assert_eq!(reopened.status(), StoreStatus::Ready);
        assert_eq!(reopened.list_connections().unwrap().len(), 3);

        // Pre-commit interruption: nothing was written, so a later open
        // performs the (deterministic) merge exactly once.
        let dir2 = tempfile::tempdir().unwrap();
        let store_path2 = dir2.path().join("connections.json");
        let transitional_path2 = dir2.path().join("libraries.json");
        write_file(&store_path2, V1_TWO_CONNECTIONS);
        write_file(
            &transitional_path2,
            &serde_json::to_string_pretty(&transitional_document()).unwrap(),
        );
        let store2 = open_unified_at(dir2.path());
        assert_eq!(store2.status(), StoreStatus::Ready);
        assert_eq!(store2.list_connections().unwrap().len(), 3);
    }

    /// Test-only helper building a fully-defaulted input around name/host.
    fn input(name: &str, host: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.to_string(),
            host: host.to_string(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            mock: false,
            library_id: String::new(),
            folder_id: None,
            password: None,
        }
    }
}

#[cfg(test)]
mod connection_crud_tests {
    //! Task 6.6 (Req 1.1–1.5, 10.7, 10.19): connection CRUD, imports and
    //! duplication over the unified v2 store — the ported v1 ConnectionStore
    //! behavior (validation, round-trips, secret-free persistence, delete)
    //! plus v2 ownership (library/folder) and duplicate semantics (fresh
    //! id/name, same folder, never a copied keyring entry).

    use super::*;
    use std::fs;

    const MALFORMED: &str = "{ not a v2 document";

    pub(super) fn sample(name: &str, host: &str) -> ConnectionInput {
        ConnectionInput {
            name: name.to_string(),
            host: host.to_string(),
            port: 3389,
            username: String::new(),
            domain: String::new(),
            color_accent: None,
            tags: Vec::new(),
            display_width: None,
            display_height: None,
            mock: false,
            library_id: String::new(),
            folder_id: None,
            password: None,
        }
    }

    fn store_at(root: &std::path::Path) -> LibraryStore {
        LibraryStore::open_unified(root.join("connections.json"), root.join("libraries.json"))
    }

    #[test]
    fn create_and_list_round_trips_name_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        store
            .create_connection(sample("Zulu", "zulu.invalid"))
            .unwrap();
        store
            .create_connection(sample("Alpha", "alpha.invalid"))
            .unwrap();
        let list = store.list_connections().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Alpha", "list() stays name-sorted");
        assert_eq!(list[1].name, "Zulu");
    }

    #[test]
    fn create_and_get_round_trips_every_field() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let mut input = sample("MyConn", "myhost.invalid");
        input.port = 3390;
        input.username = "admin".into();
        input.domain = "DOM".into();
        input.color_accent = Some("#ff8800".into());
        input.tags = vec!["tag1".into()];
        input.display_width = Some(1920);
        input.display_height = Some(1080);

        let created = store.create_connection(input).unwrap();
        let fetched = store.get_connection(&created.id).unwrap().unwrap();
        assert_eq!(fetched.name, "MyConn");
        assert_eq!(fetched.host, "myhost.invalid");
        assert_eq!(fetched.port, 3390);
        assert_eq!(fetched.username, "admin");
        assert_eq!(fetched.domain, "DOM");
        assert_eq!(fetched.color_accent, Some("#ff8800".to_string()));
        assert_eq!(fetched.tags, vec!["tag1".to_string()]);
        assert_eq!(fetched.display_width, Some(1920));
        assert_eq!(fetched.display_height, Some(1080));
        assert_eq!(fetched, created);
        assert!(!fetched.mock, "production inputs persist mock=false");
    }

    #[test]
    fn create_validates_name_host_and_port() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());

        let blank_name = ConnectionInput {
            name: "  ".into(),
            ..sample("x", "host.invalid")
        };
        assert!(matches!(
            store.create_connection(blank_name),
            Err(LibraryStoreError::InvalidConnection { .. })
        ));

        let blank_host = sample("NoHost", "   ");
        assert!(matches!(
            store.create_connection(blank_host),
            Err(LibraryStoreError::InvalidConnection { .. })
        ));

        let overlong = sample("LongHost", &"h".repeat(254));
        assert!(matches!(
            store.create_connection(overlong),
            Err(LibraryStoreError::InvalidConnection { .. })
        ));

        let zero_port = ConnectionInput {
            port: 0,
            ..sample("ZeroPort", "zero.invalid")
        };
        assert!(matches!(
            store.create_connection(zero_port),
            Err(LibraryStoreError::InvalidConnection { .. })
        ));

        // Rejections leave nothing persisted.
        assert!(!dir.path().join("connections.json").exists());
        assert!(store.list_connections().unwrap().is_empty());
    }

    #[test]
    fn create_assigns_library_and_folder_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let library = store.create_library("Work").unwrap();
        let folder = store.create_folder(&library.id, None, "Prod").unwrap();

        let rooted = ConnectionInput {
            library_id: library.id.clone(),
            ..sample("Root", "root.invalid")
        };
        let created_root = store.create_connection(rooted).unwrap();
        assert_eq!(created_root.library_id, library.id);
        assert_eq!(created_root.folder_id, None);

        let filed = ConnectionInput {
            library_id: library.id.clone(),
            folder_id: Some(folder.id.clone()),
            ..sample("Filed", "filed.invalid")
        };
        let created_filed = store.create_connection(filed).unwrap();
        assert_eq!(created_filed.folder_id.as_deref(), Some(folder.id.as_str()));

        // A folder-only destination resolves the folder's library.
        let folder_only = ConnectionInput {
            folder_id: Some(folder.id.clone()),
            ..sample("FolderOnly", "folder-only.invalid")
        };
        let created_fo = store.create_connection(folder_only).unwrap();
        assert_eq!(created_fo.library_id, library.id);
        assert_eq!(created_fo.folder_id.as_deref(), Some(folder.id.as_str()));

        // Unknown library / folder and cross-library placements are typed
        // rejections that persist nothing.
        let unknown_library = ConnectionInput {
            library_id: "99999999-9999-4999-8999-999999999999".into(),
            ..sample("Ghost", "ghost.invalid")
        };
        assert!(matches!(
            store.create_connection(unknown_library),
            Err(LibraryStoreError::Invalid(
                LibraryError::UnknownLibrary { .. }
            ))
        ));
        let other = store.create_library("Other").unwrap();
        let cross = ConnectionInput {
            library_id: other.id.clone(),
            folder_id: Some(folder.id.clone()),
            ..sample("Cross", "cross.invalid")
        };
        assert!(matches!(
            store.create_connection(cross),
            Err(LibraryStoreError::Invalid(
                LibraryError::ConnectionLibraryMismatch { .. }
            ))
        ));
        assert_eq!(store.list_connections().unwrap().len(), 3);
    }

    #[test]
    fn update_replaces_editable_fields_and_preserves_identity_and_placement() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let library = store.create_library("Work").unwrap();
        let folder = store.create_folder(&library.id, None, "Prod").unwrap();
        let created = store
            .create_connection(ConnectionInput {
                library_id: library.id.clone(),
                folder_id: Some(folder.id.clone()),
                ..sample("Old", "old.invalid")
            })
            .unwrap();
        store.update_last_connected(&created.id).unwrap();

        let mut update = sample("New", "new-host.invalid");
        update.port = 3390;
        update.username = "newuser".into();
        let updated = store
            .update_connection(&created.id, update)
            .unwrap()
            .expect("connection exists");
        assert_eq!(updated.name, "New");
        assert_eq!(updated.host, "new-host.invalid");
        assert_eq!(updated.port, 3390);
        assert_eq!(updated.username, "newuser");
        // Identity, ownership and history are preserved: re-filing goes
        // through move_node (task 6.5), not update.
        assert_eq!(updated.id, created.id);
        assert_eq!(updated.created_at, created.created_at);
        assert_eq!(updated.library_id, library.id);
        assert_eq!(updated.folder_id.as_deref(), Some(folder.id.as_str()));
        assert!(updated.last_connected_at.is_some());

        // Ownership fields in the update payload are ignored, not applied.
        let move_attempt = ConnectionInput {
            library_id: "99999999-9999-4999-8999-999999999999".into(),
            folder_id: None,
            ..sample("Still New", "new-host.invalid")
        };
        let again = store
            .update_connection(&created.id, move_attempt)
            .unwrap()
            .unwrap();
        assert_eq!(again.library_id, library.id);
        assert_eq!(again.folder_id.as_deref(), Some(folder.id.as_str()));
    }

    /// Regression for baseline defect B3-1: the v1 update seam accepted
    /// empty name/host. Req 1.1 requires create AND edit validation, and
    /// the v2 store now rejects the submission (flips the pinned defect).
    #[test]
    fn update_rejects_blank_name_and_host_regression_b3_1() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let created = store
            .create_connection(sample("Valid", "valid.invalid"))
            .unwrap();

        let blank = sample("", "");
        assert!(matches!(
            store.update_connection(&created.id, blank),
            Err(LibraryStoreError::InvalidConnection { .. })
        ));
        let unchanged = store.get_connection(&created.id).unwrap().unwrap();
        assert_eq!(unchanged.name, "Valid");
        assert_eq!(unchanged.host, "valid.invalid");
    }

    #[test]
    fn update_of_missing_id_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        assert_eq!(
            store
                .update_connection(
                    "00000000-0000-4000-8000-000000000000",
                    sample("X", "x.invalid")
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn delete_removes_the_connection_and_rewrites_the_store_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let keep = store
            .create_connection(sample("Keep", "keep.invalid"))
            .unwrap();
        let drop_conn = store
            .create_connection(sample("Drop", "drop.invalid"))
            .unwrap();

        assert!(store.delete_connection(&drop_conn.id).unwrap());
        assert!(
            !store.delete_connection(&drop_conn.id).unwrap(),
            "second delete: already gone"
        );

        let raw = fs::read_to_string(dir.path().join("connections.json")).unwrap();
        assert!(!raw.contains(&drop_conn.id), "deleted id leaves the file");
        let reopened = store_at(dir.path());
        assert!(reopened.get_connection(&keep.id).unwrap().is_some());
        assert!(reopened.get_connection(&drop_conn.id).unwrap().is_none());
        assert_eq!(reopened.list_connections().unwrap().len(), 1);
    }

    #[test]
    fn crud_round_trip_preserves_fields_across_reload() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let mut rich = sample("Rich", "rich.invalid");
        rich.port = 3390;
        rich.username = "admin".into();
        rich.domain = "CORP".into();
        rich.color_accent = Some("#ff8800".into());
        rich.tags = vec!["prod".into(), "rdp".into()];
        rich.display_width = Some(2560);
        rich.display_height = Some(1440);
        let created = store.create_connection(rich).unwrap();
        store.update_last_connected(&created.id).unwrap();

        let reopened = store_at(dir.path());
        let reloaded = reopened.get_connection(&created.id).unwrap().unwrap();
        assert_eq!(reloaded.id, created.id);
        assert_eq!(reloaded.name, "Rich");
        assert_eq!(reloaded.host, "rich.invalid");
        assert_eq!(reloaded.port, 3390);
        assert_eq!(reloaded.username, "admin");
        assert_eq!(reloaded.domain, "CORP");
        assert_eq!(reloaded.color_accent, Some("#ff8800".to_string()));
        assert_eq!(reloaded.tags, vec!["prod".to_string(), "rdp".to_string()]);
        assert_eq!(reloaded.display_width, Some(2560));
        assert_eq!(reloaded.display_height, Some(1440));
        assert_eq!(reloaded.created_at, created.created_at);
        assert!(reloaded.last_connected_at.is_some());
        // v2 ownership survives the reload too.
        assert_eq!(reloaded.library_id, created.library_id);
    }

    #[test]
    fn created_store_file_is_a_v2_document_without_any_password_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let mut input = sample("SecretHolder", "vault.invalid");
        input.password = Some("Sup3rSecret-Vault!".into());
        let created = store.create_connection(input).unwrap();

        let raw = fs::read_to_string(dir.path().join("connections.json")).unwrap();
        assert!(
            !raw.contains("Sup3rSecret-Vault!"),
            "password value must never reach the store file (Req 1.2/6.1)"
        );
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["version"], 2, "unified store file is v2");
        let connections = value["connections"].as_array().unwrap();
        assert_eq!(connections.len(), 1);
        assert!(connections[0].get("password").is_none());
        assert_eq!(connections[0]["id"].as_str(), Some(created.id.as_str()));
    }

    /// Store seam of baseline defect B3-2: a password submitted on update
    /// never reaches the file. COVERAGE GAP (recorded per the round-1
    /// review): the keyring half of the fix — `update_connection` now
    /// calls `CredentialStore::store_password` after the commit — is
    /// unpinnable at unit level (command layer, no app harness; no numbered
    /// gap id exists yet) and stays covered by the commands convention +
    /// 12.x E2E. Recorded for the controller's baseline-coverage sweep.
    #[test]
    fn update_with_password_keeps_store_file_secret_free_defect_b3_2() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let created = store
            .create_connection(sample("Editable", "edit.invalid"))
            .unwrap();
        let mut updated = sample("Editable", "edit2.invalid");
        updated.password = Some("EditedSecret-2026!".into());
        store
            .update_connection(&created.id, updated)
            .unwrap()
            .unwrap();

        let raw = fs::read_to_string(dir.path().join("connections.json")).unwrap();
        assert!(!raw.contains("EditedSecret-2026!"));
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(value["connections"][0].get("password").is_none());
    }

    #[test]
    fn duplicate_copies_non_secret_fields_with_fresh_id_and_available_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let library = store.create_library("Work").unwrap();
        let folder = store.create_folder(&library.id, None, "Prod").unwrap();
        let mut input = ConnectionInput {
            library_id: library.id.clone(),
            folder_id: Some(folder.id.clone()),
            ..sample("Web", "web.invalid")
        };
        input.username = "svc".into();
        input.color_accent = Some("#00ff00".into());
        input.tags = vec!["prod".into()];
        input.display_width = Some(1920);
        let source = store.create_connection(input).unwrap();
        store.update_last_connected(&source.id).unwrap();

        let first = store.duplicate_connection(&source.id).unwrap();
        assert_ne!(first.id, source.id, "fresh UUID (Req 10.19)");
        assert_eq!(first.name, "Web copy");
        assert_eq!(first.host, "web.invalid");
        assert_eq!(first.port, source.port);
        assert_eq!(first.username, "svc");
        assert_eq!(first.color_accent, Some("#00ff00".to_string()));
        assert_eq!(first.tags, vec!["prod".to_string()]);
        assert_eq!(first.display_width, Some(1920));
        assert_eq!(first.library_id, library.id, "same library");
        assert_eq!(
            first.folder_id.as_deref(),
            Some(folder.id.as_str()),
            "duplicate lands in the same folder (Req 10.19)"
        );
        assert!(first.last_connected_at.is_none(), "history is not copied");
        assert!(first.created_at >= source.created_at, "fresh timestamps");

        // Deterministic name sequence: "Web copy" taken → "Web copy 2".
        let second = store.duplicate_connection(&source.id).unwrap();
        assert_eq!(second.name, "Web copy 2");
        let third = store.duplicate_connection(&source.id).unwrap();
        assert_eq!(third.name, "Web copy 3");

        // Name availability is scoped to the destination folder: a same
        // name in another scope does not extend the sequence.
        let other_folder = store.create_folder(&library.id, None, "QA").unwrap();
        store
            .move_node(
                NodeId::connection(second.id.clone()),
                NodeId::folder(other_folder.id.clone()),
            )
            .unwrap();
        let fourth = store.duplicate_connection(&source.id).unwrap();
        assert_eq!(
            fourth.name, "Web copy 2",
            "scope is the source folder's siblings"
        );

        // Never a copied credential: the store model holds no secret and
        // the persisted duplicate carries no password key.
        let raw = fs::read_to_string(dir.path().join("connections.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let duplicates: Vec<&serde_json::Value> = value["connections"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["id"].as_str() == Some(first.id.as_str()))
            .collect();
        assert_eq!(duplicates.len(), 1);
        assert!(duplicates[0].get("password").is_none());

        assert_eq!(store.list_connections().unwrap().len(), 5);
    }

    #[test]
    fn duplicate_of_unknown_id_is_a_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        assert!(matches!(
            store.duplicate_connection("00000000-0000-4000-8000-000000000000"),
            Err(LibraryStoreError::Invalid(LibraryError::UnknownNode { .. }))
        ));
    }

    #[test]
    fn write_blocked_store_rejects_every_connection_mutation_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.json");
        fs::write(&path, MALFORMED).unwrap();
        let store = store_at(dir.path());
        assert!(matches!(store.status(), StoreStatus::WriteBlocked { .. }));

        assert!(matches!(
            store.create_connection(sample("A", "a.invalid")),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.update_connection("x", sample("B", "b.invalid")),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.delete_connection("x"),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.duplicate_connection("x"),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.update_last_connected("x"),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.get_connection("x"),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert!(matches!(
            store.list_connections(),
            Err(LibraryStoreError::WriteBlocked { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), MALFORMED.as_bytes());
    }
}

/// Production-build mock policy (Req 12.10): no production write path may
/// set `mock` — create/import/duplicate requests carrying a mock flag are
/// rejected, update preserves the persisted flag only, and a duplicate of
/// legacy mock data lands as inert non-mock data.
#[cfg(test)]
#[cfg(not(feature = "test-support"))]
mod production_mock_inertness_tests {
    use super::connection_crud_tests::sample;

    use super::*;

    /// Open a unified store at a temp dir (same layout as the CRUD tests).
    fn store_at(root: &std::path::Path) -> LibraryStore {
        LibraryStore::open_unified(root.join("connections.json"), root.join("libraries.json"))
    }

    /// Seed one persisted (possibly mock-flagged) connection as a
    /// test-support build could have written it.
    fn seed_legacy_mock(dir: &std::path::Path, id: &str, mock: bool) -> LibraryStore {
        let store = store_at(dir);
        let library = Library::new("Work");
        let mut legacy = ConnectionConfig {
            id: id.to_string(),
            name: "Legacy".to_string(),
            host: "legacy.invalid".to_string(),
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
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_connected_at: None,
        };
        legacy.mock = mock;
        store
            .save(LibraryDocument::new(
                vec![library],
                Vec::new(),
                vec![legacy],
            ))
            .unwrap();
        store
    }

    #[test]
    fn create_rejects_a_mock_flag() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_at(dir.path());
        let mut input = sample("Mock", "mock.invalid");
        input.mock = true;
        let err = store
            .create_connection(input)
            .expect_err("production must reject mock creation requests");
        assert!(err.to_string().to_lowercase().contains("mock"));
    }

    #[test]
    fn update_preserves_a_persisted_mock_flag_without_applying_the_payload_flag() {
        let dir = tempfile::tempdir().unwrap();
        let store = seed_legacy_mock(dir.path(), "11111111-1111-4111-8111-111111111111", true);

        let mut update = sample("Renamed", "renamed.invalid");
        update.mock = false;
        let updated = store
            .update_connection("11111111-1111-4111-8111-111111111111", update)
            .unwrap()
            .unwrap();
        assert!(
            updated.mock,
            "update never rewrites the persisted flag in production (data stays data-only)"
        );
    }

    #[test]
    fn duplicate_of_legacy_mock_data_lands_inert() {
        let dir = tempfile::tempdir().unwrap();
        let store = seed_legacy_mock(dir.path(), "11111111-1111-4111-8111-111111111111", true);

        let duplicate = store
            .duplicate_connection("11111111-1111-4111-8111-111111111111")
            .unwrap();
        assert!(
            !duplicate.mock,
            "no production write path sets mock — the duplicate is inert data"
        );
    }
}

/// Test-support-build mock policy (Req 12.2/12.10): seeding synthetic saved
/// connections through the v2 store persists the mock flag (data-only; the
/// connect boundary stays the gate compiled in rdp::session).
#[cfg(test)]
#[cfg(feature = "test-support")]
mod test_support_mock_flag_tests {
    use super::connection_crud_tests::sample;

    use super::*;

    #[test]
    fn create_persists_the_mock_flag_for_harness_seeding() {
        let dir = tempfile::tempdir().unwrap();
        let store = LibraryStore::open_unified(
            dir.path().join("connections.json"),
            dir.path().join("libraries.json"),
        );
        let mut input = sample("E2E Mock 1", "drift-e2e-mock.invalid");
        input.mock = true;
        let created = store.create_connection(input).unwrap();
        assert!(created.mock);

        let reopened = LibraryStore::open_unified(
            dir.path().join("connections.json"),
            dir.path().join("libraries.json"),
        );
        assert!(reopened.get_connection(&created.id).unwrap().unwrap().mock);
    }
}
