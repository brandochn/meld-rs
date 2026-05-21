//! Integration tests for the meld-rs diff engine.

use meld_rs::diff::engine::{DiffOp, Differ};
use meld_rs::diff::matchers::CachedSequenceMatcher;

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
