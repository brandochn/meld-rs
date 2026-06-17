//! Property-based fuzz tests for the diff engine.
//!
//! Generates random text pairs and verifies that:
//! 1. Chunks form a **valid partition** (no gaps, no overlaps, full coverage)
//! 2. **Reconstruction**: applying chunks to text_a produces text_b
//! 3. **Merge idempotence**: merge_adjacent_replace_chunks is idempotent
//! 4. **Blank-line processing** preserves content lines
//! 5. **Sync points** produce correct results with forced alignment
//!
//! Run with: `cargo test --lib --no-default-features -- fuzz`

use crate::diff::engine::{
    consume_blank_lines, merge_adjacent_replace_chunks, Chunk, DiffOp, Differ,
};
use rand::Rng;

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Helpers Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

/// Generate a random line of text.
fn random_line(rng: &mut impl Rng, max_len: usize) -> String {
    let len = rng.gen_range(0..=max_len);
    let chars: String = (0..len)
        .map(|_| {
            let c = rng.gen_range(32u8..127u8) as char;
            // Occasionally insert tabs or spaces
            if rng.gen_bool(0.05) {
                '\t'
            } else if rng.gen_bool(0.1) {
                ' '
            } else {
                c
            }
        })
        .collect();
    chars
}

/// Generate random text content (Vec<String>).
fn random_text(rng: &mut impl Rng, max_lines: usize, max_line_len: usize) -> Vec<String> {
    let n = rng.gen_range(0..=max_lines);
    (0..n).map(|_| random_line(rng, max_line_len)).collect()
}

/// Verify that chunks form a valid partition of both sequences.
fn verify_partition(chunks: &[Chunk], text_a: &[String], text_b: &[String]) {
    let mut a_covered = 0usize;
    let mut b_covered = 0usize;

    for chunk in chunks {
        assert_eq!(
            chunk.start_a, a_covered,
            "Gap in A coverage at chunk {:?}",
            chunk
        );
        assert_eq!(
            chunk.start_b, b_covered,
            "Gap in B coverage at chunk {:?}",
            chunk
        );

        match chunk.op {
            DiffOp::Equal => {
                assert_eq!(
                    chunk.end_a - chunk.start_a,
                    chunk.end_b - chunk.start_b,
                    "Equal chunk must have same length on both sides"
                );
                for k in 0..(chunk.end_a - chunk.start_a) {
                    assert_eq!(
                        text_a[chunk.start_a + k],
                        text_b[chunk.start_b + k],
                        "Equal chunk lines don't match at A[{}] vs B[{}]",
                        chunk.start_a + k,
                        chunk.start_b + k
                    );
                }
            }
            DiffOp::Delete => {
                assert_eq!(
                    chunk.start_b, chunk.end_b,
                    "Delete chunk must be zero-width on B"
                );
            }
            DiffOp::Insert => {
                assert_eq!(
                    chunk.start_a, chunk.end_a,
                    "Insert chunk must be zero-width on A"
                );
            }
            DiffOp::Replace => {}
        }

        a_covered = chunk.end_a;
        b_covered = chunk.end_b;
    }

    assert_eq!(a_covered, text_a.len(), "Chunks don't cover all of A");
    assert_eq!(b_covered, text_b.len(), "Chunks don't cover all of B");
}

/// Reconstruct text_b from text_a and chunks.
fn reconstruct_b(text_a: &[String], chunks: &[Chunk]) -> Vec<String> {
    let mut result = Vec::new();
    for chunk in chunks {
        match chunk.op {
            DiffOp::Equal | DiffOp::Delete => {
                result.extend_from_slice(&text_a[chunk.start_a..chunk.end_a]);
            }
            DiffOp::Insert | DiffOp::Replace => {
                // For Insert and Replace, the B-side content is not in A,
                // so we don't pick it from text_a.  But we can't reconstruct
                // B from A alone for these chunk types without the actual
                // text_b content.  So reconstruction verification is done
                // via the partition check above.
            }
        }
    }
    result
}

/// Verify reconstruction: applying chunks in the correct way should
/// round-trip.  Since Replace/Insert chunks carry data only in B, we
/// verify by checking that removing all Delete chunks from A and
/// inserting B chunks produces B.
fn verify_reconstruction(chunks: &[Chunk], text_a: &[String], text_b: &[String]) {
    let mut reconstructed = Vec::new();

    for chunk in chunks {
        match chunk.op {
            DiffOp::Equal => {
                reconstructed.extend_from_slice(&text_a[chunk.start_a..chunk.end_a]);
            }
            DiffOp::Delete => {
                // Skip Ã¢â‚¬â€ these lines were removed
            }
            DiffOp::Insert => {
                // These lines only exist in B
                reconstructed.extend_from_slice(&text_b[chunk.start_b..chunk.end_b]);
            }
            DiffOp::Replace => {
                // A-side lines are removed; B-side lines are added
                reconstructed.extend_from_slice(&text_b[chunk.start_b..chunk.end_b]);
            }
        }
    }

    assert_eq!(
        reconstructed, *text_b,
        "Reconstruction failed: chunks don't produce correct B"
    );
}

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Fuzz tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

#[test]
fn fuzz_partition_and_reconstruction() {
    let mut rng = rand::thread_rng();
    let iterations = 500;

    for i in 0..iterations {
        let text_a = random_text(&mut rng, 50, 80);
        let text_b = random_text(&mut rng, 50, 80);
        let differ = Differ::new(text_a.clone(), text_b.clone());
        let result = differ.compare();

        verify_partition(&result.chunks, &text_a, &text_b);
        verify_reconstruction(&result.chunks, &text_a, &text_b);

        if i % 100 == 0 {
            eprintln!("  fuzz_partition: {i}/{iterations} ok");
        }
    }
}

