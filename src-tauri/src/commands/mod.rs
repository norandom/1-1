pub mod clipboard;
pub mod connection;
pub mod session;
pub mod settings;
/// Test-support-only E2E commands (task 5.3): compiled and registered ONLY
/// in `test-support` builds; production builds never see this module.
#[cfg(feature = "test-support")]
pub mod test_support;
