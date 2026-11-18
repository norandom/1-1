//! Task 3.1: grants persist, stay off on missing or malformed data, and are
//! withdrawn inside the library mutations that retarget or remove a host.
use super::*;
use crate::rdp::peer_identity::host_endpoint_key;
use crate::store::connections::{ConnectionConfig, ConnectionInput};
use crate::store::library::{LibraryStore, NodeId};
use std::sync::Arc;

fn grant(profile_id: &str, endpoint: &str) -> ShareGrant {
    ShareGrant {
        profile_id: profile_id.into(),
        endpoint: endpoint.into(),
        spki_sha256: [7; 32],
        root_path: r"D:\work\shared".into(),
        volume_id: 0x1234_5678,
        directory_file_id: [9; 16],
        share_name: "work".into(),
        mode: ShareMode::ReadOnly,
        revision: Uuid::new_v4(),
    }
}

fn input(name: &str, host: &str, port: u16, library_id: &str) -> ConnectionInput {
    serde_json::from_value(serde_json::json!({
        "name": name, "host": host, "port": port, "libraryId": library_id,
    }))
    .unwrap()
}

fn input_like(host: &ConnectionConfig) -> ConnectionInput {
    input(&host.name, &host.host, host.port, &host.library_id)
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    library: LibraryStore,
    grants: Arc<ShareGrantStore>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let library = LibraryStore::open(root.join("connections.json"));
        let grants = Arc::new(ShareGrantStore::new(root.join("share-grants.json")));
        library.attach_share_grants(Arc::clone(&grants));
        Self {
            _dir: dir,
            root,
            library,
            grants,
        }
    }

    fn reopen_grants(&self) -> ShareGrantStore {
        ShareGrantStore::new(self.root.join("share-grants.json"))
    }

    /// A saved host with a committed grant for its current endpoint.
    fn shared_host(&self, name: &str, host: &str) -> ConnectionConfig {
        let library = self
            .library
            .create_library(&format!("{name} library"))
            .unwrap();
        let saved = self
            .library
            .create_connection(input(name, host, 3389, &library.id))
            .unwrap();
        let endpoint = host_endpoint_key(&saved.host, saved.port).unwrap();
        self.grants.put(grant(&saved.id, &endpoint)).unwrap();
        saved
    }
}

#[test]
fn missing_file_means_no_grants_and_revoking_writes_nothing() {
    let fixture = Fixture::new();
    assert_eq!(fixture.grants.get("anything"), Ok(None));
    fixture.grants.revoke(["anything"]).unwrap();
    assert!(!fixture.root.join("share-grants.json").exists());
}

#[test]
fn grants_survive_restart_and_replace_per_profile() {
    let fixture = Fixture::new();
    let first = grant("host-a", "a.example:3389");
    fixture.grants.put(first.clone()).unwrap();
    assert_eq!(fixture.reopen_grants().get("host-a"), Ok(Some(first)));
    let renewed = ShareGrant {
        mode: ShareMode::ReadWrite,
        revision: Uuid::new_v4(),
        ..grant("host-a", "a.example:3389")
    };
    fixture.grants.put(renewed.clone()).unwrap();
    let reopened = fixture.reopen_grants();
    assert_eq!(reopened.get("host-a"), Ok(Some(renewed)));
    assert_eq!(reopened.all().unwrap().len(), 1, "one grant per saved host");
}

