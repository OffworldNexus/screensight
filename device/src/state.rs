//! Per-instance dashboard state and the component visitor interface.
//!
//! Each paired Home Assistant instance owns a small key/value map (the
//! dashboard state). A dashboard component subscribes to the key it renders and
//! is called with the new value whenever that key changes — on every change, and
//! therefore also when a reconnecting Home Assistant resends its full state.
//!
//! The manager is transport-agnostic and synchronous: the WebSocket server and
//! the GPUI panel both touch it from their own threads.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Called with the new value when a subscribed key changes.
pub type Visitor = Arc<dyn Fn(&str) + Send + Sync>;
/// Called (with no arguments) when any value of a subscribed instance changes.
pub type InstanceVisitor = Arc<dyn Fn() + Send + Sync>;

/// Identifies one (instance, key) subscription.
type Key = (String, String);

#[derive(Default)]
struct Inner {
    /// `ha_id -> key -> value`.
    values: HashMap<String, BTreeMap<String, String>>,
    key_visitors: HashMap<Key, Vec<(u64, Visitor)>>,
    instance_visitors: HashMap<String, Vec<(u64, InstanceVisitor)>>,
}

/// Thread-safe dashboard state with per-key and per-instance subscriptions.
#[derive(Clone, Default)]
pub struct StateManager {
    inner: Arc<Mutex<Inner>>,
    next_id: Arc<AtomicU64>,
}

impl StateManager {
    /// An empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The values stored for `ha_id` (empty if the instance has none).
    #[must_use]
    pub fn values(&self, ha_id: &str) -> BTreeMap<String, String> {
        self.inner
            .lock()
            .expect("state mutex poisoned")
            .values
            .get(ha_id)
            .cloned()
            .unwrap_or_default()
    }

    /// One value, if set.
    #[must_use]
    pub fn value(&self, ha_id: &str, key: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("state mutex poisoned")
            .values
            .get(ha_id)
            .and_then(|m| m.get(key))
            .cloned()
    }

    /// Set one key, notifying its visitors (and the instance's) if it changed.
    pub fn set_value(&self, ha_id: &str, key: &str, value: &str) {
        let mut inner = self.inner.lock().expect("state mutex poisoned");
        let entry = inner.values.entry(ha_id.to_owned()).or_default();
        if entry.get(key).map(String::as_str) == Some(value) {
            return;
        }
        entry.insert(key.to_owned(), value.to_owned());
        let (key_visitors, instance_visitors) = collect(&inner, ha_id, std::iter::once(key));
        drop(inner);
        for (_, visitor) in key_visitors {
            visitor(value);
        }
        for visitor in instance_visitors {
            visitor();
        }
    }

    /// Replace an instance's whole map, notifying visitors for every key whose
    /// value changed (a key that disappeared notifies with an empty value).
    pub fn set_values(&self, ha_id: &str, values: BTreeMap<String, String>) {
        let mut inner = self.inner.lock().expect("state mutex poisoned");
        let previous = inner
            .values
            .insert(ha_id.to_owned(), values.clone())
            .unwrap_or_default();

        let mut changed: Vec<String> = Vec::new();
        for (key, value) in &values {
            if previous.get(key) != Some(value) {
                changed.push(key.clone());
            }
        }
        for key in previous.keys() {
            if !values.contains_key(key) {
                changed.push(key.clone());
            }
        }
        if changed.is_empty() {
            return;
        }
        let (key_visitors, instance_visitors) =
            collect(&inner, ha_id, changed.iter().map(String::as_str));
        drop(inner);
        for (key, visitor) in key_visitors {
            let new = values.get(&key).map(String::as_str).unwrap_or("");
            visitor(new);
        }
        for visitor in instance_visitors {
            visitor();
        }
    }

    /// Forget an instance's values and subscriptions (on unpair).
    pub fn remove_instance(&self, ha_id: &str) {
        let mut inner = self.inner.lock().expect("state mutex poisoned");
        inner.values.remove(ha_id);
        inner.key_visitors.retain(|(id, _), _| id != ha_id);
        inner.instance_visitors.remove(ha_id);
    }

    /// Subscribe `visitor` to one `(ha_id, key)`. Drops to unsubscribe.
    pub fn subscribe(
        &self,
        ha_id: &str,
        key: &str,
        visitor: impl Fn(&str) + Send + Sync + 'static,
    ) -> Subscription {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let target = (ha_id.to_owned(), key.to_owned());
        self.inner
            .lock()
            .expect("state mutex poisoned")
            .key_visitors
            .entry(target.clone())
            .or_default()
            .push((id, Arc::new(visitor)));
        Subscription {
            inner: Arc::clone(&self.inner),
            target: SubscriptionTarget::Key(target),
            id,
        }
    }

