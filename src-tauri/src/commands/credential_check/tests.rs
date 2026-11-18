use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[tokio::test]
async fn repeated_activation_shares_one_selected_host_check() {
    let checks = Arc::new(CredentialChecks::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let release = Arc::new(Notify::new());
    let (started, entered) = tokio::sync::oneshot::channel();
    let first = {
        let checks = checks.clone();
        let calls = calls.clone();
        let release = release.clone();
        tokio::spawn(async move {
            checks
                .check("saved-host", async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let _ = started.send(());
                    release.notified().await;
                    CredentialOutcome::Accepted
                })
                .await
        })
    };
    entered
        .await
        .expect("the selected saved-host probe must start");
    let second = checks.check("saved-host", async {
        panic!("a repeated activation must join the first exchange")
    });
    tokio::pin!(second);
    tokio::select! { biased;
        result = &mut second => panic!("second invocation finished before release: {result:?}"),
        () = tokio::task::yield_now() => {}
    }
    release.notify_one();
    let first = first.await.unwrap().unwrap();
    let second = second.await.unwrap();
    assert_eq!(first.revision, second.revision);
    assert_eq!(second.result, CredentialOutcome::Accepted);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

fn profile(
    port: u16,
    user: &str,
    secret: Option<&str>,
) -> crate::store::connections::ConnectionInput {
    serde_json::from_value(serde_json::json!({
        "name":"Local fixture","host":"127.0.0.1","port":port,"username":user,"password":secret
    }))
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn one_deadline_includes_blocking_lookup_and_discards_late_input() {
    late_lookup(false).await;
}

#[tokio::test(start_paused = true)]
async fn edit_discards_blocking_lookup_before_it_can_connect() {
    late_lookup(true).await;
}

async fn late_lookup(edit: bool) {
    let directory = tempfile::tempdir().unwrap();
    let peer = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    peer.set_nonblocking(true).unwrap();
    let store = LibraryStore::open(directory.path().join("connections.json"));
    let host = store
        .create_connection(profile(peer.local_addr().unwrap().port(), "local", None))
        .unwrap();
    let id = host.id.clone();
    let checks = Arc::new(CredentialChecks::default());
    let (entered, mut started) = oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let (finished, completed) = oneshot::channel();
    let pending_checks = checks.clone();
    let pending_id = id.clone();
    let work = tokio::spawn(async move {
        check_saved(
            &pending_checks,
            &pending_id,
            move || {
                let _ = entered.send(());
                blocked.recv().unwrap();
                let _ = finished.send(());
                Ok(SavedInputs {
                    host: Arc::new(host),
                    password: "fixture-only".into(),
                    pin: None,
                })
            },
            |_| true,
        )
        .await
    });
    // Keep virtual time still until the real blocking pool starts the lookup.
    while started.try_recv().is_err() {
        tokio::task::yield_now().await;
    }
    tokio::time::advance(CHECK_TIMEOUT - Duration::from_secs(1)).await;
    assert!(!work.is_finished());
    if edit {
        checks.edit(&id, async {}).await;
        assert!(work.await.unwrap().is_none());
    } else {
        tokio::time::advance(Duration::from_secs(1)).await;
        let result = work.await.unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({
                "connectionId": id, "revision": result.revision, "outcome":"unableToVerify", "reason":"timeout"
            })
        );
    }
    release.send(()).unwrap();
    completed.await.unwrap();
    tokio::task::yield_now().await;
    assert_eq!(
        peer.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the keyring fixture owns the test process's mock provider"
)]
async fn saving_identity_closes_old_socket_and_next_check_reads_effective_inputs() {
    use crate::commands::connection::update_saved_connection;
    use crate::store::credentials::tests::install_scripted;
    use tokio::{io::AsyncReadExt, net::TcpListener};

    let (_lock, _restore) = install_scripted(Arc::default(), false, None, None, None);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(LibraryStore::open(
        directory.path().join("connections.json"),
    ));
    let credentials = Arc::new(CredentialStore::new());
    let pins = Arc::new(PinStore::new(directory.path().join("pins.json")));
    let peer = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let host = store
        .create_connection(profile(peer.local_addr().unwrap().port(), "first", None))
        .unwrap();
    credentials
        .store_password(&host.id, "first fixture secret")
        .unwrap();
    let checks = Arc::new(CredentialChecks::default());
    let pending = {
        let (checks, store, credentials, pins, id) = (
            checks.clone(),
            store.clone(),
            credentials.clone(),
            pins.clone(),
            host.id.clone(),
        );
        tokio::spawn(async move {
            let selection = id.clone();
            let verification = store.clone();
            check_saved(
                &checks,
                &selection,
                move || read_saved_inputs(&id, &store, &credentials, &pins),
                move |host| verification.with_reviewed_host(host, || Ok(())).is_ok(),
            )
            .await
        })
    };
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(3), peer.accept())
        .await
        .unwrap()
        .unwrap();
    let mut header = [0; 4];
    socket.read_exact(&mut header).await.unwrap();
    let mut request = vec![0; usize::from(u16::from_be_bytes([header[2], header[3]])) - 4];
    socket.read_exact(&mut request).await.unwrap();
    let old_revision = checks.host(&host.id).revision.lock().unwrap().id;
    let new_port = if host.port == 3389 { 3390 } else { 3389 };
    let mut edit = profile(new_port, "second", Some("second fixture secret"));
    edit.domain = "fixture-domain".into();
    update_saved_connection(&host.id, edit, &store, &credentials, &checks)
        .await
        .unwrap();
    assert_eq!(socket.read(&mut [0; 1]).await.unwrap(), 0);
    assert!(pending.await.unwrap().is_none());
    let saved = read_saved_inputs(&host.id, &store, &credentials, &pins).unwrap();
    assert_eq!(saved.host.port, new_port);
    assert_eq!(saved.host.username, "second");
    assert_eq!(saved.host.domain, "fixture-domain");
    assert_eq!(saved.password, "second fixture secret");
    let next = checks
        .check(&host.id, async { CredentialOutcome::Accepted })
        .await
        .unwrap();
    assert_ne!(old_revision, next.revision);

    let mut secret_only = profile(new_port, "second", Some("different fixture secret"));
    secret_only.domain = saved.host.domain.clone();
    update_saved_connection(&host.id, secret_only, &store, &credentials, &checks)
        .await
        .unwrap();
    assert_ne!(
        checks.host(&host.id).revision.lock().unwrap().id,
        next.revision
    );
    assert_eq!(
        read_saved_inputs(&host.id, &store, &credentials, &pins)
            .unwrap()
            .password,
        "different fixture secret"
    );
}

