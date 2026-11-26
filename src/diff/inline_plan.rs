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
}
