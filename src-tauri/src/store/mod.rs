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
