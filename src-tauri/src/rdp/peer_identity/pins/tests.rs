use super::*;
use crate::store::connections::ConnectionInput;

fn input(host: &str) -> ConnectionInput {
    serde_json::from_value(serde_json::json!({ "name": "Fixture", "host": host, "port": 3391 }))
        .unwrap()
}

fn offer(
    pins: &PinStore,
    library: &LibraryStore,
    host: &Arc<ConnectionConfig>,
    key: u8,
) -> IdentityReview {
    let endpoint = ConnectionTarget::new(&host.host, host.port)
        .unwrap()
        .endpoint;
    pins.offer(
        library,
        host,
        ObservedPeer {
            endpoint,
            spki_sha256: [key; 32],
        },
    )
    .unwrap()
}

#[test]
fn rotation_displays_both_keys_and_cancel_expiry_and_write_failure_preserve_the_prior_pin() {
    let dir = tempfile::tempdir().unwrap();
    let lib = LibraryStore::open(dir.path().join("connections.json"));
    let id = lib.create_connection(input("Mixed.Case.Test.")).unwrap().id;
    let host = lib.select_host_for_review(&id).unwrap();
    let pins = PinStore::new(dir.path().join("server-pins.json"));
    let first = offer(&pins, &lib, &host, 0);
    assert_eq!(first.endpoint, "mixed.case.test:3391");
    assert_eq!(
        first.observed_fingerprint,
        "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
    );
    pins.confirm(&lib, first.selection).unwrap();
    let original = fs::read(&pins.path).unwrap();

    let changed = offer(&pins, &lib, &host, 1);
    assert_eq!(
        changed.previous_fingerprint.as_ref(),
        Some(&first.observed_fingerprint)
    );
    assert_ne!(changed.observed_fingerprint, first.observed_fingerprint);
    pins.cancel(changed.selection);
    assert!(pins.confirm(&lib, changed.selection).is_err());
    assert_eq!(fs::read(&pins.path).unwrap(), original);

    let expired = offer(&pins, &lib, &host, 1);
    pins.drafts
        .lock()
        .unwrap()
        .get_mut(&expired.selection)
        .unwrap()
        .expires = Instant::now();
    assert!(pins.confirm(&lib, expired.selection).is_err());
    let failed = offer(&pins, &lib, &host, 1);
    // Lock contention is deterministic on every supported OS, including root-run CI.
    let lock = fs::File::open(pins.path.with_extension("lock")).unwrap();
    lock.lock().unwrap();
    assert!(pins.confirm(&lib, failed.selection).is_err());
    drop(lock);
    assert_eq!(fs::read(&pins.path).unwrap(), original);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let failed = offer(&pins, &lib, &host, 1);
        // Permit readers but prevent the atomic replacement's delete/rename.
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&pins.path)
            .unwrap();
        assert!(pins.confirm(&lib, failed.selection).is_err());
        drop(reader);
        assert_eq!(fs::read(&pins.path).unwrap(), original);
    }
    let confirmed = offer(&pins, &lib, &host, 1);
    pins.confirm(&lib, confirmed.selection).unwrap();
    assert_eq!(
        pins.read().unwrap()[&confirmed.endpoint].spki_sha256,
        [1; 32]
    );
}

#[test]
fn stale_review_is_invalid_after_host_edits_deletion_reload_or_concurrent_repin() {
    let dir = tempfile::tempdir().unwrap();
    let lib = LibraryStore::open(dir.path().join("connections.json"));
    let id = lib.create_connection(input("[2001:0db8::1]")).unwrap().id;
    let pins = PinStore::new(dir.path().join("server-pins.json"));
    let host = lib.select_host_for_review(&id).unwrap();
    let stale = offer(&pins, &lib, &host, 9);
    assert_eq!(stale.endpoint, "[2001:db8::1]:3391");
    lib.update_connection(&id, input("changed.test")).unwrap();
    lib.update_connection(&id, input(&host.host)).unwrap();
    assert!(pins.confirm(&lib, stale.selection).is_err());
    let peer = ObservedPeer {
        endpoint: ConnectionTarget::new(&host.host, host.port)
            .unwrap()
            .endpoint,
        spki_sha256: [0; 32],
    };
    assert!(pins.offer(&lib, &host, peer).is_err()); // edit during discovery
    assert!(!pins.path.exists());

    let host = lib.select_host_for_review(&id).unwrap();
    let fresh = offer(&pins, &lib, &host, 2);
    let competing = PinStore::new(pins.path.clone());
    let rival = offer(&competing, &lib, &host, 3);
    // A change to a different host preserves the selected host's revision.
    lib.create_connection(input("another.test")).unwrap();
    pins.confirm(&lib, fresh.selection).unwrap();
    assert!(competing.confirm(&lib, rival.selection).is_err());
    assert_eq!(pins.read().unwrap()[&fresh.endpoint].spki_sha256, [2; 32]);
    let stale = offer(&pins, &lib, &host, 4);
    lib.reload().unwrap();
    assert!(pins.confirm(&lib, stale.selection).is_err());
    let host = lib.select_host_for_review(&id).unwrap();
    let removed = offer(&pins, &lib, &host, 5);
    lib.delete_connection(&id).unwrap();
    assert!(pins.confirm(&lib, removed.selection).is_err());
    assert_eq!(pins.read().unwrap()[&fresh.endpoint].spki_sha256, [2; 32]);
}

