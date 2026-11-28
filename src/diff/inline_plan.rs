//! Inline-highlighting planning helpers (pure, no GTK).
//!
//! The old implementation capped inline work at 20 000 line pairs per pass,
//! which silently stopped highlighting around line 40 000 for the reporter's
//! files (issue #45). The GUI now schedules every eligible `Replace` chunk in
//! time-sliced batches; these predicates keep that decision testable.

use crate::diff::engine::{Chunk, DiffOp};

/// Whether a chunk receives inline (word-level) highlighting.
pub fn should_inline(chunk: &Chunk) -> bool {
    chunk.op == DiffOp::Replace && chunk.end_a > chunk.start_a && chunk.end_b > chunk.start_b
}

/// Number of `(pane, chunk)` inline tasks for a diff.
///
/// Both panes process the same chunks, so this is twice the number of eligible
/// chunks. It is intentionally not capped: the time-sliced driver bounds the
/// work per tick instead of dropping the tail of the file.
pub fn inline_task_count(chunks: &[Chunk]) -> usize {
    chunks.iter().filter(|c| should_inline(c)).count() * 2
}

/// One planned `(pane, chunk)` inline task in document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedTask {
    pub pane: usize,
    pub chunk_index: usize,
    /// End line of the chunk on pane A, used as the progress frontier.
    pub end_a: usize,
}

/// Plan inline tasks in document order: for each chunk, pane 0 then pane 1.
///
/// Chunk-major order keeps the progress frontier monotonic through the
/// document (the previous pane-major order processed all of pane 0 first).
pub fn planned_tasks(chunks: &[Chunk]) -> Vec<PlannedTask> {
    let mut out = Vec::new();
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        if !should_inline(chunk) {
            continue;
        }
        for pane in 0..2 {
            out.push(PlannedTask {
                pane,
                chunk_index,
                end_a: chunk.end_a,
            });
        }
    }
    out
}

/// Fraction (0.0..=1.0) of the document whose inline markup has been applied.
///
/// `frontier_line` is a pane-A line number. When `total_lines` is 0 there is
/// nothing to process, so the fraction is 1.0.
pub fn progress_fraction(frontier_line: usize, total_lines: usize) -> f32 {
    if total_lines == 0 {
        return 1.0;
    }
    (frontier_line as f32 / total_lines as f32).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(start_a: usize, end_a: usize) -> Chunk {
        Chunk {
            start_a,
            end_a,
            start_b: start_a,
            end_b: end_a,
            op: DiffOp::Replace,
        }
    }

    fn equal(start: usize, end: usize) -> Chunk {
        Chunk {
            start_a: start,
            end_a: end,
            start_b: start,
            end_b: end,
            op: DiffOp::Equal,
        }
    }

    #[test]
    fn zero_span_and_equal_chunks_are_not_inlined() {
        let empty_side = Chunk {
            start_a: 0,
            end_a: 0,
            start_b: 0,
            end_b: 5,
            op: DiffOp::Replace,
        };
        let equal = Chunk {
            start_a: 0,
            end_a: 5,
            start_b: 0,
            end_b: 5,
            op: DiffOp::Equal,
        };
        assert!(!should_inline(&empty_side));
        assert!(!should_inline(&equal));
    }

    #[test]
    fn no_line_based_cutoff_for_many_replace_chunks() {
        // 10 000 blocks of 5 changed lines = 100 000 lines. The old 20 000-pair
        // budget stopped after 4 000 blocks (~40 000 lines); every block must
        // now be scheduled.
        let mut chunks = Vec::new();
        for i in 0..10_000 {
            chunks.push(replace(i * 10, i * 10 + 5));
        }
        assert_eq!(inline_task_count(&chunks), 20_000);
    }

    #[test]
    fn planned_tasks_are_chunk_major_and_monotonic() {
        let chunks = vec![replace(0, 5), equal(5, 10), replace(10, 12)];
        let tasks = planned_tasks(&chunks);
        assert_eq!(tasks.len(), 4);
        assert_eq!(
            (tasks[0].pane, tasks[0].chunk_index, tasks[0].end_a),
            (0, 0, 5)
        );
        assert_eq!(
            (tasks[1].pane, tasks[1].chunk_index, tasks[1].end_a),
            (1, 0, 5)
        );
        assert_eq!(
            (tasks[2].pane, tasks[2].chunk_index, tasks[2].end_a),
            (0, 2, 12)
        );
        assert_eq!(
            (tasks[3].pane, tasks[3].chunk_index, tasks[3].end_a),
            (1, 2, 12)
        );
        assert!(tasks.windows(2).all(|w| w[0].end_a <= w[1].end_a));
    }

    #[test]
    fn progress_fraction_edges() {
        assert_eq!(progress_fraction(0, 0), 1.0);
        assert_eq!(progress_fraction(0, 100), 0.0);
        assert_eq!(progress_fraction(50, 100), 0.5);
        assert_eq!(progress_fraction(200, 100), 1.0);
    }
}
