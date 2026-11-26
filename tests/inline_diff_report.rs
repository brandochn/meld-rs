//! Reproductions for the inline-diff report discussed in the v0.1.8 thread.
//!
//! All inputs are synthetic: the reporter's session files contain private
//! browsing data and are intentionally NOT checked in. The shapes below keep
//! the properties that matter for the diff (line-equality pattern, repeated
//! record bodies, long base64-like lines).
//!
//! Run the whole file:
//!
//! ```text
//! cargo test --no-default-features --test inline_diff_report
//! ```

use std::collections::HashMap;

use meld_rs::diff::engine::{Chunk, DiffOp, Differ, InlineDiffer};

fn compare(a: &[String], b: &[String]) -> Vec<Chunk> {
    Differ::new(a.to_vec(), b.to_vec()).compare().chunks
}

/// Largest side of any `Replace` chunk, in lines.
fn largest_replace_side(chunks: &[Chunk]) -> usize {
    chunks
        .iter()
        .filter(|c| c.op == DiffOp::Replace)
        .map(|c| (c.end_a - c.start_a).max(c.end_b - c.start_b))
        .max()
        .unwrap_or(0)
}

/// Additional extreme case: *every* line differs (not the alternating shape of
/// the reporter's `same-1MB`/`diff-1MB`). The engine collapses the whole file
/// into one `Replace` chunk; `apply_inline_diff` processes it line by line (up
/// to its 20 000-line budget) and the coarse fallback marks everything after
/// the shared prefix.
#[test]
fn all_lines_different_collapses_into_one_huge_replace_chunk() {
    const N: usize = 1_000;
    let a: Vec<String> = (0..N)
        .map(|i| format!("{i:04}-{}", "1234567890".repeat(9)))
        .collect();
    let b: Vec<String> = (0..N)
        .map(|i| format!("{i:04}-{}", "abcdefghij".repeat(9)))
        .collect();

    let chunks = compare(&a, &b);

    assert_eq!(chunks.len(), 1, "chunks: {chunks:?}");
    assert_eq!(chunks[0].op, DiffOp::Replace);
    assert_eq!(
        largest_replace_side(&chunks),
        N,
        "the whole file is one Replace chunk: {chunks:?}"
    );

    // The shared `NNNN-` prefix stays unmarked; the differing tail is marked.
    let ranges = InlineDiffer::highlight_ranges(&a[0], &b[0]);
    assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
    assert_eq!((ranges[0].start_a, ranges[0].start_b), (5, 5));
    assert_eq!(ranges[0].end_a, a[0].chars().count());
    assert_eq!(ranges[0].end_b, b[0].chars().count());
    assert_eq!(ranges[0].op, DiffOp::Replace);
}

/// Completely different long lines still get one coarse `Replace` range
/// instead of no highlight at all.
#[test]
fn completely_different_long_lines_get_a_coarse_range() {
    let a = "A".repeat(600);
    let b = "B".repeat(600);

    for ranges in [
        InlineDiffer::highlight_ranges(&a, &b),
        InlineDiffer::highlight_ranges_tokens(&a, &b),
    ] {
        assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
        assert_eq!(ranges[0].op, DiffOp::Replace);
        assert_eq!((ranges[0].start_a, ranges[0].end_a), (0, 600));
        assert_eq!((ranges[0].start_b, ranges[0].end_b), (0, 600));
    }
}

/// Edge case: one line is a strict prefix of the other.  The fallback must
/// emit an `Insert` on the longer side with an empty range on the shorter one.
#[test]
fn coarse_fallback_marks_prefix_only_insert() {
    let a = "A".repeat(100);
    let b = format!("{a}{}", "B".repeat(1000));

    let ranges = InlineDiffer::highlight_ranges(&a, &b);
    assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
    assert_eq!(ranges[0].op, DiffOp::Insert);
    assert_eq!((ranges[0].start_a, ranges[0].end_a), (100, 100));
    assert_eq!((ranges[0].start_b, ranges[0].end_b), (100, 1100));
}

