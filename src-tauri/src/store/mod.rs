/// Task 6.2 atomic writer (Req 6.1/10.10/10.12): same-directory staging
/// plus rename replacement through the runtime-promoted tempfile crate.
/// Nothing in the running app calls it yet — migration (6.3) and CRUD
/// read models (6.4) are the first production callers — so the otherwise
/// dead functions are allowed here rather than wired early across the
/// task boundary (same convention as `pub mod library` above; drop the
/// allow when the first production caller lands, tasks 6.3-6.6).
#[allow(dead_code)]
pub mod atomic_write;
pub mod connections;
pub mod credentials;
/// Task 6.1 model layer (Req 10.1/10.5/10.8): pure v2 document data, tree
/// invariants and descendant queries. Nothing in the running app calls it
/// yet — persistence/migration (6.3), CRUD read models (6.4), cross-library
/// moves (6.5) and connection integration (6.6) are the first production
/// callers, so the otherwise-unreachable items are allowed here rather than
/// being deleted or wired early across the task boundary.
#[allow(dead_code)]
pub mod library;
pub mod settings;