#[test]
fn malformed_or_inconsistent_data_leaves_sharing_off_and_is_never_extended() {
    let fixture = Fixture::new();
    let path = fixture.root.join("share-grants.json");
    let valid = serde_json::to_value(GrantFile {
        version: FILE_VERSION,
        grants: BTreeMap::from([("host-a".to_string(), grant("host-a", "a.example:3389"))]),
    })
    .unwrap();
    let mut wrong_version = valid.clone();
    wrong_version["version"] = 2.into();
    let mut wrong_key = valid.clone();
    wrong_key["grants"]["host-a"]["profileId"] = "host-b".into();
    let mut bad_name = valid.clone();
    bad_name["grants"]["host-a"]["shareName"] = "not portable".into();
    let mut unknown_field = valid.clone();
    unknown_field["grants"]["host-a"]["extra"] = true.into();
    for data in [
        b"{ not json".to_vec(),
        serde_json::to_vec(&wrong_version).unwrap(),
        serde_json::to_vec(&wrong_key).unwrap(),
        serde_json::to_vec(&bad_name).unwrap(),
        serde_json::to_vec(&unknown_field).unwrap(),
    ] {
        fs::write(&path, &data).unwrap();
        assert_eq!(fixture.grants.get("host-a"), Err(GrantError::Corrupt));
        assert_eq!(fixture.grants.all(), Err(GrantError::Corrupt));
        assert_eq!(
            fixture.grants.put(grant("host-c", "c.example:3389")),
            Err(GrantError::Corrupt)
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            data,
            "a malformed file is never rewritten by a new grant"
        );
    }
}

#[test]
fn revoking_a_malformed_file_sets_it_aside_so_no_grant_can_return() {
    let fixture = Fixture::new();
    let path = fixture.root.join("share-grants.json");
    fs::write(&path, b"{ not json").unwrap();
    fixture.grants.revoke(["host-a"]).unwrap();
    assert_eq!(fixture.grants.get("host-a"), Ok(None));
    assert_eq!(
        fs::read(fixture.root.join("share-grants.json.corrupt")).unwrap(),
        b"{ not json"
    );
}

#[test]
fn failed_saves_keep_the_previous_committed_state() {
    let fixture = Fixture::new();
    let first = grant("host-a", "a.example:3389");
    fixture.grants.put(first.clone()).unwrap();
    fixture.grants.fail_next_write();
    let changed = ShareGrant {
        mode: ShareMode::ReadWrite,
        ..first.clone()
    };
    assert_eq!(fixture.grants.put(changed), Err(GrantError::SaveFailed));
    assert_eq!(
        fixture.reopen_grants().get("host-a"),
        Ok(Some(first.clone()))
    );
    fixture.grants.fail_next_write();
    assert_eq!(
        fixture.grants.revoke(["host-a"]),
        Err(GrantError::SaveFailed)
    );
    assert_eq!(fixture.reopen_grants().get("host-a"), Ok(Some(first)));
    assert_eq!(
        fixture.grants.put(grant("", "a.example:3389")),
        Err(GrantError::Invalid)
    );
}

#[test]
fn name_edits_moves_and_same_endpoint_spellings_keep_the_grant() {
    let fixture = Fixture::new();
    let host = fixture.shared_host("Build", "build.example");
    let mut renamed = input_like(&host);
    renamed.name = "Build server".into();
    renamed.username = "operator".into();
    fixture
        .library
        .update_connection(&host.id, renamed)
        .unwrap();
    let mut respelled = input_like(&host);
    respelled.host = "BUILD.example.".into();
    fixture
        .library
        .update_connection(&host.id, respelled)
        .unwrap();
    let other = fixture.library.create_library("Elsewhere").unwrap();
    fixture
        .library
        .move_node(NodeId::connection(&host.id), NodeId::library(&other.id))
        .unwrap();
    assert!(fixture.grants.get(&host.id).unwrap().is_some());
}

#[test]
fn retargeting_withdraws_the_grant_and_changing_back_does_not_reactivate_it() {
    let fixture = Fixture::new();
    for (index, change) in ["host", "port"].into_iter().enumerate() {
        let host = fixture.shared_host(&format!("Target {index}"), "target.example");
        let mut away = input_like(&host);
        match change {
            "host" => away.host = "elsewhere.example".into(),
            _ => away.port = 3390,
        }
        fixture.library.update_connection(&host.id, away).unwrap();
        assert_eq!(
            fixture.grants.get(&host.id),
            Ok(None),
            "{change} change withdraws the grant"
        );
        fixture
            .library
            .update_connection(&host.id, input_like(&host))
            .unwrap();
        assert_eq!(
            fixture.reopen_grants().get(&host.id),
            Ok(None),
            "changing the {change} back needs fresh consent"
        );
    }
}

