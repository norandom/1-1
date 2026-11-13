//! Harness-owned storage isolation (task 5.3, Req 12.7/12.10).
//!
//! Compiled ONLY in `test-support` builds. The E2E harness redirects the
//! app's persistent storage into a directory and credential namespace it
//! owns, so connection files, settings and keyring entries never touch user
//! data. Production builds do not contain this file at all: the environment
//! variables below do not exist there and cannot bypass the mock-session
//! gate (Req 12.10).

/// Environment variable redirecting the app-data directory (connections.json,
/// settings) into a harness-owned directory. Must be absolute.
pub const DATA_DIR_ENV: &str = "DRIFT_TEST_DATA_DIR";

/// Environment variable selecting a harness-owned keyring service namespace
/// (see [`crate::store::credentials::CredentialStore::for_namespace`]).
pub const CREDENTIAL_NAMESPACE_ENV: &str = "DRIFT_TEST_CREDENTIAL_NAMESPACE";

/// Absolute harness-owned data directory from [`DATA_DIR_ENV`], created if
/// missing. `None` when the variable is unset or blank (caller falls back to
/// the real app-data dir). Panics on a relative path: a silently wrong data
/// dir would break the 12.7 isolation guarantee.
pub fn harness_data_dir() -> Option<std::path::PathBuf> {
    let raw = std::env::var(DATA_DIR_ENV).ok()?;
    if raw.trim().is_empty() {
        return None;
    }
    if !std::path::Path::new(&raw).is_absolute() {
        panic!("{DATA_DIR_ENV} must be an absolute harness-owned path (got {raw:?})");
    }
    let path = std::path::PathBuf::from(raw);
    std::fs::create_dir_all(&path)
        .unwrap_or_else(|e| panic!("Failed to create harness data directory {path:?}: {e}"));
    log::info!("Harness data directory override: {:?}", path);
    Some(path)
}

/// Harness-namespaced credential store from [`CREDENTIAL_NAMESPACE_ENV`].
/// `None` when unset or blank (caller uses the production store).
pub fn harness_credential_store() -> Option<crate::store::credentials::CredentialStore> {
    let namespace = std::env::var(CREDENTIAL_NAMESPACE_ENV).ok()?;
    if namespace.trim().is_empty() {
        return None;
    }
    log::info!(
        "Harness credential namespace override: {}",
        crate::store::credentials::CredentialStore::for_namespace(&namespace).service_name()
    );
    Some(crate::store::credentials::CredentialStore::for_namespace(
        &namespace,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Removes the variable on scope exit (including panics) so shared-process
    /// test runs never inherit harness environment settings.
    struct EnvVarGuard(&'static str);
    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            unsafe { std::env::set_var(key, value) };
            EnvVarGuard(key)
        }
        fn unset(key: &'static str) -> Self {
            unsafe { std::env::remove_var(key) };
            EnvVarGuard(key)
        }
    }
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe { std::env::remove_var(self.0) };
        }
    }

    #[test]
    fn test_unset_or_blank_data_dir_env_means_no_override() {
        let _a = EnvVarGuard::unset(DATA_DIR_ENV);
        assert_eq!(harness_data_dir(), None, "unset → production path");

        let _b = EnvVarGuard::set(DATA_DIR_ENV, "   ");
        assert_eq!(harness_data_dir(), None, "blank → production path");
    }

    #[test]
    fn test_absolute_data_dir_env_is_created_and_returned() {
        let base = tempfile::tempdir().unwrap();
        let target = base.path().join("harness-data");
        let _g = EnvVarGuard::set(DATA_DIR_ENV, target.to_str().unwrap());

        let got = harness_data_dir();
        assert_eq!(got, Some(PathBuf::from(&target)), "absolute dir is honored");
        assert!(target.is_dir(), "harness data dir is created");
    }

    #[test]
    #[should_panic(expected = "absolute")]
    fn test_relative_data_dir_env_is_rejected() {
        let _g = EnvVarGuard::set(DATA_DIR_ENV, "relative/harness-data");
        harness_data_dir().expect("relative paths must panic, not fall back");
    }

    #[test]
    fn test_unset_or_blank_namespace_env_means_no_override() {
        let _a = EnvVarGuard::unset(CREDENTIAL_NAMESPACE_ENV);
        assert!(
            harness_credential_store().is_none(),
            "unset → production store"
        );

        let _b = EnvVarGuard::set(CREDENTIAL_NAMESPACE_ENV, "   ");
        assert!(
            harness_credential_store().is_none(),
            "blank → production store"
        );
    }

    #[test]
    fn test_namespace_env_yields_isolated_harness_store() {
        let _g = EnvVarGuard::set(CREDENTIAL_NAMESPACE_ENV, "e2e-run-42");
        let store = harness_credential_store().expect("namespace env yields a store");
        assert!(
            store.is_test_namespaced(),
            "harness store must be cleanup-eligible"
        );
        assert_eq!(
            store.service_name(),
            crate::store::credentials::CredentialStore::for_namespace("e2e-run-42").service_name()
        );
    }
}