/// Edge case: empty line on one side.  The fallback must mark the whole other
/// line as an insertion.
#[test]
fn coarse_fallback_handles_empty_side() {
    let b = "x".repeat(600);
    let ranges = InlineDiffer::highlight_ranges("", &b);
    assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
    assert_eq!(ranges[0].op, DiffOp::Insert);
    assert_eq!((ranges[0].start_a, ranges[0].end_a), (0, 0));
    assert_eq!((ranges[0].start_b, ranges[0].end_b), (0, 600));
}

/// Offsets are character indices, not bytes.  A 600-char line made of
/// two-byte characters must not produce byte-based ranges.
#[test]
fn coarse_fallback_uses_character_offsets() {
    let a = format!("{}{}", "á".repeat(600), "fin");
    let b = format!("{}{}", "é".repeat(600), "fin");

    let ranges = InlineDiffer::highlight_ranges(&a, &b);
    assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
    assert_eq!((ranges[0].start_a, ranges[0].end_a), (0, 600));
    assert_eq!((ranges[0].start_b, ranges[0].end_b), (0, 600));

    // A shared multi-byte prefix is counted in chars as well.
    let a = format!("{}{}", "á".repeat(100), "X".repeat(500));
    let b = format!("{}{}", "á".repeat(100), "Y".repeat(500));
    let ranges = InlineDiffer::highlight_ranges(&a, &b);
    assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
    assert_eq!(ranges[0].start_a, 100);
    assert_eq!(ranges[0].start_b, 100);
}

/// A line with dozens of separate changes must not paint hundreds of tags;
/// after `MAX_RANGES_PER_LINE` the result collapses to one coarse range.
#[test]
fn too_many_inline_ranges_fall_back_to_coarse() {
    let a = format!("a{}", "Xa".repeat(100));
    let b = format!("a{}", "Ya".repeat(100));

    let ranges = InlineDiffer::highlight_ranges(&a, &b);
    assert_eq!(ranges.len(), 1, "ranges should collapse to one: {ranges:?}");
    assert_eq!(ranges[0].op, DiffOp::Replace);
    assert_eq!((ranges[0].start_a, ranges[0].end_a), (1, 200));
    assert_eq!((ranges[0].start_b, ranges[0].end_b), (1, 200));
}

/// The real `same-1MB.txt` vs `diff-1MB.txt` shape: alternating equal and
/// changed lines, with each changed line sharing only the `NNNN-` prefix.
///
/// Measured on the reporter's files (not checked in): 10 000 lines,
/// 5 000 `Equal` + 5 000 `1x1 Replace` chunks.  v0.1.8 marked **0 of the 5 000**
/// changed lines because each pair fails the similarity guard; the new coarse
/// fallback marks all 5 000, starting exactly after the line-number prefix and
/// ending at the end of the line.
#[test]
fn alternating_changed_lines_get_coarse_tail_ranges() {
    const N: usize = 200;
    let a: Vec<String> = (0..N)
        .map(|i| format!("{i}-{}", "1234567890".repeat(10)))
        .collect();
    let b: Vec<String> = (0..N)
        .map(|i| {
            if i % 2 == 0 {
                a[i].clone()
            } else {
                format!("{i}-{}", "abcdefghij".repeat(10))
            }
        })
        .collect();

    let chunks = compare(&a, &b);
    let replaces: Vec<&Chunk> = chunks.iter().filter(|c| c.op == DiffOp::Replace).collect();
    assert_eq!(replaces.len(), N / 2, "chunks: {chunks:?}");

    for chunk in replaces {
        assert_eq!(
            (chunk.end_a - chunk.start_a, chunk.end_b - chunk.start_b),
            (1, 1)
        );
        let ia = chunk.start_a;
        let ranges = InlineDiffer::highlight_ranges(&a[ia], &b[ia]);
        assert_eq!(ranges.len(), 1, "line {ia}: {ranges:?}");
        assert_eq!(ranges[0].op, DiffOp::Replace);
        let prefix = a[ia].split_once('-').unwrap().0.len() + 1;
        assert_eq!((ranges[0].start_a, ranges[0].start_b), (prefix, prefix));
        assert_eq!(ranges[0].end_a, a[ia].chars().count());
        assert_eq!(ranges[0].end_b, b[ia].chars().count());
    }
}

