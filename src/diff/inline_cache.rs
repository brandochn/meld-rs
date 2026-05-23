//! LRU cache for inline (character-level) diff results.
//!
//! Mirrors the original Meld's `CachedSequenceMatcher` which caches
//! results of `InlineMyersSequenceMatcher` comparisons to avoid
//! recomputing the same inline diff multiple times.
//!
//! Uses a simple LRU cache keyed by `(line_a, line_b)` string pairs.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::diff::engine::{InlineChange, InlineDiffer};

/// Maximum number of cached inline diff results.
/// The original Meld uses `size_hint * 2` as the cache limit.
const DEFAULT_CACHE_SIZE: usize = 512;

/// Constant XORed into token-level cache keys to avoid collisions with
/// character-level (`compare_line`) cache entries for the same line pair.
const TOKEN_KEY_XOR: u64 = 0x8000_0000_0000_0000;

/// A simple LRU cache for inline diff results.
///
/// Keyed by `(line_a_hash, line_b_hash)` to avoid storing full strings
/// in the cache keys. Uses a generation counter for LRU eviction.
#[derive(Debug)]
pub struct InlineDiffCache {
    /// Cached results: key → (generation, result).
    cache: RefCell<HashMap<u64, (u64, Rc<Vec<InlineChange>>)>>,
    /// Monotonically increasing generation counter.
    generation: RefCell<u64>,
    /// Maximum number of entries before eviction.
    max_entries: usize,
}

impl InlineDiffCache {
    /// Create a new cache with the default capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CACHE_SIZE)
    }

    /// Create a new cache with the given capacity.
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            cache: RefCell::new(HashMap::new()),
            generation: RefCell::new(0),
            max_entries,
        }
    }

    /// Compute a hash for a pair of strings.
    fn hash_pair(line_a: &str, line_b: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        line_a.hash(&mut hasher);
        line_b.hash(&mut hasher);
        hasher.finish()
    }

    /// Get the inline diff for a pair of lines, using the cache if available.
    pub fn compare_line(&self, line_a: &str, line_b: &str) -> Rc<Vec<InlineChange>> {
        if line_a == line_b {
            return Rc::new(Vec::new());
        }

        let key = Self::hash_pair(line_a, line_b);

        // Check cache
        {
            let cache = self.cache.borrow();
            if let Some((_, result)) = cache.get(&key) {
                return Rc::clone(result);
            }
        }

        // Compute and store
        let changes = InlineDiffer::compare_line(line_a, line_b);
        let result = Rc::new(changes);

        let mut cache = self.cache.borrow_mut();
        let mut gen = self.generation.borrow_mut();

        *gen += 1;
        cache.insert(key, (*gen, Rc::clone(&result)));

        // Evict oldest entries if over capacity
        if cache.len() > self.max_entries {
            // Find the entry with the smallest generation
            let mut oldest_key: Option<u64> = None;
            let mut oldest_gen = u64::MAX;
            for (k, (g, _)) in cache.iter() {
                if *g < oldest_gen {
                    oldest_gen = *g;
                    oldest_key = Some(*k);
                }
            }
            if let Some(k) = oldest_key {
                cache.remove(&k);
            }
        }

        result
    }

    /// Get the token-level inline diff for a pair of lines, using the cache if available.
    ///
    /// Mirrors [`compare_line`] but calls [`InlineDiffer::compare_line_tokens`]
    /// instead of [`InlineDiffer::compare_line`], producing token-aware diffs
    /// that treat words/identifiers as atomic units.
    pub fn compare_line_tokens(&self, line_a: &str, line_b: &str) -> Rc<Vec<InlineChange>> {
        if line_a == line_b {
            return Rc::new(Vec::new());
        }

        // XOR with a sentinel bit to avoid collisions with compare_line keys.
        let key = Self::hash_pair(line_a, line_b) ^ TOKEN_KEY_XOR;

        // Check cache
        {
            let cache = self.cache.borrow();
            if let Some((_, result)) = cache.get(&key) {
                return Rc::clone(result);
            }
        }

        // Compute and store
        let changes = InlineDiffer::compare_line_tokens(line_a, line_b);
        let result = Rc::new(changes);

        let mut cache = self.cache.borrow_mut();
        let mut gen = self.generation.borrow_mut();

        *gen += 1;
        cache.insert(key, (*gen, Rc::clone(&result)));

        // Evict oldest entries if over capacity
        if cache.len() > self.max_entries {
            let mut oldest_key: Option<u64> = None;
            let mut oldest_gen = u64::MAX;
            for (k, (g, _)) in cache.iter() {
                if *g < oldest_gen {
                    oldest_gen = *g;
                    oldest_key = Some(*k);
                }
            }
            if let Some(k) = oldest_key {
                cache.remove(&k);
            }
        }

        result
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
        *self.generation.borrow_mut() = 0;
    }
}

