//! Pure proportional scroll mapping between two panes (no GTK).
//!
//! `FileDiff::map_scroll_proportionally` used this logic inline. Extracting it
//! keeps the arithmetic unit-testable and makes the "line count vs pixel"
//! assumptions explicit (issue #45, RESEARCH 4.4).

use crate::diff::engine::Chunk;

/// Map a master scroll position to a slave scroll position.
///
/// * `master_value` / `master_scroll_max` - current and maximum pixel scroll of
///   the master pane.
/// * `master_line_count` / `slave_line_count` - last line index of each pane.
/// * `slave_scroll_max` - maximum pixel scroll of the slave pane.
/// * `chunks` - diff chunks used to keep changed regions aligned.
#[allow(clippy::too_many_arguments)]
pub fn proportional_scroll_target(
    master_value: f64,
    master_scroll_max: f64,
    master_line_count: f64,
    slave_line_count: f64,
    slave_scroll_max: f64,
    chunks: &[Chunk],
    from_pane: usize,
    to_pane: usize,
) -> f64 {
    if master_scroll_max <= 0.0 {
        return 0.0;
    }

    let master_scroll_frac = (master_value / master_scroll_max.max(1.0)).clamp(0.0, 1.0);
    let master_line = master_scroll_frac * master_line_count;
    let mut slave_line = master_scroll_frac * slave_line_count;

    for chunk in chunks {
        let m_start = if from_pane == 0 {
            chunk.start_a as f64
        } else {
            chunk.start_b as f64
        };
        let m_end = if from_pane == 0 {
            chunk.end_a as f64
        } else {
            chunk.end_b as f64
        };
        let m_range = m_end - m_start;
        if m_range <= 0.0 {
            continue;
        }

        if master_line >= m_start && master_line < m_end {
            let chunk_frac = (master_line - m_start) / m_range;

            let s_start = if to_pane == 0 {
                chunk.start_a as f64
            } else {
                chunk.start_b as f64
            };
            let s_end = if to_pane == 0 {
                chunk.end_a as f64
            } else {
                chunk.end_b as f64
            };
            let s_range = s_end - s_start;

            if s_range > 0.0 {
                slave_line = s_start + chunk_frac * s_range;
            }
            break;
        }
    }

    let slave_scroll_frac = (slave_line / slave_line_count.max(1.0)).clamp(0.0, 1.0);
    slave_scroll_frac * slave_scroll_max.max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::engine::DiffOp;

    fn chunk(start_a: usize, end_a: usize, start_b: usize, end_b: usize) -> Chunk {
        Chunk {
            start_a,
            end_a,
            start_b,
            end_b,
            op: DiffOp::Replace,
        }
    }

    #[test]
    fn proportional_without_chunks() {
        // Halfway through the master maps to halfway through the slave.
        let target = proportional_scroll_target(50.0, 100.0, 99.0, 99.0, 100.0, &[], 0, 1);
        assert!((target - 50.0).abs() < 1e-6);
    }

    #[test]
    fn zero_master_max_is_safe() {
        assert_eq!(
            proportional_scroll_target(10.0, 0.0, 99.0, 99.0, 100.0, &[], 0, 1),
            0.0
        );
    }

    #[test]
    fn clamps_out_of_range_values() {
        let target = proportional_scroll_target(1_000.0, 100.0, 99.0, 99.0, 100.0, &[], 0, 1);
        assert!(target <= 100.0 + 1e-6);

        let target = proportional_scroll_target(-1_000.0, 100.0, 99.0, 99.0, 100.0, &[], 0, 1);
        assert!(target >= -1e-6);
    }

    #[test]
    fn maps_within_a_chunk() {
        // Master line ~5 falls in the A range 0..10 of the chunk; the B side of
        // that chunk is 20..40, so halfway maps to line 30.
        let chunks = vec![chunk(0, 10, 20, 40)];
        // master_line_count = 99, master value fraction 5/99 ~ 0.0505.
        let master_value = 100.0 * (5.0 / 99.0);
        let target =
            proportional_scroll_target(master_value, 100.0, 99.0, 99.0, 99.0, &chunks, 0, 1);
        // slave_line = 30 -> fraction 30/99 -> ~30.
        assert!((target - 30.0).abs() < 0.5, "target was {target}");
    }
}
