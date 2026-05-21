//! Sequence matching helpers for line- and word-level comparisons.
//!
//! Ported from the Python `difflib.SequenceMatcher` used in the original Meld.

use similar::{DiffTag, TextDiff};

/// Provides similarity ratio and matching-block extraction between two sequences
/// of strings. Uses the `similar` crate's `TextDiff` internally.
#[derive(Debug, Clone)]
pub struct CachedSequenceMatcher {
    text_a: Vec<String>,
    text_b: Vec<String>,
}

impl CachedSequenceMatcher {
    /// Creates a new matcher for the given pair of sequences.
    pub fn new(text_a: Vec<String>, text_b: Vec<String>) -> Self {
        Self { text_a, text_b }
    }

    /// Returns the similarity ratio (0.0 to 1.0) between the two sequences.
    ///
    /// Uses the formula: `2 * M / T` where `M` is the total number of
    /// matching characters and `T` is the total number of characters in both
    /// sequences.
    pub fn ratio(&self) -> f64 {
        let a = self.text_a.join("\n");
        let b = self.text_b.join("\n");

        let (a_len, b_len) = (a.len(), b.len());
        if a_len == 0 && b_len == 0 {
            return 1.0;
        }
        if a_len == 0 || b_len == 0 {
            return 0.0;
        }

        let diff = TextDiff::from_chars(&a, &b);
        let matches: usize = diff
            .iter_all_changes()
            .filter(|c| c.tag() == similar::ChangeTag::Equal)
            .map(|c| c.value().len())
            .sum();

        2.0 * matches as f64 / (a_len + b_len) as f64
    }

    /// Returns triples `(i, j, n)` representing matching blocks, where `i` is
    /// a position in `text_a`, `j` in `text_b`, and `n` is the match length.
    pub fn matching_blocks(&self) -> Vec<(usize, usize, usize)> {
        let a = self.text_a.join("\n");
        let b = self.text_b.join("\n");
        let diff = TextDiff::from_lines(&a, &b);
        let mut blocks = Vec::new();

        for op in diff.ops() {
            if op.tag() == DiffTag::Equal {
                let len = op.old_range().len().min(op.new_range().len());
                blocks.push((op.old_range().start, op.new_range().start, len));
            }
        }

        // Sentinel block at the end
        blocks.push((self.text_a.len(), self.text_b.len(), 0));
        blocks
    }

    /// Returns the longest contiguous matching subsequence within the
    /// specified index ranges `(alo..ahi, blo..bhi)`.
    pub fn longest_match(
        &self,
        alo: usize,
        ahi: usize,
        blo: usize,
        bhi: usize,
    ) -> (usize, usize, usize) {
        let a_end = ahi.min(self.text_a.len());
        let b_end = bhi.min(self.text_b.len());

        let a_slice = &self.text_a[alo..a_end];
        let b_slice = &self.text_b[blo..b_end];

        let a_str = a_slice.join("\n");
        let b_str = b_slice.join("\n");
        let diff = TextDiff::from_lines(&a_str, &b_str);

        let mut best_len = 0;
        let mut best_i = alo;
        let mut best_j = blo;

        for op in diff.ops() {
            if op.tag() == DiffTag::Equal {
                let len = op.old_range().len().min(op.new_range().len());
                if len > best_len {
                    best_len = len;
                    best_i = alo + op.old_range().start;
                    best_j = blo + op.new_range().start;
                }
            }
        }

        (best_i, best_j, best_len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ratio_identical() {
        let a = vec!["line1".into(), "line2".into()];
        let matcher = CachedSequenceMatcher::new(a.clone(), a);
        assert!((matcher.ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ratio_completely_different() {
        let a = vec!["AAAA".into()];
        let b = vec!["BBBB".into()];
        let matcher = CachedSequenceMatcher::new(a, b);
        assert!(matcher.ratio() < 0.5);
    }

    #[test]
    fn test_matching_blocks() {
        let a = vec!["A".into(), "B".into(), "C".into()];
        let b = vec!["A".into(), "X".into(), "C".into()];
        let matcher = CachedSequenceMatcher::new(a, b);
        let blocks = matcher.matching_blocks();
        // At least the sentinel block
        assert!(!blocks.is_empty());
        // The sentinel is always last
        assert_eq!(blocks.last().unwrap().2, 0);
    }

    #[test]
    fn test_longest_match() {
        let a = vec!["aa".into(), "bb".into(), "cc".into(), "dd".into()];
        let b = vec!["xx".into(), "bb".into(), "cc".into(), "yy".into()];
        let matcher = CachedSequenceMatcher::new(a, b);
        let (i, j, n) = matcher.longest_match(0, 4, 0, 4);
        assert_eq!(i, 1);
        assert_eq!(j, 1);
        assert_eq!(n, 2);
    }
}