#[tokio::test]
#[allow(
    clippy::await_holding_lock,
    reason = "the keyring fixture owns the test process's mock provider"
)]
async fn failed_keyring_save_reloads_committed_profile_and_effective_credential() {
    use crate::commands::connection::update_saved_connection;
    use crate::store::credentials::tests::{Fail, install_scripted};
    let (_lock, _restore) =
        install_scripted(Arc::default(), false, Some(Fail::Platform), None, None);
    let directory = tempfile::tempdir().unwrap();
    let store = LibraryStore::open(directory.path().join("connections.json"));
    let host = store
        .create_connection(profile(3389, "first", None))
        .unwrap();
    let checks = CredentialChecks::default();
    let credentials = CredentialStore::new();
    let before = checks
        .check(&host.id, async { CredentialOutcome::Accepted })
        .await
        .unwrap();
    let failed = update_saved_connection(
        &host.id,
        profile(3390, "second", Some("unsaved fixture secret")),
        &store,
        &credentials,
        &checks,
    )
    .await;
    assert!(failed.is_err());
    let actual = store.get_connection(&host.id).unwrap().unwrap();
    assert_eq!((actual.username.as_str(), actual.port), ("second", 3390));
    let pins = PinStore::new(directory.path().join("pins.json"));
    assert!(matches!(
        read_saved_inputs(&host.id, &store, &credentials, &pins),
        Err(ProbeReason::MissingCredential)
    ));
    assert_ne!(
        checks.host(&host.id).revision.lock().unwrap().id,
        before.revision
    );
}

#[tokio::test]
async fn cascade_and_reload_invalidate_only_their_pending_revisions() {
    let checks = Arc::new(CredentialChecks::default());
    let mut waiters = Vec::new();
    for id in ["removed", "surviving"] {
        let checks = checks.clone();
        let (started, entered) = oneshot::channel();
        waiters.push(tokio::spawn(async move {
            checks
                .check(id, async {
                    let _ = started.send(());
                    std::future::pending::<CredentialOutcome>().await
                })
                .await
        }));
        entered.await.unwrap();
    }
    let living = waiters.pop().unwrap();
    {
        let _replace = checks.library_gate.write().await;
        checks
            .publish_library(Some(&["removed".to_owned()]), async {})
            .await;
    }
    assert!(waiters.pop().unwrap().await.unwrap().is_none());
    assert!(!living.is_finished());
    {
        let _reload = checks.library_gate.write().await;
        checks.publish_library(None, async {}).await;
    }
    assert!(living.await.unwrap().is_none());
}

#[tokio::test]
async fn queued_click_cannot_authenticate_the_replacement_host() {
    let checks = CredentialChecks::default();
    let replace = checks.library_gate.write().await;
    let pending = checks.check("same-id", async {
        panic!("superseded request reached the exchange")
    });
    tokio::pin!(pending);
    tokio::select! { biased;
        result = &mut pending => panic!("blocked check finished before replacement: {result:?}"),
        () = tokio::task::yield_now() => {}
    }
    checks.publish_library(None, async {}).await;
    drop(replace);
    assert!(pending.await.is_none());
}

#[tokio::test]
async fn click_after_edit_invalidation_does_not_test_unpublished_identity() {
    click_during_publication(false).await;
}

#[tokio::test]
async fn click_after_reload_invalidation_does_not_test_unpublished_identity() {
    click_during_publication(true).await;
}

async fn click_during_publication(reload: bool) {
    let checks = Arc::new(CredentialChecks::default());
    let _previous = checks
        .check("saved", async { CredentialOutcome::Rejected })
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let (started, ready) = oneshot::channel();
    let (release, publication) = oneshot::channel();
    let writer = {
        let checks = checks.clone();
        tokio::spawn(async move {
            let change = async {
                started.send(()).unwrap();
                publication.await.unwrap();
            };
            if reload {
                let _reload = checks.library_gate.write().await;
                checks.publish_library(None, change).await;
            } else {
                checks.edit("saved", change).await;
            }
        })
    };
    ready.await.unwrap();
    let selected = checks.check("saved", {
        let calls = calls.clone();
        async move {
            calls.fetch_add(1, Ordering::SeqCst);
            CredentialOutcome::Accepted
        }
    });
    tokio::pin!(selected);
    let early = tokio::select! { biased;
        result = &mut selected => Some(result),
        () = tokio::task::yield_now() => None,
    };
    release.send(()).unwrap();
    writer.await.unwrap();
    let result = match early {
        Some(result) => result,
        None => selected.await,
    };
    assert!(
        result.is_none(),
        "click during publication tested the new host"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let recorded = calls.clone();
    assert_eq!(
        checks
            .check("saved", async move {
                recorded.fetch_add(1, Ordering::SeqCst);
                CredentialOutcome::Accepted
            })
            .await
            .unwrap()
            .result,
        CredentialOutcome::Accepted
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
