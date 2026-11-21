//! The frame store of every session on the native path, by session id
//! (spec rdp-native-renderer, task 2.1). The session actor owns its store;
//! the registry holds weak references, so a store disappears with its
//! session and the render thread never keeps a closed session alive.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use super::shared_frame::SharedFrame;

#[derive(Default)]
pub struct FrameStores(Mutex<HashMap<String, Weak<SharedFrame>>>);

impl FrameStores {
    /// Registers a session's store once the session has its id.
    pub fn register(&self, id: &str, store: &Arc<SharedFrame>) {
        let mut stores = self.0.lock().unwrap();
        stores.retain(|_, weak| weak.strong_count() > 0);
        stores.insert(id.to_owned(), Arc::downgrade(store));
    }

    /// The live store of a session, if it is still open and native.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the render thread, spec task 4.2")
    )]
    pub fn get(&self, id: &str) -> Option<Arc<SharedFrame>> {
        self.0.lock().unwrap().get(id)?.upgrade()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_lives_as_long_as_its_session() {
        let stores = FrameStores::default();
        let store = SharedFrame::new(2, 2);
        stores.register("a", &store);
        assert!(Arc::ptr_eq(&stores.get("a").unwrap(), &store));
        assert!(stores.get("b").is_none());
        drop(store);
        assert!(stores.get("a").is_none(), "the session ended");
    }

    #[test]
    fn closed_sessions_are_pruned_when_another_registers() {
        let stores = FrameStores::default();
        stores.register("a", &SharedFrame::new(1, 1));
        let b = SharedFrame::new(1, 1);
        stores.register("b", &b);
        assert_eq!(stores.0.lock().unwrap().len(), 1);
    }
}