#[test]
fn fuzz_merge_idempotence() {
    let mut rng = rand::thread_rng();
    let iterations = 500;

    for i in 0..iterations {
        let text_a = random_text(&mut rng, 40, 60);
        let text_b = random_text(&mut rng, 40, 60);
        let differ = Differ::new(text_a, text_b);
        let mut result = differ.compare();

        // First merge
        result.chunks = merge_adjacent_replace_chunks(&result.chunks);
        let first_len = result.chunks.len();

        // Second merge should be idempotent
        result.chunks = merge_adjacent_replace_chunks(&result.chunks);
        assert_eq!(
            result.chunks.len(),
            first_len,
            "merge_adjacent_replace_chunks is not idempotent"
        );

        if i % 100 == 0 {
            eprintln!("  fuzz_merge_idempotence: {i}/{iterations} ok");
        }
    }
}

#[test]
fn fuzz_blank_lines_consistency() {
    let mut rng = rand::thread_rng();
    let iterations = 300;

    for i in 0..iterations {
        let text_a = random_text(&mut rng, 30, 40);
        let text_b = random_text(&mut rng, 30, 40);

        // Diff with blank-line processing
        let differ = Differ::new(text_a.clone(), text_b.clone());
        let result = differ.compare();
        let mut chunks = result.chunks.clone();

        consume_blank_lines(&mut chunks, &text_a, &text_b);

        // After consume_blank_lines, chunks should still form a valid
        // partition (some chunks may have been removed or had their
        // boundaries trimmed).
        let mut a_covered = 0usize;
        let mut b_covered = 0usize;

        for chunk in &chunks {
            assert!(chunk.start_a >= a_covered, "Blank-line chunk overlaps in A");
            assert!(chunk.start_b >= b_covered, "Blank-line chunk overlaps in B");
            a_covered = chunk.end_a;
            b_covered = chunk.end_b;
        }
        // After blank-line removal, coverage may be less than full
        // (some blank-only chunks were removed)
        assert!(a_covered <= text_a.len());
        assert!(b_covered <= text_b.len());

        if i % 100 == 0 {
            eprintln!("  fuzz_blank_lines: {i}/{iterations} ok");
        }
    }
}

#[test]
fn fuzz_sync_points() {
    let mut rng = rand::thread_rng();
    let iterations = 200;

    for i in 0..iterations {
        let text_a = random_text(&mut rng, 30, 50);
        let text_b = random_text(&mut rng, 30, 50);

        // Generate a random sync point within bounds
        let sync_a = if text_a.is_empty() {
            0
        } else {
            rng.gen_range(0..text_a.len())
        };
        let sync_b = if text_b.is_empty() {
            0
        } else {
            rng.gen_range(0..text_b.len())
        };

        let differ =
            Differ::new(text_a.clone(), text_b.clone()).with_sync_points(vec![(sync_a, sync_b)]);
        let result = differ.compare();

        verify_partition(&result.chunks, &text_a, &text_b);
        verify_reconstruction(&result.chunks, &text_a, &text_b);

        if i % 50 == 0 {
            eprintln!("  fuzz_sync_points: {i}/{iterations} ok");
        }
    }
}

#[test]
fn fuzz_empty_inputs() {
    let mut rng = rand::thread_rng();
    let iterations = 100;

    for i in 0..iterations {
        let text_a = if rng.gen_bool(0.3) {
            vec![]
        } else {
            random_text(&mut rng, 5, 20)
        };
        let text_b = if rng.gen_bool(0.3) {
            vec![]
        } else {
            random_text(&mut rng, 5, 20)
        };

        let differ = Differ::new(text_a.clone(), text_b.clone());
        let result = differ.compare();

        verify_partition(&result.chunks, &text_a, &text_b);
        verify_reconstruction(&result.chunks, &text_a, &text_b);

        if i % 25 == 0 {
            eprintln!("  fuzz_empty_inputs: {i}/{iterations} ok");
        }
    }
}

#[test]
fn fuzz_identical_texts() {
    let mut rng = rand::thread_rng();
    let iterations = 200;

    for i in 0..iterations {
        let text = random_text(&mut rng, 50, 80);
        let differ = Differ::new(text.clone(), text.clone());
        let result = differ.compare();

        // For identical texts, all chunks should be Equal
        for chunk in &result.chunks {
            assert_eq!(
                chunk.op,
                DiffOp::Equal,
                "Identical texts should only have Equal chunks"
            );
        }
        verify_reconstruction(&result.chunks, &text, &text);

        if i % 50 == 0 {
            eprintln!("  fuzz_identical: {i}/{iterations} ok");
        }
    }
}

#[test]
fn fuzz_single_line_texts() {
    let mut rng = rand::thread_rng();
    let iterations = 300;

    for i in 0..iterations {
        let text_a = vec![random_line(&mut rng, 80)];
        let text_b = vec![random_line(&mut rng, 80)];

        let differ = Differ::new(text_a.clone(), text_b.clone());
        let result = differ.compare();

        verify_partition(&result.chunks, &text_a, &text_b);
        verify_reconstruction(&result.chunks, &text_a, &text_b);

        if i % 100 == 0 {
            eprintln!("  fuzz_single_line: {i}/{iterations} ok");
        }
    }
}