/// `same-1MB.txt` vs `inli-1MB.txt`: periodic substitutions in a long digit
/// run. This is the case fixed in v0.1.8 (`2e284c0`): only the substituted
/// characters stay marked, both in character and in token mode.
///
/// Measured on the reporter's files (not checked in): 10 000 lines,
/// 5 000 `Equal` + 5 000 `1x1 Replace` chunks; v0.1.8 and the new code both
/// mark all 5 000, and on a 2 000-line sample the marked spans exactly equal
/// the differing character positions (0 mismatches).
#[test]
fn periodic_substitutions_highlight_only_changed_characters() {
    let a = format!("9971-{}", "1234567890".repeat(9));
    let b = format!("9971-{}", "1234x67890".repeat(9));

    for ranges in [
        InlineDiffer::highlight_ranges(&a, &b),
        InlineDiffer::highlight_ranges_tokens(&a, &b),
    ] {
        assert_eq!(ranges.len(), 9, "ranges: {ranges:?}");
        let changed: usize = ranges
            .iter()
            .filter(|r| r.op != DiffOp::Equal)
            .map(|r| (r.end_a - r.start_a).max(r.end_b - r.start_b))
            .sum();
        assert_eq!(changed, 9, "ranges: {ranges:?}");
        assert!(ranges.iter().all(|r| r.op == DiffOp::Replace));
    }
}

/// A long line with a handful of edits spread across it is 99% identical, so
/// it should get precise inline highlighting.  This used to be skipped because
/// the positional match count was disabled above 512 characters; it is now
/// sampled for long lines, with a 50% similarity floor and a timeout on the
/// character diff.
#[test]
fn long_line_with_scattered_edits_highlights() {
    let chars: Vec<char> = "A".repeat(600).chars().collect();

    let mut a_chars = chars.clone();
    let mut b_chars = chars.clone();
    for pos in [0usize, 200, 400, 599] {
        a_chars[pos] = 'X';
        b_chars[pos] = 'Y';
    }
    let a: String = a_chars.into_iter().collect();
    let b: String = b_chars.into_iter().collect();

    assert_eq!(a.len(), 600);

    for ranges in [
        InlineDiffer::highlight_ranges(&a, &b),
        InlineDiffer::highlight_ranges_tokens(&a, &b),
    ] {
        let changed: usize = ranges
            .iter()
            .filter(|r| r.op != DiffOp::Equal)
            .map(|r| (r.end_a - r.start_a).max(r.end_b - r.start_b))
            .sum();
        assert_eq!(changed, 4, "ranges: {ranges:?}");
    }
}

/// Control for the test above: the same edit pattern just below the 512-char
/// cutoff does highlight, isolating `POSITIONAL_MAX_CHARS` as the cause.
#[test]
fn long_line_with_scattered_edits_below_cutoff_highlights() {
    let chars: Vec<char> = "A".repeat(510).chars().collect();

    let mut a_chars = chars.clone();
    let mut b_chars = chars.clone();
    for pos in [0usize, 170, 340, 509] {
        a_chars[pos] = 'X';
        b_chars[pos] = 'Y';
    }
    let a: String = a_chars.into_iter().collect();
    let b: String = b_chars.into_iter().collect();

    let ranges = InlineDiffer::highlight_ranges(&a, &b);
    assert_eq!(ranges.len(), 4, "ranges: {ranges:?}");
    assert!(ranges.iter().all(|r| r.op == DiffOp::Replace));
}

