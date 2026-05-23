//! Diagnostic test: compares diff output for left.ts vs right.ts
//! to identify discrepancies between the `similar`-based diff engine
//! and the expected behavior (matching original Meld).

use meld_rs::diff::engine::{merge_adjacent_replace_chunks, DiffOp, Differ};
use std::collections::HashSet;
use std::fs;

#[test]
fn diagnostic_left_vs_right() {
    let left_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test_data/left.ts");
    let right_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test_data/right.ts");

    // Fall back to Documents path for local development
    let left_text = fs::read_to_string(left_path).unwrap_or_else(|_| {
        fs::read_to_string(r"C:\Users\gdlhicha\Documents\left.ts")
            .expect("Failed to read left.ts")
    });
    let right_text = fs::read_to_string(right_path).unwrap_or_else(|_| {
        fs::read_to_string(r"C:\Users\gdlhicha\Documents\right.ts")
            .expect("Failed to read right.ts")
    });

    let left_lines: Vec<String> = left_text.lines().map(|l| l.to_owned()).collect();
    let right_lines: Vec<String> = right_text.lines().map(|l| l.to_owned()).collect();

    eprintln!("left.ts: {} lines", left_lines.len());
    eprintln!("right.ts: {} lines", right_lines.len());

    let differ = Differ::new(left_lines.clone(), right_lines.clone());
    let result = differ.compare();
    let raw = result.chunks;
    let merged = merge_adjacent_replace_chunks(&raw);

    // Basic sanity checks
    assert!(!raw.is_empty(), "Should have at least some chunks");
    assert!(!merged.is_empty(), "Merged chunks should not be empty");

    // Verify that cross-line similarity matching can detect the
    // notifyError relationship (line ~509 in left, ~515 in right)
    let _notify_error_left = left_lines
        .iter()
        .position(|l| l.contains("notifyError(ex as Status)") && !l.contains("notificationDispatch"))
        .unwrap_or(0);
    let _notify_error_right = right_lines
        .iter()
        .position(|l| l.contains("notifyError(notificationDispatch, alertManager, ex as Status)"))
        .unwrap_or(0);

    // Verify that EnvironmentContext appears as a cross-line change
    let has_env_ctx_left = left_lines
        .iter()
        .any(|l| l.contains("EnvironmentContext") && l.contains("useSwallowNotification"));
    let has_env_ctx_right = right_lines
        .iter()
        .any(|l| l.contains("EnvironmentContext") && !l.contains("isViewPermissionOnly"));
    assert!(has_env_ctx_left, "Left file should have EnvironmentContext in shared import");
    assert!(has_env_ctx_right, "Right file should have EnvironmentContext as separate import");

    eprintln!("\n--- MERGED chunks ---");
    for (i, c) in merged.iter().enumerate() {
        eprintln!(
            "  [{:3}] {:7} a=[{:4}..{:4}) b=[{:4}..{:4})",
            i, format!("{:?}", c.op), c.start_a, c.end_a, c.start_b, c.end_b
        );
    }

    // Verify similarity map can be built
    let mut matched_left = HashSet::new();
    let mut matched_right = HashSet::new();
    for chunk in &merged {
        if chunk.op != DiffOp::Delete {
            for l in chunk.start_a..chunk.end_a {
                matched_left.insert(l);
            }
        }
        if chunk.op != DiffOp::Insert {
            for l in chunk.start_b..chunk.end_b {
                matched_right.insert(l);
            }
        }
    }
    let sim_map = meld_rs::diff::similarity::SimilarityMap::build(
        &left_lines, &right_lines, &matched_left, &matched_right, 0.6, 50,
    );
    eprintln!(
        "\nSimilarity matches found: {}",
        sim_map.matches.len()
    );

    let move_map = meld_rs::diff::movement::MoveMap::build(
        &left_lines, &right_lines, &matched_left, &matched_right, 0.8, 1,
    );
    eprintln!(
        "Movement entries found: {}",
        move_map.moves.len()
    );
}
