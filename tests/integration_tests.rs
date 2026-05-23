//! Integration tests for the meld-rs diff engine.

use meld_rs::diff::engine::{DiffOp, Differ, LineCache};
use meld_rs::diff::matchers::CachedSequenceMatcher;
use std::collections::HashSet;

#[test]
fn test_diff_empty_files() {
    let a: Vec<String> = vec![];
    let b: Vec<String> = vec![];
    let differ = Differ::new(a, b);
    let result = differ.compare();
    assert!(result.chunks.is_empty());
}

#[test]
fn test_diff_one_line_change() {
    let a = vec!["hello".to_string()];
    let b = vec!["world".to_string()];
    let differ = Differ::new(a, b);
    let result = differ.compare();
    assert!(!result.chunks.is_empty());
}

#[test]
fn test_diff_multi_line() {
    let a = vec![
        "line1".into(),
        "line2".into(),
        "line3".into(),
        "line4".into(),
    ];
    let b = vec![
        "line1".into(),
        "line2-modified".into(),
        "line3".into(),
        "line5".into(),
    ];
    let differ = Differ::new(a, b);
    let result = differ.compare();
    assert!(!result.chunks.is_empty());
}

#[test]
fn test_sequence_matcher_ratio() {
    let a = vec!["hello world".into(), "foo bar".into()];
    let b = vec!["hello world".into(), "baz qux".into()];
    let matcher = CachedSequenceMatcher::new(a, b);
    let ratio = matcher.ratio();
    assert!(ratio > 0.0);
    assert!(ratio < 1.0);
}

#[test]
fn test_sequence_matcher_identical() {
    let a = vec!["same".into(), "text".into()];
    let matcher = CachedSequenceMatcher::new(a.clone(), a);
    assert!((matcher.ratio() - 1.0).abs() < 0.01);
}

#[test]
fn test_merge_adjacent_chunks() {
    use meld_rs::diff::engine::{merge_adjacent_replace_chunks, Chunk};

    let chunks = vec![
        Chunk {
            start_a: 0,
            end_a: 1,
            start_b: 0,
            end_b: 1,
            op: DiffOp::Equal,
        },
        Chunk {
            start_a: 1,
            end_a: 2,
            start_b: 0,
            end_b: 0,
            op: DiffOp::Delete,
        },
        Chunk {
            start_a: 0,
            end_a: 0,
            start_b: 1,
            end_b: 2,
            op: DiffOp::Insert,
        },
    ];
    let merged = merge_adjacent_replace_chunks(&chunks);
    assert_eq!(merged.len(), 2);
    assert_eq!(merged[1].op, DiffOp::Replace);
}

#[test]
fn test_chunk_triad_basic() {
    use meld_rs::diff::engine::Chunk;
    let chunks = vec![
        Chunk {
            start_a: 0,
            end_a: 2,
            start_b: 0,
            end_b: 2,
            op: DiffOp::Equal,
        },
        Chunk {
            start_a: 2,
            end_a: 3,
            start_b: 2,
            end_b: 2,
            op: DiffOp::Delete,
        },
        Chunk {
            start_a: 3,
            end_a: 5,
            start_b: 2,
            end_b: 4,
            op: DiffOp::Replace,
        },
    ];
    let cache = LineCache::new(&chunks, 5);
    // Line 2 is in Delete chunk
    let (prev, curr, next) = cache.chunk_triad(2);
    assert_eq!(curr, Some(1));
    assert_eq!(prev, None);
    assert_eq!(next, Some(2));
    // Line 3 is in Replace chunk
    let (prev, curr, next) = cache.chunk_triad(3);
    assert_eq!(curr, Some(2));
    assert_eq!(prev, Some(1));
    assert_eq!(next, None);
}

#[test]
fn test_similarity_map_notify_error_scenario() {
    use meld_rs::diff::similarity::SimilarityMap;
    // Use lines with same length + same prefix for fingerprint to match
    let left = vec!["notifyEr(ex);".into()];
    let right = vec!["notifyEr(dx);".into()];
    let matched_left = HashSet::new();
    let matched_right = HashSet::new();
    let map = SimilarityMap::build(&left, &right, &matched_left, &matched_right, 0.15, 50);
    assert!(!map.matches.is_empty());
    assert!(map.matches[0].score > 0.15);
}

#[test]
fn test_move_detection_basic() {
    use meld_rs::diff::movement::MoveMap;
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
    let map = MoveMap::build(&left, &right, &matched_left, &matched_right, 0.8, 1);
    assert!(!map.moves.is_empty());
    assert_eq!(map.moves[0].left_start, 1);
    assert_eq!(map.moves[0].right_start, 0);
}

#[test]
fn test_pair_changes_filtered() {
    let differ = Differ::new(
        vec!["a".into(), "b".into(), "c".into()],
        vec!["a".into(), "x".into(), "c".into()],
    );
    let result = differ.compare();
    // Only the Replace chunk at position 1 should be returned
    let pairs = result.pair_changes(0, 1, None);
    assert!(!pairs.is_empty());
    assert!(pairs.iter().all(|(_, c)| c.op != DiffOp::Equal));
}

#[test]
fn test_tokenize_with_offsets_camelcase() {
    // The tokenizer is private, but compare_line_tokens exercises it
    use meld_rs::diff::engine::InlineDiffer;
    let changes = InlineDiffer::compare_line_tokens(
        "fooBarBaz",
        "fooBarQux",
    );
    // Should find the differing token at the end
    assert!(!changes.is_empty());
}
