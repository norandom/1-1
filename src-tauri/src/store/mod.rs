/// Task 6.2 atomic writer (Req 6.1/10.10/10.12): same-directory staging
/// plus rename replacement through the runtime-promoted tempfile crate.
/// First store-layer caller: the 6.3 migration/recovery store in
/// `store::library`. The running app does not wire it in yet — the 6.4-6.6
/// integration tasks are the first production callers (migrating
/// connections.json before 6.6 would strand the still-v1 ConnectionStore)
/// — so the dead-code allowance stays until then.
#[allow(dead_code)]
pub mod atomic_write;
pub mod connections;
pub mod credentials;
/// Task 6.1 model layer (Req 10.1/10.5/10.8) plus task 6.3 migration and
/// write-blocked recovery (Req 6.1/6.2/10.10/10.12): pure v2 document
/// data, tree invariants, descendant queries, v1→v2 migration and the
/// single-owner `LibraryStore`. Nothing in the running app calls it yet —
/// CRUD read models (6.4), cross-library moves (6.5) and connection
/// integration (6.6) are the first production callers, so the
/// otherwise-unreachable items are allowed here rather than
/// being deleted or wired early across the task boundary.
#[allow(dead_code)]
pub mod library;
pub mod settings;