#[test]
fn second_owner_retarget_or_deletion_rejects_a_stale_confirmation_and_keeps_prior_pin() {
    for delete in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let open = || {
            LibraryStore::open_unified(
                dir.path().join("connections.json"),
                dir.path().join("libraries.json"),
            )
        };
        let reviewing = open();
        let id = reviewing
            .create_connection(input("original.test"))
            .unwrap()
            .id;
        let editing = open();
        let host = reviewing.select_host_for_review(&id).unwrap();
        let pins = PinStore::new(dir.path().join("server-pins.json"));
        let first = offer(&pins, &reviewing, &host, 1);
        editing.create_connection(input("unrelated.test")).unwrap();
        pins.confirm(&reviewing, first.selection).unwrap();
        let prior = fs::read(&pins.path).unwrap();
        let replacement = offer(&pins, &reviewing, &host, 2);
        if delete {
            editing.delete_connection(&id).unwrap();
        } else {
            editing
                .update_connection(&id, input("changed.test"))
                .unwrap();
        }
        assert!(
            pins.confirm(&reviewing, replacement.selection).is_err(),
            "stale native selection replaced a previously confirmed pin"
        );
        assert_eq!(fs::read(&pins.path).unwrap(), prior);
    }
}

#[test]
fn confirmed_revision_is_protected_until_the_pin_action_returns() {
    use std::sync::mpsc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    let open = || LibraryStore::open_unified(&path, dir.path().join("libraries.json"));
    let reviewing = open();
    let id = reviewing
        .create_connection(input("original.test"))
        .unwrap()
        .id;
    let editing = open();
    let host = reviewing.select_host_for_review(&id).unwrap();
    std::thread::scope(|scope| {
        let (start, go) = mpsc::channel();
        let (entered, ready) = mpsc::channel();
        let (finished, result) = mpsc::channel();
        scope.spawn(move || {
            go.recv().unwrap();
            entered.send(()).unwrap();
            finished
                .send(editing.update_connection(&id, input("changed.test")))
                .unwrap();
        });
        reviewing
            .with_reviewed_host(&host, || {
                start.send(()).unwrap();
                ready.recv().unwrap();
                assert!(matches!(
                    result.recv_timeout(Duration::from_millis(100)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ));
                #[cfg(windows)]
                assert!(fs::File::options().write(true).open(&path).is_err());
                Ok(())
            })
            .unwrap();
        let updated = result
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(updated.host, "changed.test");
    });
}

#[test]
fn invalid_private_file_cannot_be_treated_as_first_use_or_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let lib = LibraryStore::open(dir.path().join("connections.json"));
    let id = lib.create_connection(input("test.invalid")).unwrap().id;
    let host = lib.select_host_for_review(&id).unwrap();
    let pins = PinStore::new(dir.path().join("server-pins.json"));
    let draft = offer(&pins, &lib, &host, 1);
    fs::write(&pins.path, b"{").unwrap();
    let endpoint = ConnectionTarget::new(&host.host, host.port)
        .unwrap()
        .endpoint;
    assert!(pins.lookup(&endpoint).is_err());
    assert!(pins.confirm(&lib, draft.selection).is_err());
    assert_eq!(fs::read(&pins.path).unwrap(), b"{");
}
