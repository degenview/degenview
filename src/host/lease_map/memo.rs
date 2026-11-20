//! One value kept while what it was made from has not changed.

use std::sync::Arc;

pub struct Memo<Q, V> {
    kept: Option<(Q, Arc<V>)>,
}

impl<Q, V> Default for Memo<Q, V> {
    fn default() -> Self {
        Self { kept: None }
    }
}

impl<Q: PartialEq, V> Memo<Q, V> {
    /// The value kept for `key`, or the one `make` builds, which then replaces it.
    pub fn get_or(&mut self, key: Q, make: impl FnOnce() -> V) -> Arc<V> {
        if let Some((kept, value)) = &self.kept
            && *kept == key
        {
            return Arc::clone(value);
        }
        let value = Arc::new(make());
        self.kept = Some((key, Arc::clone(&value)));
        value
    }
}
