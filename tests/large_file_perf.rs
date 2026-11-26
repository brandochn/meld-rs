//! Large-file timing harness for issue #45.
//!
//! This test is `#[ignore]`d so it never runs as part of CI or a normal
//! `cargo test` invocation. Run it manually with the engine only:
//!
//! ```text
//! MELD_PERF_MB=1,10 cargo test --no-default-features --test large_file_perf \
//!     -- --ignored --nocapture
//! ```
//!
//! It generates the reporter's shapes in memory (no file I/O), times the line
//! diff, and predicts where the current inline budget
//! (`MAX_INLINE_LINE_PAIRS_PER_PASS = 20_000`) would stop applying inline
//! highlights. That prediction is used to validate the RESEARCH.md 4.2
//! hypothesis before changing the budget logic.

use std::time::Instant;

use meld_rs::diff::engine::{Chunk, DiffOp, Differ};

/// 100-character base line, matching the reporter's scripts.
fn base_line() -> String {
    "1234567890".repeat(10)
}

/// 100-character fully-different line.
fn diff_line() -> String {
    "abcdefghij".repeat(10)
}

/// Base line with every `5` replaced by `x` (the reporter's `inli` shape).
fn inli_line() -> String {
    base_line().replace('5', "x")
}

#[derive(Clone, Copy, Debug)]
enum Shape {
    /// Every line is `<i>-<base>` (same file).
    UniformSame,
    /// Every line is `<i>-<diff>` (fully different).
    UniformDiff,
    /// 5 same lines followed by 5 changed lines, repeated.
    Block55Diff,
    /// 5 same lines followed by 5 inline-changed lines, repeated.
    Block55Inli,
}

/// Build `mb` MB worth of lines (reporter uses ~10 000 lines per MB).
fn generate(shape: Shape, mb: usize) -> (Vec<String>, Vec<String>) {
    let total = mb.saturating_mul(10_000);
    let base = base_line();
    let full = diff_line();
    let inline = inli_line();

    let mut a = Vec::with_capacity(total);
    let mut b = Vec::with_capacity(total);
    let mut i = 0usize;

    let push_same = |a: &mut Vec<String>, b: &mut Vec<String>, i: &mut usize| {
        *i += 1;
        let line = format!("{}-{}", i, base);
        a.push(line.clone());
        b.push(line);
    };
    let push_changed = |a: &mut Vec<String>, b: &mut Vec<String>, i: &mut usize, text: &str| {
        *i += 1;
        a.push(format!("{}-{}", i, base));
        b.push(format!("{}-{}", i, text));
    };

    match shape {
        Shape::UniformSame => {
            for _ in 0..total {
                push_same(&mut a, &mut b, &mut i);
            }
        }
        Shape::UniformDiff => {
            for _ in 0..total {
                push_changed(&mut a, &mut b, &mut i, &full);
            }
        }
        Shape::Block55Diff => {
            while a.len() < total {
                for _ in 0..5 {
                    push_same(&mut a, &mut b, &mut i);
                }
                for _ in 0..5 {
                    push_changed(&mut a, &mut b, &mut i, &full);
                }
            }
        }
        Shape::Block55Inli => {
            while a.len() < total {
                for _ in 0..5 {
                    push_same(&mut a, &mut b, &mut i);
                }
                for _ in 0..5 {
                    push_changed(&mut a, &mut b, &mut i, &inline);
                }
            }
        }
    }

    (a, b)
}

/// Approximate the inline cost charged by `apply_inline_diff` for one chunk.
fn inline_cost(chunk: &Chunk) -> usize {
    let count_a = chunk.end_a - chunk.start_a;
    let count_b = chunk.end_b - chunk.start_b;
    if count_a == 0 || count_b == 0 {
        return 0;
    }
    let big = count_a.max(count_b);
    if count_a == count_b {
        count_a
    } else if big <= 500 {
        // Nested alignment path charges max(count) up front.
        big
    } else {
        count_a.min(count_b)
    }
}

/// Line (on pane A) where the current 20 000-pair budget would be exhausted.
fn predicted_inline_cutoff(chunks: &[Chunk]) -> Option<usize> {
    const BUDGET: usize = 20_000;
    let mut budget = BUDGET;
    for chunk in chunks {
        if chunk.op != DiffOp::Replace {
            continue;
        }
        let cost = inline_cost(chunk);
        if cost == 0 {
            continue;
        }
        if cost > budget {
            return Some(chunk.start_a);
        }
        budget -= cost;
    }
    None
}

fn run(shape: Shape, mb: usize) {
    let (a, b) = generate(shape, mb);
    let lines = a.len().max(b.len());
    let start = Instant::now();
    let chunks = Differ::new(a, b).compare().chunks;
    let elapsed = start.elapsed();

    let replace: usize = chunks.iter().filter(|c| c.op == DiffOp::Replace).count();
    let old_cutoff = predicted_inline_cutoff(&chunks);
    let scheduled = meld_rs::diff::inline_plan::inline_task_count(&chunks);
    println!(
        "{:?} {} MB: {} lines, {} chunks ({} replace), diff {:?}, old-budget cutoff {:?}, scheduled inline tasks {}",
        shape,
        mb,
        lines,
        chunks.len(),
        replace,
        elapsed,
        old_cutoff,
        scheduled
    );
}

#[test]
#[ignore = "large-file timing harness; run manually with MELD_PERF_MB"]
fn large_file_perf() {
    let sizes: Vec<usize> = std::env::var("MELD_PERF_MB")
        .unwrap_or_else(|_| "1".to_string())
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let sizes = if sizes.is_empty() { vec![1] } else { sizes };

    let shapes = std::env::var("MELD_PERF_SHAPES")
        .unwrap_or_else(|_| "same,diff,block55diff,block55inli".to_string());

    for mb in sizes {
        for shape in shapes.split(',') {
            let shape = match shape.trim() {
                "same" => Shape::UniformSame,
                "diff" => Shape::UniformDiff,
                "block55diff" => Shape::Block55Diff,
                "block55inli" => Shape::Block55Inli,
                other => {
                    eprintln!("unknown shape {other:?}, skipping");
                    continue;
                }
            };
            run(shape, mb);
        }
    }
}
