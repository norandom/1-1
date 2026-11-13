use keyring::Entry;

const SERVICE_NAME: &str = "com.rdpclient.app";

/// Task 5.3 (Req 12.7): reserved prefix for harness-owned credential
/// namespaces in test-support builds. A namespaced service can never equal
/// the production [`SERVICE_NAME`].
#[cfg(feature = "test-support")]
const TEST_SERVICE_PREFIX: &str = "com.rdpclient.app.test.";

pub struct CredentialStore {
    service: String,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self {
            service: SERVICE_NAME.to_string(),
        }
    }

    fn entry(&self, connection_id: &str) -> Result<Entry, String> {
        Entry::new(&self.service, connection_id)
            .map_err(|e| format!("Failed to create keyring entry: {}", e))
    }

    pub fn store_password(&self, connection_id: &str, password: &str) -> Result<(), String> {
        self.entry(connection_id)?
            .set_password(password)
            .map_err(|e| format!("Failed to store password in keychain: {}", e))?;
        Ok(())
    }

    pub fn get_password(&self, connection_id: &str) -> Result<Option<String>, String> {
        match self.entry(connection_id)?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to retrieve password from keychain: {}", e)),
        }
    }

    pub fn delete_password(&self, connection_id: &str) -> Result<(), String> {
        match self.entry(connection_id)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()), // Already gone, that's fine
            Err(e) => Err(format!("Failed to delete password from keychain: {}", e)),
        }
    }

    /// Task 5.3 (Req 12.7): a harness-owned namespace. Exists only in
    /// test-support builds; the service can never collide with user data.
    #[cfg(feature = "test-support")]
    pub fn for_namespace(namespace: &str) -> Self {
        Self {
            service: format!("{TEST_SERVICE_PREFIX}{namespace}"),
        }
    }

    /// The keyring service this store reads/writes. Test-support builds only.
    #[cfg(feature = "test-support")]
    pub fn service_name(&self) -> &str {
        &self.service
    }

    /// Whether scoped cleanup is allowed on this store (harness namespace
    /// only, never the production service). Test-support builds only.
    #[cfg(feature = "test-support")]
    pub fn is_test_namespaced(&self) -> bool {
        self.service.starts_with(TEST_SERVICE_PREFIX)
    }

    /// Task 5.3 (Req 12.7): delete only this harness namespace's entries for
    /// the given connection ids — never user data. Refuses to run against a
    /// production (non-namespaced) store. Returns how many entries actually
    /// existed and were removed.
    #[cfg(feature = "test-support")]
    pub fn delete_test_credentials(&self, connection_ids: &[String]) -> Result<usize, String> {
        if !self.is_test_namespaced() {
            return Err(
                "Refusing to clean credentials: this store is not a harness-namespaced (test-support) store"
                    .to_string(),
            );
        }
        let mut removed = 0usize;
        for id in connection_ids {
            if self.get_password(id)?.is_some() {
                self.delete_password(id)?;
                removed += 1;
            }
        }
        Ok(removed)
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring::credential::{Credential, CredentialApi, CredentialBuilder, CredentialBuilderApi};
    use keyring::{mock, set_default_credential_builder};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, MutexGuard};

    // ---- Task 3.1 characterization tests (persistence baseline, Req 1.2/1.5/6.2).
    //
    // The OS keyring is unavailable in CI containers and must not be touched from
    // tests (Req 12.7 isolation). keyring 3.6.3 exposes a public, unsealed
    // `CredentialBuilderApi`, so these tests swap the process-wide default builder
    // via `set_default_credential_builder` (keyring's own client-store hook) with a
    // scripted in-memory backend. nextest runs every test in its own process; the
    // extra mutex + restore-to-mock guard keeps plain `cargo test` (shared process)
    // deterministic as well. No production seam was added for this.
    //
    // Task 5.1 exception (Req 8.2/12.7): on Windows the real Credential Manager
    // IS available on every tier host, so the two task-5.1 round-trip tests
    // exercise the real vault through isolated, self-cleaned entries. ----

    fn keyring_lock() -> &'static Mutex<()> {
        static LOCK: Mutex<()> = Mutex::new(());
        &LOCK
    }

    /// Restores the keyring mock builder on drop so a later test in the same
    /// process never inherits a scripted/failing builder.
    struct BuilderGuard;
    impl BuilderGuard {
        fn install(builder: Box<CredentialBuilder>) -> Self {
            set_default_credential_builder(builder);
            BuilderGuard
        }
    }
    impl Drop for BuilderGuard {
        fn drop(&mut self) {
            set_default_credential_builder(mock::default_credential_builder());
        }
    }

    /// Which failure a scripted keyring operation returns.
    #[derive(Clone, Copy)]
    enum Fail {
        NoEntry,
        Platform,
    }

    fn err(f: Fail) -> keyring::Error {
        match f {
            Fail::NoEntry => keyring::Error::NoEntry,
            Fail::Platform => keyring::Error::PlatformFailure("scripted keyring outage".into()),
        }
    }

    type SharedStore = Arc<Mutex<HashMap<(String, String), Vec<u8>>>>;

    /// Scripted keyring backend: a shared (service, user) -> secret map plus
    /// injectable build/operation failures. Behaves like a real OS store across
    /// Entry instances (CredentialStore creates a fresh Entry per call).
    struct ScriptedBuilder {
        store: SharedStore,
        seen: Arc<Mutex<Vec<(String, String)>>>,
        fail_build: bool,
        fail_set: Option<Fail>,
        fail_get: Option<Fail>,
        fail_delete: Option<Fail>,
    }

    impl CredentialBuilderApi for ScriptedBuilder {
        fn build(
            &self,
            _target: Option<&str>,
            service: &str,
            user: &str,
        ) -> keyring::Result<Box<Credential>> {
            self.seen
                .lock()
                .unwrap()
                .push((service.to_string(), user.to_string()));
            if self.fail_build {
                return Err(err(Fail::Platform));
            }
            Ok(Box::new(ScriptedCredential {
                store: self.store.clone(),
                key: (service.to_string(), user.to_string()),
                fail_set: self.fail_set,
                fail_get: self.fail_get,
                fail_delete: self.fail_delete,
            }))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    struct ScriptedCredential {
        store: SharedStore,
        key: (String, String),
        fail_set: Option<Fail>,
        fail_get: Option<Fail>,
        fail_delete: Option<Fail>,
    }

    impl CredentialApi for ScriptedCredential {
        fn set_secret(&self, password: &[u8]) -> keyring::Result<()> {
            if let Some(f) = self.fail_set {
                return Err(err(f));
            }
            self.store
                .lock()
                .unwrap()
                .insert(self.key.clone(), password.to_vec());
            Ok(())
        }
        fn get_secret(&self) -> keyring::Result<Vec<u8>> {
            if let Some(f) = self.fail_get {
                return Err(err(f));
            }
            self.store
                .lock()
                .unwrap()
                .get(&self.key)
                .cloned()
                .ok_or(keyring::Error::NoEntry)
        }
        fn delete_credential(&self) -> keyring::Result<()> {
            if let Some(f) = self.fail_delete {
                return Err(err(f));
            }
            match self.store.lock().unwrap().remove(&self.key) {
                Some(_) => Ok(()),
                None => Err(keyring::Error::NoEntry),
            }
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn install_scripted(
        seen: Arc<Mutex<Vec<(String, String)>>>,
        fail_build: bool,
        fail_set: Option<Fail>,
        fail_get: Option<Fail>,
        fail_delete: Option<Fail>,
    ) -> (MutexGuard<'static, ()>, BuilderGuard) {
        let lock = keyring_lock().lock().unwrap();
        let guard = BuilderGuard::install(Box::new(ScriptedBuilder {
            store: Arc::new(Mutex::new(HashMap::new())),
            seen,
            fail_build,
            fail_set,
            fail_get,
            fail_delete,
        }));
        (lock, guard)
    }

    #[test]
    fn test_round_trip_through_scripted_keyring() {
        let (_lock, _guard) =
            install_scripted(Arc::new(Mutex::new(Vec::new())), false, None, None, None);
        let store = CredentialStore::new();
        let id = "char-3-1-roundtrip";

        store.store_password(id, "s3cret!").unwrap();
        assert_eq!(store.get_password(id).unwrap(), Some("s3cret!".to_string()));
        store.delete_password(id).unwrap();
        assert_eq!(
            store.get_password(id).unwrap(),
            None,
            "deleted entry reads as None"
        );
    }

    #[test]
    fn test_keyring_account_is_service_plus_connection_id() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (_lock, _guard) = install_scripted(seen.clone(), false, None, None, None);
        let store = CredentialStore::new();
        store.store_password("char-3-1-account", "pw").unwrap();

        let seen = seen.lock().unwrap();
        assert!(
            seen.iter()
                .any(|(service, user)| service == "com.rdpclient.app" && user == "char-3-1-account"),
            "credentials are keyed as (service com.rdpclient.app, connection id): {:?}",
            seen
        );
    }

    #[test]
    fn test_get_password_no_entry_maps_to_none() {
        let (_lock, _guard) = install_scripted(
            Arc::new(Mutex::new(Vec::new())),
            false,
            None,
            Some(Fail::NoEntry),
            None,
        );
        let store = CredentialStore::new();
        // Backend NoEntry -> Ok(None) (Req 1.2/6.2 mapping).
        assert_eq!(store.get_password("char-3-1-never-stored").unwrap(), None);
    }

    #[test]
    fn test_delete_password_no_entry_maps_to_ok() {
        let (_lock, _guard) = install_scripted(
            Arc::new(Mutex::new(Vec::new())),
            false,
            None,
            None,
            Some(Fail::NoEntry),
        );
        let store = CredentialStore::new();
        // Deleting a never-stored credential is Ok — "already gone, that's fine"
        // (this is what makes the command-level delete cleanup best-effort safe).
        store.delete_password("char-3-1-never-stored").unwrap();
    }

    #[test]
    fn test_store_password_platform_failure_maps_to_error_string() {
        let (_lock, _guard) = install_scripted(
            Arc::new(Mutex::new(Vec::new())),
            false,
            Some(Fail::Platform),
            None,
            None,
        );
        let err = CredentialStore::new()
            .store_password("char-3-1-fail-set", "pw")
            .unwrap_err();
        assert!(
            err.contains("Failed to store password in keychain"),
            "unexpected error text: {}",
            err
        );
    }

    #[test]
    fn test_get_password_platform_failure_maps_to_error_string() {
        let (_lock, _guard) = install_scripted(
            Arc::new(Mutex::new(Vec::new())),
            false,
            None,
            Some(Fail::Platform),
            None,
        );
        let err = CredentialStore::new()
            .get_password("char-3-1-fail-get")
            .unwrap_err();
        assert!(
            err.contains("Failed to retrieve password from keychain"),
            "unexpected error text: {}",
            err
        );
    }

    #[test]
    fn test_delete_password_platform_failure_maps_to_error_string() {
        let (_lock, _guard) = install_scripted(
            Arc::new(Mutex::new(Vec::new())),
            false,
            None,
            None,
            Some(Fail::Platform),
        );
        let err = CredentialStore::new()
            .delete_password("char-3-1-fail-del")
            .unwrap_err();
        assert!(
            err.contains("Failed to delete password from keychain"),
            "unexpected error text: {}",
            err
        );
    }

    #[test]
    fn test_entry_creation_failure_maps_to_error_string() {
        let (_lock, _guard) =
            install_scripted(Arc::new(Mutex::new(Vec::new())), true, None, None, None);
        let store = CredentialStore::new();
        for err in [
            store
                .store_password("char-3-1-fail-build", "pw")
                .unwrap_err(),
            store.get_password("char-3-1-fail-build").unwrap_err(),
            store.delete_password("char-3-1-fail-build").unwrap_err(),
        ] {
            assert!(
                err.contains("Failed to create keyring entry"),
                "unexpected error text: {}",
                err
            );
        }
    }

    // Task 5.1 (Req 8.2): the Windows build compiles keyring WITH the
    // windows-native feature, so the default credential builder routes to
    // the real Windows Credential Manager instead of keyring's in-process
    // mock store. The pre-5.1 baseline pinned the mock fallback here
    // (baseline-coverage.md row 6.2 classified Windows as the future
    // delta); this test is that delta landing, inverted to the target
    // behavior. The entry is isolated and cleaned up (Req 12.7): a unique
    // task-5-1 connection id, pre-deleted before and deleted after — no
    // other credential is ever touched.
    //
    // nextest (the canonical tier) runs each test in its own process, so
    // this test exercises keyring's compiled-in platform default — the
    // exact production path CredentialStore takes (Entry::new with no
    // test-installed builder).
    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_default_keyring_round_trips_credential_manager() {
        let _lock = keyring_lock().lock().unwrap();
        let store = CredentialStore::new();
        let id = "drift-task-5-1-win-default-roundtrip";

        // Clean up only this test's own entry (a leftover from an aborted
        // earlier run), then prove a full store/read/delete/read cycle.
        store.delete_password(id).unwrap();
        assert_eq!(
            store.get_password(id).unwrap(),
            None,
            "pre-cleaned entry must start absent"
        );
        store.store_password(id, "task-5-1-s3cret!").unwrap();
        // The mock-store discriminator: CredentialStore creates a fresh
        // Entry per call, and keyring's in-process mock never persists
        // across Entry instances, so this assertion fails if the Windows
        // build regresses to the mock fallback.
        assert_eq!(
            store.get_password(id).unwrap(),
            Some("task-5-1-s3cret!".to_string()),
            "Windows default keyring must persist across Entry::new calls (real Credential Manager, Req 8.2)"
        );
        store.delete_password(id).unwrap();
        assert_eq!(
            store.get_password(id).unwrap(),
            None,
            "deleted entry reads as None"
        );
    }

    // Task 5.1 (Req 8.2), deterministic twin of the default-builder test
    // above: `keyring::windows` only exists when the windows-native feature
    // is compiled in, so this test doubles as the compile-time pin for the
    // Cargo.toml feature (drop the feature and this file no longer builds
    // on Windows). Installing the windows builder explicitly also keeps the
    // round trip deterministic in a shared plain `cargo test` process,
    // where an earlier BuilderGuard drop has restored the mock as the
    // process default. Same Req 12.7 isolation: unique id, cleaned up
    // before and after, nothing else touched.
    #[cfg(target_os = "windows")]
    #[test]
    fn test_windows_native_builder_round_trips_credential_manager() {
        let _lock = keyring_lock().lock().unwrap();
        let _guard = BuilderGuard::install(keyring::windows::default_credential_builder());
        let store = CredentialStore::new();
        let id = "drift-task-5-1-win-explicit-roundtrip";

        store.delete_password(id).unwrap();
        assert_eq!(store.get_password(id).unwrap(), None);
        store.store_password(id, "task-5-1-s3cret!").unwrap();
        assert_eq!(
            store.get_password(id).unwrap(),
            Some("task-5-1-s3cret!".to_string()),
            "windows-native store must persist across Entry::new calls (Req 8.2)"
        );
        store.delete_password(id).unwrap();
        assert_eq!(store.get_password(id).unwrap(), None);
    }

    #[test]
    #[ignore] // Requires a keyring daemon; run with: cargo test -- --ignored
    fn test_store_and_retrieve_password() {
        let store = CredentialStore::new();
        let id = "test-conn-store-retrieve";
        store.store_password(id, "s3cret!").unwrap();
        let pw = store.get_password(id).unwrap();
        assert_eq!(pw, Some("s3cret!".to_string()));
        // cleanup
        let _ = store.delete_password(id);
    }

    #[test]
    #[ignore] // Requires a keyring daemon; run with: cargo test -- --ignored
    fn test_delete_password() {
        let store = CredentialStore::new();
        let id = "test-conn-delete";
        store.store_password(id, "temp").unwrap();
        store.delete_password(id).unwrap();
        let pw = store.get_password(id).unwrap();
        assert_eq!(pw, None);
    }

    // ---- Task 5.3: harness-owned credential namespace (Req 12.7).
    //
    // The E2E harness must isolate credential entries from user data and
    // clean up only its own resources. Test-support builds can name a
    // CredentialStore with a namespace that can never equal the production
    // service; scoped cleanup refuses to run against a production store.

    #[cfg(feature = "test-support")]
    mod test_support_namespace_isolation {
        use super::*;

        #[test]
        fn test_namespace_store_never_reuses_the_production_service_name() {
            let namespaced = CredentialStore::for_namespace("e2e-run-1");
            assert_ne!(
                namespaced.service_name(),
                SERVICE_NAME,
                "harness namespace must never equal the production service"
            );
            assert!(
                namespaced
                    .service_name()
                    .starts_with(&format!("{SERVICE_NAME}.test.")),
                "harness service stays under the reserved test prefix: {}",
                namespaced.service_name()
            );
            assert_eq!(CredentialStore::new().service_name(), SERVICE_NAME);
            assert!(
                !CredentialStore::new().is_test_namespaced(),
                "production store is never cleanup-eligible"
            );
        }

        #[test]
        fn test_namespaced_credentials_do_not_collide_with_production_entries() {
            let (_lock, _guard) =
                install_scripted(Arc::new(Mutex::new(Vec::new())), false, None, None, None);
            let production = CredentialStore::new();
            let harness = CredentialStore::for_namespace("e2e-run-1");

            production.store_password("prod-conn", "prod-pw").unwrap();
            harness.store_password("e2e-conn", "e2e-pw").unwrap();

            // Namespaces are disjoint: each store sees only its own entries.
            assert_eq!(production.get_password("e2e-conn").unwrap(), None);
            assert_eq!(harness.get_password("prod-conn").unwrap(), None);
            assert_eq!(
                production.get_password("prod-conn").unwrap(),
                Some("prod-pw".to_string())
            );
            assert_eq!(
                harness.get_password("e2e-conn").unwrap(),
                Some("e2e-pw".to_string())
            );
        }

        #[test]
        fn test_scoped_cleanup_deletes_only_harness_namespaced_entries() {
            let (_lock, _guard) =
                install_scripted(Arc::new(Mutex::new(Vec::new())), false, None, None, None);
            let production = CredentialStore::new();
            let harness = CredentialStore::for_namespace("e2e-run-2");

            production.store_password("prod-keep", "prod-pw").unwrap();
            harness.store_password("e2e-a", "a").unwrap();
            harness.store_password("e2e-b", "b").unwrap();

            // Cleanup refuses to run against a non-namespaced store…
            assert!(
                production
                    .delete_test_credentials(&["prod-keep".to_string()])
                    .is_err(),
                "scoped cleanup must refuse production stores"
            );
            // …and the refusal leaves production data intact.
            assert_eq!(
                production.get_password("prod-keep").unwrap(),
                Some("prod-pw".to_string())
            );

            // Scoped cleanup removes exactly the harness's own entries.
            let removed = harness
                .delete_test_credentials(&[
                    "e2e-a".to_string(),
                    "e2e-b".to_string(),
                    "e2e-missing".to_string(),
                ])
                .unwrap();
            assert_eq!(removed, 2, "only entries that existed are counted");
            assert_eq!(harness.get_password("e2e-a").unwrap(), None);
            assert_eq!(harness.get_password("e2e-b").unwrap(), None);
            assert_eq!(
                production.get_password("prod-keep").unwrap(),
                Some("prod-pw".to_string()),
                "cleanup never touches production credentials"
            );
        }
    }
}
