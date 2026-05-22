//! Diagnostic test: compares diff output for left.ts vs right.ts
//! to identify discrepancies between the `similar`-based diff engine
//! and the expected behavior (matching original Meld).

use meld_rs::diff::engine::{merge_adjacent_replace_chunks, DiffOp, Differ};
use std::fs;

#[test]
fn diagnostic_left_vs_right() {
    let left_path = r"C:\Users\gdlhicha\Documents\left.ts";
    let right_path = r"C:\Users\gdlhicha\Documents\right.ts";

    let left_text = fs::read_to_string(left_path).expect("Failed to read left.ts");
    let right_text = fs::read_to_string(right_path).expect("Failed to read right.ts");

    let left_lines: Vec<String> = left_text.lines().map(|l| l.to_owned()).collect();
    let right_lines: Vec<String> = right_text.lines().map(|l| l.to_owned()).collect();

    eprintln!("left.ts: {} lines", left_lines.len());
    eprintln!("right.ts: {} lines", right_lines.len());

    let differ = Differ::new(left_lines, right_lines);
    let result = differ.compare();
    let raw = result.chunks;
    let merged = merge_adjacent_replace_chunks(&raw);

    // Count by type
    let mut raw_counts = std::collections::HashMap::new();
    for c in &raw {
        *raw_counts.entry(format!("{:?}", c.op)).or_insert(0) += 1;
    }
    let mut merged_counts = std::collections::HashMap::new();
    for c in &merged {
        *merged_counts.entry(format!("{:?}", c.op)).or_insert(0) += 1;
    }

    eprintln!("\n--- RAW chunks (from similar) ---");
    eprintln!("Total: {}, Counts: {:?}", raw.len(), raw_counts);

    eprintln!("\n--- MERGED chunks ---");
    eprintln!("Total: {}, Counts: {:?}", merged.len(), merged_counts);

    // Print first 30 chunks for inspection
    eprintln!("\n--- First 30 RAW chunks ---");
    for (i, c) in raw.iter().take(30).enumerate() {
        eprintln!(
            "  [{:3}] {:7} a=[{:4}..{:4}) b=[{:4}..{:4})",
            i, format!("{:?}", c.op), c.start_a, c.end_a, c.start_b, c.end_b
        );
    }

    eprintln!("\n--- First 30 MERGED chunks ---");
    for (i, c) in merged.iter().take(30).enumerate() {
        eprintln!(
            "  [{:3}] {:7} a=[{:4}..{:4}) b=[{:4}..{:4})",
            i, format!("{:?}", c.op), c.start_a, c.end_a, c.start_b, c.end_b
        );
    }

    // Check for potential issues: consecutive non-Equal chunks that
    // should have been merged
    eprintln!("\n--- Potential merge failures (consecutive Delete+Insert pairs that were NOT merged) ---");
    let mut issue_count = 0;
    for i in 0..raw.len().saturating_sub(1) {
        if (raw[i].op == DiffOp::Delete && raw[i + 1].op == DiffOp::Insert)
            || (raw[i].op == DiffOp::Insert && raw[i + 1].op == DiffOp::Delete)
        {
            // Check if they were merged
            let was_merged = merged.iter().any(|m| {
                m.op == DiffOp::Replace
                    && m.start_a <= raw[i].start_a
                    && m.end_a >= raw[i + 1].end_a.max(raw[i].end_a)
                    && m.start_b <= raw[i].start_b.min(raw[i + 1].start_b)
                    && m.end_b >= raw[i + 1].end_b.max(raw[i].end_b)
            });
            if !was_merged && issue_count < 10 {
                eprintln!(
                    "  ISSUE at raw[{},{}]: {:?} a=[{:4}..{:4}) b=[{:4}..{:4}) + {:?} a=[{:4}..{:4}) b=[{:4}..{:4})",
                    i,
                    i + 1,
                    raw[i].op,
                    raw[i].start_a,
                    raw[i].end_a,
                    raw[i].start_b,
                    raw[i].end_b,
                    raw[i + 1].op,
                    raw[i + 1].start_a,
                    raw[i + 1].end_a,
                    raw[i + 1].start_b,
                    raw[i + 1].end_b,
                );
                issue_count += 1;
            }
        }
    }
    if issue_count == 0 {
        eprintln!("  None found");
    }

    // Sanity checks
    assert!(!raw.is_empty(), "Should have at least some chunks");
    assert!(!merged.is_empty(), "Merged chunks should not be empty");
}
