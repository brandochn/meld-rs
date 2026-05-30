//! Move detection for relocated code blocks.
//!
//! When code is moved (e.g. reordered imports, extracted functions), the
//! line-level diff produces a Delete on the left and an Insert at a different
//! position on the right. This module detects those as **movement** rather
//! than independent deletion+insertion.
//!
//! The algorithm groups consecutive unmatched lines from each side into blocks,
//! computes block-level similarity, and pairs blocks that exceed a threshold.
//!
//! Like [`SimilarityMap`], [`MoveMap`] is an **overlay** — it does not modify
//! the core chunk list, preserving the linear partition invariant.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

/// A detected code movement from one position to another.
#[derive(Debug, Clone)]
pub struct MoveEntry {
    /// Start line of the moved block in the left (A) file.
    pub left_start: usize,
    /// End line (exclusive) of the moved block in the left file.
    pub left_end: usize,
    /// Start line of the moved block in the right (B) file.
    pub right_start: usize,
    /// End line (exclusive) of the moved block in the right file.
    pub right_end: usize,
    /// Similarity score between the two blocks (0.0–1.0).
    pub score: f64,
    /// Was the moved block also modified? (score < 1.0)
    pub is_modified: bool,
}

/// The complete set of detected code movements for a diff.
#[derive(Debug, Clone, Default)]
pub struct MoveMap {
    /// Detected movements, sorted by `left_start`.
    pub moves: Vec<MoveEntry>,
}

impl MoveMap {
    /// Build a movement map from two line sequences, given the sets of lines
    /// that are already matched by the line-level diff.
    ///
    /// Only unmatched regions (Delete-only on left, Insert-only on right)
    /// are candidates. Blocks are formed by grouping consecutive unmatched
    /// lines, then compared via Jaccard similarity of token sets.
    ///
    /// Similarity threshold for movement is higher than for cross-line
    /// similarity (default 0.8) to avoid false positives — a moved block
    /// should be almost identical.
    pub fn build(
        left: &[String],
        right: &[String],
        matched_left: &HashSet<usize>,
        matched_right: &HashSet<usize>,
        threshold: f64,
        min_block_lines: usize,
        cancel: &AtomicBool,
    ) -> Self {
        let mut map = Self::default();

        // Find unmatched left blocks
        let left_blocks = find_consecutive_blocks(left.len(), matched_left, min_block_lines);
        let right_blocks = find_consecutive_blocks(right.len(), matched_right, min_block_lines);

        if left_blocks.is_empty() || right_blocks.is_empty() {
            return map;
        }

        // Try matching each left block against each right block
        let mut block_count = 0;
        for &(ls, le) in &left_blocks {
            if block_count % 10 == 0 && cancel.load(Ordering::SeqCst) {
                return map;
            }
            block_count += 1;
            let left_text = &left[ls..le];
            for &(rs, re) in &right_blocks {
                let right_text = &right[rs..re];

                let score = block_similarity(left_text, right_text);
                if score >= threshold {
                    map.moves.push(MoveEntry {
                        left_start: ls,
                        left_end: le,
                        right_start: rs,
                        right_end: re,
                        score,
                        is_modified: score < 1.0,
                    });
                    break; // Each left block matches at most one right block
                }
            }
        }

        map.moves.sort_by_key(|m| m.left_start);
        map
    }

    /// Find the movement entry involving a specific left line, if any.
    pub fn find_by_left(&self, line: usize) -> Option<&MoveEntry> {
        self.moves
            .iter()
            .find(|m| m.left_start <= line && line < m.left_end)
    }

    /// Find the movement entry involving a specific right line, if any.
    pub fn find_by_right(&self, line: usize) -> Option<&MoveEntry> {
        self.moves
            .iter()
            .find(|m| m.right_start <= line && line < m.right_end)
    }
}

/// Find maximal runs of consecutive unmatched line indices.
fn find_consecutive_blocks(
    total: usize,
    matched: &HashSet<usize>,
    min_lines: usize,
) -> Vec<(usize, usize)> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < total {
        if !matched.contains(&i) {
            let start = i;
            while i < total && !matched.contains(&i) {
                i += 1;
            }
            let end = i;
            if end - start >= min_lines {
                blocks.push((start, end));
            }
        } else {
            i += 1;
        }
    }
    blocks
}

/// Compute Jaccard similarity between two blocks of lines.
///
/// Each line is tokenized into a set of tokens, and the sets are merged
/// into a single union per block.
fn block_similarity(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() && right.is_empty() {
        return 1.0;
    }
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }

    let tokenize = |line: &str| -> HashSet<String> {
        line.split(|c: char| {
            c.is_whitespace() || c == ',' || c == ';' || c == '{' || c == '}' || c == '(' || c == ')'
        })
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
    };

    let left_tokens: HashSet<String> =
        left.iter().flat_map(|l| tokenize(l).into_iter()).collect();
    let right_tokens: HashSet<String> =
        right.iter().flat_map(|l| tokenize(l).into_iter()).collect();

    let intersection = left_tokens.intersection(&right_tokens).count();
    let union = left_tokens.len() + right_tokens.len() - intersection;
    if union == 0 {
        return 1.0;
    }
    intersection as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_consecutive_blocks() {
        let matched: HashSet<usize> = vec![0, 1, 4, 5].into_iter().collect();
        let blocks = find_consecutive_blocks(6, &matched, 1);
        assert_eq!(blocks, vec![(2, 4)]);
    }

    #[test]
    fn test_find_consecutive_blocks_min_lines() {
        let matched: HashSet<usize> = vec![0, 1, 4, 5].into_iter().collect();
        let blocks = find_consecutive_blocks(6, &matched, 3);
        assert!(blocks.is_empty());
    }

    #[test]
    fn test_block_similarity_identical() {
        let a = vec!["import { Foo } from 'bar';".to_string()];
        let b = vec!["import { Foo } from 'bar';".to_string()];
        assert!(block_similarity(&a, &b) > 0.99);
    }

    #[test]
    fn test_block_similarity_different() {
        let a = vec!["const x = 1;".to_string()];
        let b = vec!["const y = 2;".to_string()];
        assert!(block_similarity(&a, &b) < 0.5);
    }

    #[test]
    fn test_detect_moved_import() {
        let left: Vec<String> = vec![
            "a".into(),
            "import { X } from 'm';".into(),
            "b".into(),
        ];
        let right: Vec<String> = vec![
            "import { X } from 'm';".into(),
            "a".into(),
            "b".into(),
        ];
        let matched_left: HashSet<usize> = vec![0, 2].into_iter().collect();
        let matched_right: HashSet<usize> = vec![1, 2].into_iter().collect();
        let map = MoveMap::build(&left, &right, &matched_left, &matched_right, 0.8, 1, &AtomicBool::new(false));
        assert!(!map.moves.is_empty());
        assert_eq!(map.moves[0].left_start, 1);
        assert_eq!(map.moves[0].left_end, 2);
        assert_eq!(map.moves[0].right_start, 0);
        assert_eq!(map.moves[0].right_end, 1);
    }
}
