//! Regression tests for issue #44 ("edge case missing inline differences").
//!
//! The fixtures are sanitised copies of the reporter's session files; see
//! `tests/fixtures/issue44/README.md` for provenance and the verification gate.
//!
//! These tests fail on `main` before the fix:
//! * the chunk list is not an ordered partition (overlapping/backwards ranges),
//! * lines are painted by more than one chunk,
//! * there is roughly one chunk per line.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

use meld_rs::diff::engine::{Chunk, DiffOp, Differ};

struct Pair {
    name: &'static str,
    a: Vec<String>,
    b: Vec<String>,
    chunks: Vec<Chunk>,
}

fn fixture(name: &str) -> Vec<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("issue44")
        .join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    text.lines().map(|l| l.to_string()).collect()
}

fn load_pair(name: &'static str) -> Pair {
    let a = fixture(&format!("a-{name}.json"));
    let b = fixture(&format!("b-{name}.json"));
    let chunks = Differ::new(a.clone(), b.clone()).compare().chunks;
    Pair { name, a, b, chunks }
}

/// Both fixture pairs, diffed once per test binary.
fn pairs() -> &'static [Pair] {
    static CACHE: OnceLock<Vec<Pair>> = OnceLock::new();
    CACHE.get_or_init(|| vec![load_pair("cut3"), load_pair("cut4")])
}

/// The chunk list must be an ordered partition of both inputs.
#[test]
fn partition_is_valid() {
    for pair in pairs() {
        let mut covered_a = 0usize;
        let mut covered_b = 0usize;

        for chunk in &pair.chunks {
            assert_eq!(
                chunk.start_a, covered_a,
                "[{}] gap/overlap on A before {chunk:?}",
                pair.name
            );
            assert_eq!(
                chunk.start_b, covered_b,
                "[{}] gap/overlap on B before {chunk:?}",
                pair.name
            );
            assert!(
                chunk.end_a >= chunk.start_a && chunk.end_b >= chunk.start_b,
                "[{}] inverted range: {chunk:?}",
                pair.name
            );
            covered_a = chunk.end_a;
            covered_b = chunk.end_b;
        }

        assert_eq!(
            covered_a,
            pair.a.len(),
            "[{}] chunks do not cover all of A",
            pair.name
        );
        assert_eq!(
            covered_b,
            pair.b.len(),
            "[{}] chunks do not cover all of B",
            pair.name
        );
    }
}

/// No line may be covered by more than one non-`Equal` chunk. Overlapping
/// ranges make the renderer apply conflicting tags to the same line, so the
/// visible colour depends on chunk order.
#[test]
fn no_duplicate_painting() {
    for pair in pairs() {
        for (label, pane_a) in [("A", true), ("B", false)] {
            let mut painted: HashSet<usize> = HashSet::new();
            for chunk in &pair.chunks {
                if chunk.op == DiffOp::Equal {
                    continue;
                }
                let (start, end) = if pane_a {
                    (chunk.start_a, chunk.end_a)
                } else {
                    (chunk.start_b, chunk.end_b)
                };
                for line in start..end {
                    assert!(
                        painted.insert(line),
                        "[{}] line {label}[{line}] is painted by more than one chunk (chunk {chunk:?})",
                        pair.name
                    );
                }
            }
        }
    }
}

/// `similar` emits one change per line, so a naive pipeline produces one chunk
/// per line. The fix coalesces adjacent runs; keep the result well below that.
#[test]
fn chunk_count_is_bounded() {
    for pair in pairs() {
        let lines = pair.a.len().max(pair.b.len());
        assert!(
            pair.chunks.len() <= lines / 2,
            "[{}] {} chunks for {} lines: adjacent runs are not coalesced",
            pair.name,
            pair.chunks.len(),
            lines
        );
    }
}

/// A `Replace` chunk may not be wildly unbalanced: on these files `similar`
/// otherwise pairs a deleted record with a few changed header lines, painting
/// a whole entry blue on one side and a tiny block on the other.  The split
/// pass turns that into a `Delete` plus a small balanced `Replace`.
#[test]
fn no_unbalanced_replace_chunks() {
    for pair in pairs() {
        for chunk in &pair.chunks {
            if chunk.op != DiffOp::Replace {
                continue;
            }
            let len_a = chunk.end_a - chunk.start_a;
            let len_b = chunk.end_b - chunk.start_b;
            let big = len_a.max(len_b);
            let small = len_a.min(len_b);
            assert!(
                big < 10 || small.saturating_mul(3) >= big,
                "[{}] unbalanced Replace {chunk:?} ({len_a} x {len_b})",
                pair.name
            );
        }
    }
}

/// The reporter's `"3603"` entry is deleted in B, while B's `"3618"` entry is
/// the next surviving record whose `id`/`index` changed.  The rendering must be
/// a `Delete` of the whole entry followed by a small `3 x 3` `Replace` for the
/// changed header, not one large `Replace` covering the entry.
#[test]
fn deleted_entry_is_separated_from_header_change() {
    let pair = pairs()
        .iter()
        .find(|p| p.name == "cut3")
        .expect("cut3 fixture");

    let find = |lines: &[String], text: &str| {
        lines
            .iter()
            .position(|line| line.trim() == text)
            .unwrap_or_else(|| panic!("line {text:?} not found"))
    };

    let a_3603 = find(&pair.a, "\"3603\": {");
    let a_3605 = find(&pair.a, "\"3605\": {");
    let b_3618 = find(&pair.b, "\"3618\": {");

    let covering = |line: usize| {
        pair.chunks
            .iter()
            .find(|c| c.start_a <= line && line < c.end_a)
            .unwrap_or_else(|| panic!("no chunk covers A[{line}]"))
    };

    let deleted = covering(a_3603);
    assert_eq!(
        deleted.op,
        DiffOp::Delete,
        "A3603 should be a deletion: {deleted:?}"
    );
    assert!(
        deleted.start_a <= a_3603 && deleted.end_a >= a_3605,
        "the deletion must cover the whole deleted entry: {deleted:?}"
    );

    let header = covering(a_3605);
    assert_eq!(
        header.op,
        DiffOp::Replace,
        "A3605 header should be a Replace: {header:?}"
    );
    assert_eq!(
        header.end_a - header.start_a,
        3,
        "unexpected header chunk: {header:?}"
    );
    assert_eq!(
        header.start_a, a_3605,
        "unexpected header start: {header:?}"
    );
    assert_eq!(
        header.start_b, b_3618,
        "header must pair with B3618: {header:?}"
    );
}

/// `Equal` chunks must pair identical lines of equal length.
#[test]
fn equal_chunks_pair_identical_lines() {
    for pair in pairs() {
        for chunk in &pair.chunks {
            if chunk.op != DiffOp::Equal {
                continue;
            }
            let len_a = chunk.end_a - chunk.start_a;
            let len_b = chunk.end_b - chunk.start_b;
            assert_eq!(
                len_a, len_b,
                "[{}] Equal chunk with unequal sides: {chunk:?}",
                pair.name
            );
            for k in 0..len_a {
                assert_eq!(
                    pair.a[chunk.start_a + k],
                    pair.b[chunk.start_b + k],
                    "[{}] Equal chunk pairs different lines at offset {k}: {chunk:?}",
                    pair.name
                );
            }
        }
    }
}