/// `favIconUrl` shape: a long shared prefix followed by unrelated base64
/// tails. The shared prefix must stay unmarked and the differing tail must be
/// marked, matching the reporter's "whole line except the prefix" expectation.
///
/// Measured on the reporter's real `a-cut6`/`b-cut6` (not checked in): of 848
/// `favIconUrl` lines, 593 differ, 550 are longer than 512 chars, and the
/// v0.1.8 guard skipped 502 of them. With the changes below all 593 are marked
/// (547 coarse tail ranges and 46 precise multi-range highlights; the per-line
/// cap turns the noisiest cases into one coarse range).
#[test]
fn long_line_common_prefix_stays_unmarked() {
    let prefix = format!(
        "          \"favIconUrl\": \"data:image/png;base64,{}",
        "A".repeat(100)
    );
    let a = format!("{prefix}{}", "B".repeat(600));
    let b = format!("{prefix}{}", "C".repeat(600));

    for ranges in [
        InlineDiffer::highlight_ranges(&a, &b),
        InlineDiffer::highlight_ranges_tokens(&a, &b),
    ] {
        assert_eq!(ranges.len(), 1, "ranges: {ranges:?}");
        let start = prefix.chars().count();
        assert_eq!((ranges[0].start_a, ranges[0].start_b), (start, start));
        assert_eq!(ranges[0].end_a, a.chars().count());
        assert_eq!(ranges[0].end_b, b.chars().count());
        assert_eq!(ranges[0].op, DiffOp::Replace);
    }
}

