//! The answer a read gave last time, so a view can open with it.
//!
//! `use_resource` already keeps its value while re-fetching, so a query that
//! re-runs inside a living component never blanks the screen. What it cannot do
//! is help a component that has just been created: its signal starts at `None`,
//! and the view shows a spinner until the network answers. That is every switch
//! between the apps of a group, which is why they felt slow next to the old
//! gqty client, whose normalised cache answered from memory and revalidated
//! behind it.
//!
//! This is that cache. A read opens with what it returned last time, marked by
//! nothing and corrected the moment the fetch lands.
//!
//! In memory, not storage: no `Serialize` bound, so it works for every read in
//! the tree without touching the 62 call sites, and nothing survives the tab.
//! `offline.rs` is the separate, deliberate thing that outlives a reload.
//!
//! **The access token is part of every key.** Two readers on one device can
//! never see each other's answers, and a token rotation empties the cache
//! rather than carrying answers across it.
//!
//! What this buys is a stale first paint. A permission revoked since the last
//! read is visible for one round trip before the fresh answer replaces it, on
//! every read rather than only the ones that cannot fail. That is a deliberate
//! trade, made on request.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use dioxus::prelude::*;

/// Entries kept before the oldest is dropped.
///
/// A bound rather than a policy: a long session moving through a big tree would
/// otherwise hold every list it ever drew. Insertion-ordered, so what goes is
/// what was filed longest ago, not what was read longest ago; the difference
/// does not matter at this size and an LRU would need a touch on every read.
const CAPACITY: usize = 300;

thread_local! {
    static ENTRIES: RefCell<HashMap<String, Rc<dyn Any>>> = RefCell::new(HashMap::new());
    static ORDER: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// One key per call site and set of dependencies.
pub fn key(site: &str, deps: &str) -> String {
    format!("{site}|{deps}")
}

/// What this key answered last time, if anything, and if it is still the type
/// being asked for.
pub fn get<T: Clone + 'static>(key: &str) -> Option<T> {
    ENTRIES.with(|entries| {
        entries
            .borrow()
            .get(key)
            .and_then(|held| held.downcast_ref::<T>())
            .cloned()
    })
}

/// File what a read returned.
pub fn put<T: 'static>(key: &str, value: T) {
    ENTRIES.with(|entries| {
        let mut entries = entries.borrow_mut();
        if entries.insert(key.to_string(), Rc::new(value)).is_none() {
            ORDER.with(|order| {
                let mut order = order.borrow_mut();
                order.push(key.to_string());
                while order.len() > CAPACITY {
                    let oldest = order.remove(0);
                    entries.remove(&oldest);
                }
            });
        }
    });
}

/// Drop everything. Called on sign-out alongside the rest of the session's
/// residue, so nothing a reader could see survives them leaving.
pub fn clear() {
    ENTRIES.with(|entries| entries.borrow_mut().clear());
    ORDER.with(|order| order.borrow_mut().clear());
}

/// Open a read with its last answer, and replace it when the fetch lands.
///
/// The resource yields the key its fetch was started under, because it keeps its
/// previous value while re-running: without that stamp, dependencies changing
/// would file the outgoing answer under the incoming key.
pub fn use_cached<T: Clone + 'static>(
    key: String,
    res: Resource<(String, T)>,
) -> Signal<Option<T>> {
    let mut value = use_signal(|| get::<T>(&key));
    use_effect(use_reactive!(|(key,)| {
        match res.read().clone() {
            // Landed, and still what this view is asking for.
            Some((k, v)) if k == key => {
                put(&k, v.clone());
                value.set(Some(v));
            }
            // Landed, but the dependencies moved on: keep the answer, show the
            // one that belongs to what is being asked for now.
            Some((k, v)) => {
                put(&k, v);
                value.set(get::<T>(&key));
            }
            None => value.set(get::<T>(&key)),
        }
    }));
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_comes_back_for_its_own_key_only() {
        clear();
        put("a", 1u32);
        assert_eq!(get::<u32>("a"), Some(1));
        assert_eq!(get::<u32>("b"), None);
        // A key holding one type does not answer for another.
        assert_eq!(get::<String>("a"), None);
    }

    #[test]
    fn the_oldest_entries_are_dropped_at_the_cap() {
        clear();
        for i in 0..(CAPACITY + 10) {
            put(&format!("k{i}"), i);
        }
        // The first ten are gone, the last CAPACITY are held.
        assert_eq!(get::<usize>("k0"), None);
        assert_eq!(get::<usize>("k9"), None);
        assert_eq!(
            get::<usize>(&format!("k{}", CAPACITY + 9)),
            Some(CAPACITY + 9)
        );
        clear();
    }

    #[test]
    fn re_filing_a_key_does_not_grow_the_order() {
        clear();
        for _ in 0..(CAPACITY * 2) {
            put("same", 1u32);
        }
        // One key, written many times, must not evict itself.
        assert_eq!(get::<u32>("same"), Some(1));
        clear();
    }
}
