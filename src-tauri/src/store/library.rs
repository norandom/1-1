//! Task 6.1 (Req 10.1/10.5/10.8): v2 library document model and tree
//! invariants for the Royal TS-style connection tree.
//!
//! Pure data + validation + query layer: flat entities with parent IDs
//! (library → folders → connections), no graph crate, database or ORM.
//! Persistence, recovery and migration are tasks 6.2/6.3; CRUD read models
//! and `move_node` transactions are 6.4/6.5 — both build on
//! [`LibraryDocument::validate`] so a snapshot is proven legal before it is
//! persisted or published.
//!
//! Credentials never live in this model: keyring entries are keyed by
//! connection id only (design "Data Models"), so no secret material here.

use crate::store::connections::ConnectionConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
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