/// A large `Replace` block with unequal line counts (the reporter's
/// screenshot at line 110372) must be re-aligned internally before inline
/// diffing.  Pairing the whole block by offset would compare record headers
/// with record bodies; pairing whole-chunk text would produce one coarse
/// range.  After the nested alignment only the changed header lines are
/// paired, and the differing numbers get small inline ranges.
///
/// Note: on the reporter's real `a-cut6`/`b-cut6` with v0.1.8 the line diff no
/// longer produces large blocks (largest `Replace` is 4x4, and line 110372 is
/// inside a 3-line `Equal` chunk).  Those screenshots came from the older
/// `fix-44-46-47` branch.  This test keeps the unequal-block case covered
/// anyway because it is the shape the new per-line path must handle.
#[test]
fn unequal_block_is_realigned_before_inline_diff() {
    fn record(id: u64) -> Vec<String> {
        vec![
            format!("        \"{id}\": {{"),
            format!("          \"id\": {id},"),
            format!("          \"index\": {},", id + 3),
            "          \"windowId\": 7,".to_string(),
            "          \"highlighted\": false,".to_string(),
            "          \"active\": false,".to_string(),
            "          \"attention\": false,".to_string(),
            "          \"pinned\": false,".to_string(),
            "          \"status\": \"complete\",".to_string(),
            "          \"hidden\": false,".to_string(),
            "          \"discarded\": true,".to_string(),
            "          \"incognito\": false,".to_string(),
            "          \"width\": 0,".to_string(),
            "          \"height\": 0,".to_string(),
            format!("          \"lastAccessed\": {},", 1_700_000_000_000u64 + id),
            "          \"audible\": false,".to_string(),
            "          \"autoDiscardable\": true,".to_string(),
            "          \"isInReaderMode\": false,".to_string(),
            "          \"successorTabId\": -1,".to_string(),
            "          \"groupId\": -1,".to_string(),
            "          \"splitViewId\": -1,".to_string(),
            "          \"cookieStoreId\": \"firefox-default\",".to_string(),
            format!("          \"favIconUrl\": \"data:image/png;base64,{id:016x}\""),
            "        },".to_string(),
        ]
    }

    // B inserts one record before the others and the surviving records have
    // shifted ids/indices, while their bodies stay equal — the shape of the
    // reporter's large blocks.
    let mut a = Vec::new();
    let mut b = Vec::new();
    for i in 0..3 {
        a.extend(record(3658 + i));
    }
    b.extend(record(3649));
    for i in 0..3 {
        b.extend(record(3758 + i));
    }

    let aligned = compare(&a, &b);
    assert!(aligned.iter().any(|c| c.op == DiffOp::Replace));

    let mut replace_pairs = 0usize;
    let mut inline_pairs = 0usize;
    for sub in aligned.iter().filter(|c| c.op == DiffOp::Replace) {
        let pairs = (sub.end_a - sub.start_a).min(sub.end_b - sub.start_b);
        for k in 0..pairs {
            replace_pairs += 1;
            let ranges = InlineDiffer::highlight_ranges(&a[sub.start_a + k], &b[sub.start_b + k]);
            if !ranges.is_empty() {
                inline_pairs += 1;
                // The change is inside the line, not a coarse whole-line mark.
                assert!(
                    ranges.iter().any(|r| r.start_a > 0 || r.start_b > 0),
                    "coarse range for pair ({}, {}): {ranges:?}",
                    sub.start_a + k,
                    sub.start_b + k
                );
            }
        }
    }

    assert!(replace_pairs > 0, "no Replace sub-chunks: {aligned:?}");
    assert_eq!(
        inline_pairs, replace_pairs,
        "every aligned Replace pair must get inline ranges: {aligned:?}"
    );
}
/// The background explanation in the thread: a *consistent bijection* over the
/// unique values preserves the line-equality pattern, while a fresh random
/// token per occurrence destroys it and changes the line diff.
///
/// This mirrors the sanitisation gate documented in
/// `tests/fixtures/issue44/README.md`, without using the private originals.
#[test]
fn consistent_bijection_preserves_the_line_diff() {
    // Session-like records; the same URL appears in several tabs, so the same
    // line repeats on both sides.
    let records = |times: usize| -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..times {
            out.push("        {".to_string());
            out.push("          \"url\": \"https://example.com/a\",".to_string());
            out.push("          \"title\": \"Alpha\",".to_string());
            out.push("        },".to_string());
            out.push("        {".to_string());
            out.push("          \"url\": \"https://example.com/b\",".to_string());
            out.push("          \"title\": \"Beta\",".to_string());
            out.push("        },".to_string());
        }
        out
    };
    let original_a = records(20);
    let mut original_b = records(20);
    original_b[1] = "          \"url\": \"https://example.com/c\",".to_string();

    // Sanitiser 1: consistent map over unique values (the repo's gate).
    let mut mapping: HashMap<String, String> = HashMap::new();
    let mut next = 0u64;
    let mut bijection = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                if !line.contains("https://") {
                    return line.clone();
                }
                let token = mapping.entry(line.clone()).or_insert_with(|| {
                    next += 1;
                    format!("token-{next:016x}")
                });
                token.clone()
            })
            .collect()
    };
    let sanitised_a = bijection(&original_a);
    let sanitised_b = bijection(&original_b);

    // Sanitiser 2: a fresh token per occurrence (the reporter's script).
    let mut counter = 0u64;
    let mut per_occurrence = |lines: &[String]| -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                if !line.contains("https://") {
                    return line.clone();
                }
                counter += 1;
                format!("token-{counter:016x}")
            })
            .collect()
    };
    let randomised_a = per_occurrence(&original_a);
    let randomised_b = per_occurrence(&original_b);

    assert_eq!(
        compare(&original_a, &original_b),
        compare(&sanitised_a, &sanitised_b),
        "a consistent bijection must not change the chunk list"
    );
    assert_ne!(
        compare(&original_a, &original_b),
        compare(&randomised_a, &randomised_b),
        "per-occurrence randomness must change the chunk list"
    );
}
