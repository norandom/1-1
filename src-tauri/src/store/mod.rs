/// Task 6.2 atomic writer (Req 6.1/10.10/10.12): same-directory staging
/// plus rename replacement through the runtime-promoted tempfile crate.
/// Production caller since task 6.4: the library store's transactional
/// commit (`store::library::LibraryStore`) — since task 6.6 the single
/// owner of `connections.json` (v2) incl. the unification merge/adopt
/// and connection CRUD.
pub mod atomic_write;
/// Task 6.6: the connection DATA MODEL (persisted `ConnectionConfig`,
/// the CRUD/import wire payload `ConnectionInput`, the .rdp/SSH-config
/// import parsers). The v1 `ConnectionStore` was retired by the
/// unification — persistence lives in `store::library`.
pub mod connections;
pub mod credentials;
/// Task 6.1 model layer (Req 10.1/10.5/10.8) plus task 6.3 migration and
/// write-blocked recovery (Req 6.1/6.2/10.10/10.12), the task 6.4
/// CRUD/read facade (Req 10.1/10.2/10.3/10.5) behind the IPC commands in
/// `commands::library`, task 6.5 moves — and task 6.6: the unified
/// connection store (`connections.json` v2) with connection
/// CRUD/import/duplication behind `commands::connection`.
pub mod library;
pub mod settings;
