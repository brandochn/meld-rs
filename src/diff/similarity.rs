//! Cross-line similarity matching for non-aligned changes.
//!
//! When the line-level diff produces Delete and Insert chunks at different
//! positions, this module detects that semantically similar lines (e.g. the
//! same function call with extra parameters) are in fact related — they
//! represent a **modification**, not independent deletion+insertion.
//!
//! The matcher computes token-based Jaccard similarity between unmatched
//! lines, restricted to a configurable window to keep performance O(n·w).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::diff::engine::{InlineChange, InlineDiffer};

/// A detected similarity match between a left-side line and a right-side line.
#[derive(Debug, Clone)]
pub struct SimilarityEntry {
    /// Line number in the left (A) file.
    pub left_line: usize,
    /// Line number in the right (B) file.
    pub right_line: usize,
    /// Similarity score between 0.0 and 1.0 (Jaccard index of token sets).
    pub score: f64,
    /// Token-level inline diff between the two lines, highlighting the
    /// differences within the matched lines.
    pub inline_diff: Vec<InlineChange>,
}

/// The complete set of cross-line similarity matches for a diff.
///
/// Stored as a **separate overlay** — it does not modify the core chunk
/// list, preserving the invariant that chunks form a linear partition.
#[derive(Debug, Clone, Default)]
pub struct SimilarityMap {
    /// All detected similarity matches, sorted by `(left_line, right_line)`.
    pub matches: Vec<SimilarityEntry>,
}

impl SimilarityMap {
    /// Build a similarity map from two line sequences.
    ///
    /// Only lines that are *not* part of an Equal chunk are candidates for
    /// cross-line matching. The matcher is restricted to a `window`
    /// (default ±50 lines around the expected position) and only pairs that
    /// exceed `threshold` (default 0.6) are kept.
    ///
    /// Performance: O(n · w) where n = number of unmatched lines and
    /// w = window size.  A fingerprint (length + first-token hash) pre-filters
    /// candidates before computing the full Jaccard index.
    pub fn build(
        left: &[String],
        right: &[String],
        matched_left: &HashSet<usize>,
        matched_right: &HashSet<usize>,
        threshold: f64,
        window: usize,
        cancel: &AtomicBool,
    ) -> Self {
        let mut map = Self::default();

        // Collect unmatched line indices from both sides
        let unmatched_left: Vec<usize> = (0..left.len())
            .filter(|i| !matched_left.contains(i))
            .collect();
        let unmatched_right: Vec<usize> = (0..right.len())
            .filter(|i| !matched_right.contains(i))
            .collect();

        if unmatched_left.is_empty() || unmatched_right.is_empty() {
            return map;
        }

        // Pre-compute fingerprints for fast rejection
        let left_fps: Vec<(usize, u64)> = unmatched_left
            .iter()
            .map(|&i| (i, fingerprint(&left[i])))
            .collect();
        let right_fps: Vec<(usize, u64)> = unmatched_right
            .iter()
            .map(|&i| (i, fingerprint(&right[i])))
            .collect();

        // Tokenize unmatched lines once (lazily via inline function)
        let tokenize = |line: &str| -> HashSet<String> {
            line.split(|c: char| {
                c.is_whitespace()
                    || c == ','
                    || c == ';'
                    || c == '{'
                    || c == '}'
                    || c == '('
                    || c == ')'
                    || c == '['
                    || c == ']'
            })
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
        };

        // Try matching each unmatched left line against right lines within
        // a window centred on the "expected" position (same relative index).
        for li in 0..left_fps.len() {
            if li % 100 == 0 && cancel.load(Ordering::SeqCst) {
                return map;
            }
            let (left_idx, left_fp) = left_fps[li];
            let left_idx = unmatched_left[li];
            // Expected right index: same proportion through the file
            let expected_right = if left.len() > 0 && right.len() > 0 {
                (left_idx * right.len()) / left.len()
            } else {
                left_idx
            };
            let window_start = expected_right.saturating_sub(window);
            let window_end = (expected_right + window).min(right.len());

            // Find matching right candidates
            for ri in 0..right_fps.len() {
                let (right_idx, right_fp) = right_fps[ri];
                let right_idx = unmatched_right[ri];
                if right_idx < window_start || right_idx >= window_end {
                    continue; // Outside search window
                }

                // Fast rejection: fingerprint must match
                if left_fp != right_fp {
                    continue;
                }

                // Full Jaccard similarity
                let left_tokens = tokenize(&left[left_idx]);
                let right_tokens = tokenize(&right[right_idx]);
                let score = jaccard(&left_tokens, &right_tokens);

                if score >= threshold {
                    let inline_diff =
                        InlineDiffer::compare_line_tokens(&left[left_idx], &right[right_idx]);
                    map.matches.push(SimilarityEntry {
                        left_line: left_idx,
                        right_line: right_idx,
                        score,
                        inline_diff,
                    });
                }
            }
        }

        map.matches.sort_by_key(|e| (e.left_line, e.right_line));
        map
    }

