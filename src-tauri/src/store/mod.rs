/// Task 6.2 atomic writer (Req 6.1/10.10/10.12): same-directory staging
/// plus rename replacement through the runtime-promoted tempfile crate.
/// Production caller since task 6.4: the library store's transactional
/// commit (`store::library::LibraryStore`).
pub mod atomic_write;
pub mod connections;
pub mod credentials;
/// Task 6.1 model layer (Req 10.1/10.5/10.8) plus task 6.3 migration and
/// write-blocked recovery (Req 6.1/6.2/10.10/10.12) and the task 6.4
/// CRUD/read facade (Req 10.1/10.2/10.3/10.5) behind the IPC commands in
/// `commands::library`.
pub mod library;
pub mod settings;
