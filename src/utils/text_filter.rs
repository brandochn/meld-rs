//! Text filter engine — dims or removes lines matching user-defined regex
//! patterns, mirroring Meld's `misc.apply_text_filters` and `filediff._filter_text`.
//!
//! Filters operate on the full text content of each pane and produce two
//! outputs:
//!   1. Filtered text (for diff comparison) — matching regions are removed
//!   2. Dim ranges (for visual dimming)     — byte spans where the dim tag
//!      should be applied in the source view
//!
//! Interval merging ensures that overlapping/adjacent matches produce a
//! single contiguous dim region rather than flickering tag boundaries.

use regex::bytes::Regex;

/// A contiguous byte range to dim in the original buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimRange {
    pub start: usize,
    pub end: usize,
}

/// Apply text filters to the given content.
///
/// * `content` — raw bytes of the text buffer
/// * `patterns` — compiled regex patterns (bytes mode, to work with raw text)
/// * `dim_ranges` — output: byte spans in `content` where dimming should apply
///
/// Returns the filtered text (with matching regions removed) for diff comparison.
pub fn apply_text_filters(content: &[u8], patterns: &[Regex]) -> (Vec<u8>, Vec<DimRange>) {
    let mut filter_ranges: Vec<(usize, usize)> = Vec::new();

    for re in patterns {
        for m in re.find_iter(content) {
            let span = m.range();
            if span.start != span.end {
                filter_ranges.push((span.start, span.end));
            }
        }
    }

    let merged = merge_intervals(&mut filter_ranges);

    // Build dim ranges from the merged intervals
    let dim_ranges: Vec<DimRange> = merged
        .iter()
        .map(|&(s, e)| DimRange { start: s, end: e })
        .collect();

    // Build filtered text: splice non-dimmed portions together
    if dim_ranges.is_empty() {
        return (content.to_vec(), dim_ranges);
    }

    let mut filtered = Vec::with_capacity(content.len());
    let mut cursor = 0usize;
    for range in &dim_ranges {
        if cursor < range.start {
            filtered.extend_from_slice(&content[cursor..range.start]);
        }
        cursor = range.end;
    }
    if cursor < content.len() {
        filtered.extend_from_slice(&content[cursor..]);
    }

    (filtered, dim_ranges)
}

/// Merge overlapping and adjacent intervals in-place.
///
/// Returns a new sorted, merged list. The input slice is consumed and
/// sorted first.
fn merge_intervals(ranges: &mut [(usize, usize)]) -> Vec<(usize, usize)> {
    if ranges.is_empty() {
        return Vec::new();
    }

    ranges.sort_unstable_by_key(|r| r.0);

    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];

    for &next in &ranges[1..] {
        if next.0 <= current.1 {
            current.1 = current.1.max(next.1);
        } else {
            merged.push(current);
            current = next;
        }
    }
    merged.push(current);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_patterns_returns_original() {
        let content = b"hello world\nfoo bar\n";
        let patterns: Vec<Regex> = vec![];
        let (filtered, dims) = apply_text_filters(content, &patterns);
        assert_eq!(filtered, content);
        assert!(dims.is_empty());
    }

    #[test]
    fn test_single_pattern_match() {
        let content = b"// comment\nactual code\n// another\n";
        let patterns = vec![Regex::new(r"//.*").unwrap()];
        let (filtered, dims) = apply_text_filters(content, &patterns);
        assert_eq!(dims.len(), 2); // two comment lines dimmed
                                   // Filtered text should exclude the comments
        assert!(!filtered.is_empty());
        assert!(!filtered.windows(2).any(|w| w == b"//"));
    }

    #[test]
    fn test_merge_adjacent_intervals() {
        let mut ranges = vec![(0, 5), (5, 10), (12, 15)];
        let merged = merge_intervals(&mut ranges);
        assert_eq!(merged, vec![(0, 10), (12, 15)]);
    }

    #[test]
    fn test_merge_overlapping_intervals() {
        let mut ranges = vec![(0, 10), (5, 15), (20, 25)];
        let merged = merge_intervals(&mut ranges);
        assert_eq!(merged, vec![(0, 15), (20, 25)]);
    }

    #[test]
    fn test_empty_ranges() {
        let mut ranges: Vec<(usize, usize)> = vec![];
        let merged = merge_intervals(&mut ranges);
        assert!(merged.is_empty());
    }
}
