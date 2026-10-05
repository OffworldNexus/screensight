//! In-memory display state: the per-instance text and which instance is shown.
//!
//! Every paired Home Assistant instance keeps its own text, but only the
//! selected one is painted. A monotonically increasing `version` tells the
//! renderer when it must rebuild its frame; it is bumped only when the
//! *visible* content actually changes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Display state for one paired instance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisplayState {
    /// The text this instance last pushed, if any.
    pub text: Option<String>,
}

#[derive(Default)]
struct Inner {
    per_instance: HashMap<String, DisplayState>,
    selected: Option<String>,
}

/// Thread-safe per-instance display state.
#[derive(Default)]
pub struct Displays {
    inner: Mutex<Inner>,
    version: AtomicU64,
}

impl Displays {
    /// An empty display state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current frame version. Increases whenever the visible content changes.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::SeqCst)
    }

    /// Store `text` for `ha_id`. Returns the new version.
    ///
    /// Bumps the version only when `ha_id` is the selected instance (i.e. when
    /// what is on the panel changes).
    pub fn set_text(&self, ha_id: &str, text: Option<String>) -> u64 {
        let mut inner = self.inner.lock().expect("display mutex poisoned");
        let entry = inner.per_instance.entry(ha_id.to_owned()).or_default();
        entry.text = text;
        let visible = inner.selected.as_deref() == Some(ha_id);
        drop(inner);
        if visible {
            self.bump()
        } else {
            self.version()
        }
    }

    /// The text stored for `ha_id`.
    #[must_use]
    pub fn text_for(&self, ha_id: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("display mutex poisoned")
            .per_instance
            .get(ha_id)
            .and_then(|s| s.text.clone())
    }

    /// The currently selected instance id.
    #[must_use]
    pub fn selected(&self) -> Option<String> {
        self.inner
            .lock()
            .expect("display mutex poisoned")
            .selected
            .clone()
    }

    /// Change the selected instance. Returns the new version.
    pub fn set_selected(&self, ha_id: Option<String>) -> u64 {
        let mut inner = self.inner.lock().expect("display mutex poisoned");
        let changed = inner.selected != ha_id;
        inner.selected = ha_id;
        drop(inner);
        if changed {
            self.bump()
        } else {
            self.version()
        }
    }

    /// The text currently painted on the panel (selected instance's text).
    #[must_use]
    pub fn visible_text(&self) -> Option<String> {
        let inner = self.inner.lock().expect("display mutex poisoned");
        inner
            .selected
            .as_deref()
            .and_then(|id| inner.per_instance.get(id))
            .and_then(|s| s.text.clone())
    }

    fn bump(&self) -> u64 {
        self.version.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_selected_changes_bump_the_version() {
        let displays = Displays::new();
        let v0 = displays.version();

        // No selection yet: storing text does not bump.
        assert_eq!(displays.set_text("a", Some("hello".into())), v0);
        assert_eq!(displays.text_for("a").as_deref(), Some("hello"));

        // Selecting bumps once.
        let v1 = displays.set_selected(Some("a".into()));
        assert!(v1 > v0);
        assert_eq!(displays.visible_text().as_deref(), Some("hello"));

        // Changing the selected instance's text bumps.
        let v2 = displays.set_text("a", Some("world".into()));
        assert!(v2 > v1);

        // Changing a non-selected instance's text does not bump.
        assert_eq!(displays.set_text("b", Some("other".into())), v2);
        assert_eq!(displays.visible_text().as_deref(), Some("world"));

        // Switching to b bumps and reveals b's text.
        let v3 = displays.set_selected(Some("b".into()));
        assert!(v3 > v2);
        assert_eq!(displays.visible_text().as_deref(), Some("other"));

        // Re-selecting the same instance does not bump.
        assert_eq!(displays.set_selected(Some("b".into())), v3);
    }
}
