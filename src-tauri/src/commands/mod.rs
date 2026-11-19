pub mod clipboard;
pub mod connection;
pub(crate) mod credential_check;
pub mod deletion;
pub(crate) mod folder_share;
pub(crate) mod launch;
pub mod library;
pub mod native_clipboard;
pub mod peer_identity;
pub mod session;
pub mod settings;
/// Test-support-only E2E commands (task 5.3): compiled and registered ONLY
/// in `test-support` builds; production builds never see this module.
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod updater;