impl Default for InlineDiffCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_hit() {
        let cache = InlineDiffCache::new();
        let a = "import * as vscode from \"vscode\";";
        let b = "import * as vscode from 'vscode';";

        let r1 = cache.compare_line(a, b);
        let r2 = cache.compare_line(a, b);

        // Same result (Rc pointer equality)
        assert!(Rc::ptr_eq(&r1, &r2));
        assert!(!r1.is_empty());
    }

    #[test]
    fn test_identical_lines_return_empty() {
        let cache = InlineDiffCache::new();
        let a = "same line";
        let result = cache.compare_line(a, a);
        assert!(result.is_empty());
    }

    #[test]
    fn test_eviction() {
        let cache = InlineDiffCache::with_capacity(2);
        let a1 = "line_a_1";
        let b1 = "line_b_1";
        let a2 = "line_a_2";
        let b2 = "line_b_2";
        let a3 = "line_a_3";
        let b3 = "line_b_3";

        cache.compare_line(a1, b1);
        cache.compare_line(a2, b2);
        cache.compare_line(a3, b3); // should evict oldest

        // Cache should still have at most 2 entries
        let cache = cache.cache.borrow();
        assert!(cache.len() <= 2);
    }

    // ── compare_line_tokens tests ──────────────────────────────────────

    #[test]
    fn test_tokens_cache_hit() {
        let cache = InlineDiffCache::new();
        let a = "import * as vscode from \"vscode\";";
        let b = "import * as vscode from 'vscode';";

        let r1 = cache.compare_line_tokens(a, b);
        let r2 = cache.compare_line_tokens(a, b);

        assert!(Rc::ptr_eq(&r1, &r2));
        assert!(!r1.is_empty());
    }

    #[test]
    fn test_tokens_identical_lines_return_empty() {
        let cache = InlineDiffCache::new();
        let a = "same line";
        let result = cache.compare_line_tokens(a, a);
        assert!(result.is_empty());
    }

    #[test]
    fn test_tokens_eviction() {
        let cache = InlineDiffCache::with_capacity(2);
        let a1 = "line_a_1";
        let b1 = "line_b_1";
        let a2 = "line_a_2";
        let b2 = "line_b_2";
        let a3 = "line_a_3";
        let b3 = "line_b_3";

        cache.compare_line_tokens(a1, b1);
        cache.compare_line_tokens(a2, b2);
        cache.compare_line_tokens(a3, b3);

        let cache = cache.cache.borrow();
        assert!(cache.len() <= 2);
    }

    #[test]
    fn test_tokens_no_cross_cache_collision() {
        // Ensure that compare_line and compare_line_tokens use separate
        // cache keys for the same line pair.
        let cache = InlineDiffCache::new();
        let a = "fooBarBaz";
        let b = "fooBarQux";

        let r_char = cache.compare_line(a, b);
        let r_tok = cache.compare_line_tokens(a, b);

        // Both produce non-empty results but should be independent Rc allocations.
        assert!(!r_char.is_empty());
        assert!(!r_tok.is_empty());
        assert!(!Rc::ptr_eq(&r_char, &r_tok));
    }

    #[test]
    fn test_tokens_token_level_difference() {
        // compare_line_tokens should treat words as atomic units,
        // producing fewer (or different) changes than character-level for
        // simple rename scenarios.
        let cache = InlineDiffCache::new();
        let a = "old_variable_name";
        let b = "new_variable_name";

        let result = cache.compare_line_tokens(a, b);
        // Should produce insert/delete/replace changes for the token diff.
        assert!(!result.is_empty());
    }
}