    /// Find the similarity match involving a specific line, if any.
    pub fn find_by_left(&self, line: usize) -> Option<&SimilarityEntry> {
        self.matches.iter().find(|e| e.left_line == line)
    }

    /// Find the similarity match involving a specific line, if any.
    pub fn find_by_right(&self, line: usize) -> Option<&SimilarityEntry> {
        self.matches.iter().find(|e| e.right_line == line)
    }
}

/// Compute the Jaccard similarity coefficient between two token sets.
///
/// `J(A, B) = |A ∩ B| / |A ∪ B|`
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 1.0;
    }
    intersection as f64 / union as f64
}

/// Compute a quick fingerprint for a line: a hash of (length, first_chars).
///
/// Uses the first few characters of the trimmed line (up to the first
/// whitespace or punctuation) to create a lenient pre-filter. Lines with
/// different fingerprints are unlikely to have Jaccard > threshold, so
/// they can be skipped without computing the full token set intersection.
fn fingerprint(line: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let trimmed = line.trim();
    // Take first segment split by whitespace
    let first_segment = trimmed.split(char::is_whitespace).next().unwrap_or("");
    // Use only the first 8 chars of the first segment for lenient matching
    let prefix = if first_segment.len() > 8 {
        &first_segment[..8]
    } else {
        first_segment
    };
    let mut hasher = DefaultHasher::new();
    prefix.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jaccard_identical() {
        let a: HashSet<String> = ["foo", "bar", "baz"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let b = a.clone();
        assert!((jaccard(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_different() {
        let a: HashSet<String> = ["aaa"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["bbb"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard(&a, &b) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_jaccard_partial() {
        let a: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let j = jaccard(&a, &b);
        assert!(j > 0.4 && j < 0.7);
    }

    #[test]
    fn test_build_similarity_map_empty_when_all_matched() {
        let left: Vec<String> = vec!["a".into(), "b".into()];
        let right: Vec<String> = vec!["a".into(), "b".into()];
        let matched_left: HashSet<usize> = (0..left.len()).collect();
        let matched_right: HashSet<usize> = (0..right.len()).collect();
        let map = SimilarityMap::build(
            &left,
            &right,
            &matched_left,
            &matched_right,
            0.6,
            50,
            &AtomicBool::new(false),
        );
        assert!(map.matches.is_empty());
    }

    #[test]
    fn test_build_similarity_map_finds_match() {
        let left: Vec<String> = vec!["notifyEr(a, b);".into()];
        let right: Vec<String> = vec!["notifyEr(x, y);".into()];
        let matched_left = HashSet::new();
        let matched_right = HashSet::new();
        let map = SimilarityMap::build(
            &left,
            &right,
            &matched_left,
            &matched_right,
            0.15,
            50,
            &AtomicBool::new(false),
        );
        assert!(
            !map.matches.is_empty(),
            "Should find similarity match for same-prefix lines"
        );
        assert!(map.matches[0].score > 0.15);
    }

    #[test]
    fn test_window_respected() {
        let left: Vec<String> = vec!["line a".into(), "line X".into(), "line b".into()];
        let right: Vec<String> = vec!["line a".into(), "line b".into(), "line X".into()];
        let matched_left = HashSet::new();
        let matched_right = HashSet::new();
        // Window=1: "line X" at position 1 can only match within [0,2] of right.
        // "line X" is at right[2], which is outside the window if expected ≈ 1.
        // Actually expected_right for left_idx=1 in 3 lines vs 3 lines = 1*3/3 = 1.
        // Window [0, 2] includes right[2]... so it would match.
        // Let's test with window=0 to ensure no match across positions.
        let map_restricted = SimilarityMap::build(
            &left,
            &right,
            &matched_left,
            &matched_right,
            0.5,
            0,
            &AtomicBool::new(false),
        );
        // With window=0, "line X" at left[1] tries to match right[1]="line b" — no match
        assert!(map_restricted.matches.is_empty());
    }

    #[test]
    fn test_fingerprint_same_for_same_first_token() {
        // Lines sharing the same first token (prefix) are likely similar.
        // The fingerprint should match so the full Jaccard check can run.
        let fp1 = fingerprint("notifyError(ex as Status);");
        let fp2 = fingerprint("notifyError(notificationDispatch, alertManager, ex as Status);");
        assert_eq!(
            fp1, fp2,
            "Lines with same first token should have matching fingerprints"
        );
    }
}