#[test]
fn a_failed_revoke_rejects_the_retarget_and_a_failed_commit_leaves_the_grant_withdrawn() {
    let fixture = Fixture::new();
    let host = fixture.shared_host("Guarded", "guarded.example");
    let mut away = input_like(&host);
    away.host = "elsewhere.example".into();

    fixture.grants.fail_next_write();
    assert!(
        fixture
            .library
            .update_connection(&host.id, away.clone())
            .is_err()
    );
    let unchanged = fixture.library.get_connection(&host.id).unwrap().unwrap();
    assert_eq!(unchanged.host, "guarded.example", "the edit did not commit");
    assert!(
        fixture.grants.get(&host.id).unwrap().is_some(),
        "the grant still matches the unchanged host"
    );

    fixture.library.fail_next_commit();
    assert!(fixture.library.update_connection(&host.id, away).is_err());
    let unchanged = fixture.library.get_connection(&host.id).unwrap().unwrap();
    assert_eq!(unchanged.host, "guarded.example");
    assert_eq!(
        fixture.grants.get(&host.id),
        Ok(None),
        "consent was withdrawn before the failed commit"
    );
}

#[test]
fn duplicates_and_imports_never_inherit_a_grant() {
    let fixture = Fixture::new();
    let host = fixture.shared_host("Original", "original.example");
    let copy = fixture.library.duplicate_connection(&host.id).unwrap();
    assert_ne!(copy.id, host.id);
    assert_eq!(fixture.grants.get(&copy.id), Ok(None));
    let imported = fixture
        .library
        .create_connection(input_like(&host))
        .unwrap();
    assert_eq!(fixture.grants.get(&imported.id), Ok(None));
    assert!(
        fixture.grants.get(&host.id).unwrap().is_some(),
        "the original keeps its grant"
    );
}

#[test]
fn single_and_cascaded_deletions_withdraw_exactly_the_removed_hosts() {
    let fixture = Fixture::new();
    let deleted = fixture.shared_host("Deleted", "deleted.example");
    let kept = fixture.shared_host("Kept", "kept.example");
    assert!(fixture.library.delete_connection(&deleted.id).unwrap());
    assert_eq!(fixture.grants.get(&deleted.id), Ok(None));

    let cascade = fixture.library.create_library("Cascade").unwrap();
    let folder = fixture
        .library
        .create_folder(&cascade.id, None, "Nested")
        .unwrap();
    let mut nested_input = input("Nested", "nested.example", 3389, &cascade.id);
    nested_input.folder_id = Some(folder.id.clone());
    let nested = fixture.library.create_connection(nested_input).unwrap();
    fixture
        .grants
        .put(grant(&nested.id, "nested.example:3389"))
        .unwrap();
    let removed = fixture.library.delete_library(&cascade.id).unwrap();
    assert_eq!(removed.deleted_connection_ids, vec![nested.id.clone()]);
    assert_eq!(fixture.grants.get(&nested.id), Ok(None));
    assert!(fixture.grants.get(&kept.id).unwrap().is_some());
}

#[test]
fn a_failed_revoke_keeps_the_host_when_deletion_cannot_withdraw_consent() {
    let fixture = Fixture::new();
    let host = fixture.shared_host("Undeletable", "undeletable.example");
    fixture.grants.fail_next_write();
    assert!(fixture.library.delete_connection(&host.id).is_err());
    assert!(fixture.library.get_connection(&host.id).unwrap().is_some());
    assert!(fixture.grants.get(&host.id).unwrap().is_some());
}