    /// Subscribe `visitor` to any value change for one instance. Drops to
    /// unsubscribe.
    pub fn subscribe_instance(
        &self,
        ha_id: &str,
        visitor: impl Fn() + Send + Sync + 'static,
    ) -> Subscription {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .lock()
            .expect("state mutex poisoned")
            .instance_visitors
            .entry(ha_id.to_owned())
            .or_default()
            .push((id, Arc::new(visitor)));
        Subscription {
            inner: Arc::clone(&self.inner),
            target: SubscriptionTarget::Instance(ha_id.to_owned()),
            id,
        }
    }
}

/// Clone out the visitors that must fire for `changed`, so callbacks run
/// without the mutex held (a visitor may re-enter the manager).
fn collect<'a>(
    inner: &Inner,
    ha_id: &str,
    changed: impl Iterator<Item = &'a str>,
) -> (Vec<(String, Visitor)>, Vec<InstanceVisitor>) {
    let mut key_visitors = Vec::new();
    for key in changed {
        if let Some(list) = inner.key_visitors.get(&(ha_id.to_owned(), key.to_owned())) {
            for (_, visitor) in list {
                key_visitors.push((key.to_owned(), Arc::clone(visitor)));
            }
        }
    }
    let instance_visitors = inner
        .instance_visitors
        .get(ha_id)
        .map(|list| list.iter().map(|(_, v)| Arc::clone(v)).collect())
        .unwrap_or_default();
    (key_visitors, instance_visitors)
}

enum SubscriptionTarget {
    Key(Key),
    Instance(String),
}

/// Removes its visitor from the manager when dropped.
pub struct Subscription {
    inner: Arc<Mutex<Inner>>,
    target: SubscriptionTarget,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let mut inner = self.inner.lock().expect("state mutex poisoned");
        match &self.target {
            SubscriptionTarget::Key(key) => {
                if let Some(list) = inner.key_visitors.get_mut(key) {
                    list.retain(|(id, _)| *id != self.id);
                }
            }
            SubscriptionTarget::Instance(ha_id) => {
                if let Some(list) = inner.instance_visitors.get_mut(ha_id) {
                    list.retain(|(id, _)| *id != self.id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn key_visitor_is_called_with_the_new_value() {
        let manager = StateManager::new();
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&seen);
        let _sub = manager.subscribe("ha1", "text", move |value| {
            sink.lock().unwrap().push(value.to_owned());
        });

        manager.set_value("ha1", "text", "hello");
        manager.set_value("ha1", "text", "hello"); // unchanged: no call
        manager.set_value("ha1", "text", "world");

        assert_eq!(*seen.lock().unwrap(), vec!["hello", "world"]);
        assert_eq!(manager.value("ha1", "text").as_deref(), Some("world"));
    }

    #[test]
    fn set_values_notifies_changed_keys_and_removals() {
        let manager = StateManager::new();
        manager.set_values(
            "ha1",
            BTreeMap::from([
                ("text".to_owned(), "one".to_owned()),
                ("accent".to_owned(), "teal".to_owned()),
            ]),
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let _sub = manager.subscribe("ha1", "text", move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        // Only `text` changed; `accent` is gone.
        manager.set_values(
            "ha1",
            BTreeMap::from([("text".to_owned(), "two".to_owned())]),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(manager.value("ha1", "accent"), None);
    }

    #[test]
    fn instance_visitor_fires_on_any_change() {
        let manager = StateManager::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let _sub = manager.subscribe_instance("ha1", move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });

        manager.set_value("ha1", "text", "a");
        manager.set_value("ha1", "other", "b");
        manager.set_value("ha2", "text", "ignored");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn dropping_a_subscription_stops_notifications() {
        let manager = StateManager::new();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let sub = manager.subscribe("ha1", "text", move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        manager.set_value("ha1", "text", "a");
        drop(sub);
        manager.set_value("ha1", "text", "b");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn removing_an_instance_clears_its_values() {
        let manager = StateManager::new();
        manager.set_value("ha1", "text", "a");
        manager.remove_instance("ha1");
        assert!(manager.values("ha1").is_empty());
    }
}