#[test]
fn folder_deletion_keeps_grants_of_hosts_moved_to_the_library_root() {
    let fixture = Fixture::new();
    let library = fixture.library.create_library("Moves").unwrap();
    let folder = fixture
        .library
        .create_folder(&library.id, None, "Gone")
        .unwrap();
    let mut filed = input("Filed", "filed.example", 3389, &library.id);
    filed.folder_id = Some(folder.id.clone());
    let host = fixture.library.create_connection(filed).unwrap();
    fixture
        .grants
        .put(grant(&host.id, "filed.example:3389"))
        .unwrap();
    let removed = fixture.library.delete_folder(&folder.id).unwrap();
    assert_eq!(removed.moved_connection_ids, vec![host.id.clone()]);
    assert!(fixture.grants.get(&host.id).unwrap().is_some());
}

#[test]
fn a_reload_withdraws_grants_of_hosts_that_vanished_or_now_point_elsewhere() {
    let fixture = Fixture::new();
    let kept = fixture.shared_host("Kept", "kept.example");
    let moved = fixture.shared_host("Moved", "moved.example");
    let vanished = fixture.shared_host("Vanished", "vanished.example");
    // Another instance rewrites the library file: one host retargeted, one removed.
    let path = fixture.root.join("connections.json");
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let connections = document["connections"].as_array_mut().unwrap();
    connections.retain(|c| c["id"] != vanished.id.as_str());
    for connection in connections.iter_mut() {
        if connection["id"] == moved.id.as_str() {
            connection["host"] = "other.example".into();
        }
    }
    fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    fixture.library.reload().unwrap();
    assert!(fixture.grants.get(&kept.id).unwrap().is_some());
    assert_eq!(fixture.grants.get(&moved.id), Ok(None));
    assert_eq!(fixture.grants.get(&vanished.id), Ok(None));
}

#[test]
fn portable_share_names_round_trip_as_rdp_dos_names() {
    for name in ["w", "work", "build_1", "src-7ab"] {
        assert!(is_portable_share_name(name), "{name}");
    }
    for name in ["", "toolong8", "space x", "dot.ted", "ümlaut"] {
        assert!(!is_portable_share_name(name), "{name}");
    }
}

#[test]
fn ipv6_spellings_share_an_endpoint_and_ports_stay_distinct() {
    assert_eq!(
        host_endpoint_key("[2001:db8::1]", 3389),
        host_endpoint_key("[2001:0db8:0:0::1]", 3389)
    );
    assert_eq!(
        host_endpoint_key("[2001:db8::1]", 3389).as_deref(),
        Some("[2001:db8::1]:3389")
    );
    assert_ne!(
        host_endpoint_key("[2001:db8::1]", 3389),
        host_endpoint_key("[2001:db8::1]", 3390)
    );
    assert_ne!(
        host_endpoint_key("a.example", 3389),
        host_endpoint_key("b.example", 3389)
    );
}

#[test]
fn attaching_at_startup_withdraws_grants_for_hosts_changed_while_closed() {
    let dir = tempfile::tempdir().unwrap();
    let library = LibraryStore::open(dir.path().join("connections.json"));
    let kept = library
        .create_connection(input("Kept", "kept.example", 3389, ""))
        .unwrap();
    let moved = library
        .create_connection(input("Moved", "moved.example", 3389, ""))
        .unwrap();
    let grants = Arc::new(ShareGrantStore::new(dir.path().join("share-grants.json")));
    grants.put(grant(&kept.id, "kept.example:3389")).unwrap();
    // Recorded for an address the host no longer has, and for a deleted host.
    grants.put(grant(&moved.id, "old.example:3389")).unwrap();
    grants
        .put(grant("deleted-host", "gone.example:3389"))
        .unwrap();
    library.attach_share_grants(Arc::clone(&grants));
    assert!(grants.get(&kept.id).unwrap().is_some());
    assert_eq!(grants.get(&moved.id), Ok(None));
    assert_eq!(grants.get("deleted-host"), Ok(None));
}
