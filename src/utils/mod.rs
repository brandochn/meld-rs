//! Utility helpers.
//!
//! Provides encoding detection, file operations, text filtering, and background task scheduling.

pub mod encoding;
pub mod file_utils;
pub mod text_filter;

#[cfg(feature = "gui")]
pub mod task;

/// Return `true` if all elements in the slice are equal.
///
/// Mirrors Python Meld's `misc.all_same`.
pub fn all_same<T: PartialEq>(items: &[T]) -> bool {
    if items.is_empty() {
        return true;
    }
    let first = &items[0];
    items.iter().all(|x| x == first)
}

/// Remove leading and trailing blank lines from text content.
///
/// Multiple consecutive newlines at the start and end are collapsed.
/// Lines containing only whitespace are considered blank.
///
/// Mirrors Python Meld's `dirdiff.remove_blank_lines`.
pub fn remove_blank_lines(text: &[u8]) -> Vec<u8> {
    let mut result = text.to_vec();

    // Trim leading blank lines
    while let Some(pos) = result.iter().position(|&b| b == b'\n') {
        let line = &result[..pos];
        if line.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r') {
            // Entire leading line is whitespace — drop it and the newline
            result.drain(..=pos);
        } else {
            break;
        }
    }

    // Trim trailing blank lines (work backwards)
    while result.len() > 1 {
        let last_newline = result.iter().rposition(|&b| b == b'\n');
        if let Some(pos) = last_newline {
            let trailing = &result[pos + 1..];
            if trailing
                .iter()
                .all(|&b| b == b' ' || b == b'\t' || b == b'\r')
            {
                // Trailing content after last newline is blank — drop from newline
                result.truncate(pos);
                continue;
            }
            // Check if the line between the last and second-last newline is blank
            let prev_newline = result[..pos].iter().rposition(|&b| b == b'\n');
            let line_start = prev_newline.map_or(0, |p| p + 1);
            let line = &result[line_start..pos];
            if line.iter().all(|&b| b == b' ' || b == b'\t' || b == b'\r') {
                result.drain(line_start..=pos);
                continue;
            }
        }
        break;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── all_same ───────────────────────────────────────────────────────

    #[test]
    fn test_all_same_empty() {
        let items: &[i32] = &[];
        assert!(all_same(items));
    }

    #[test]
    fn test_all_same_single() {
        assert!(all_same(&[1]));
        assert!(all_same(&["a"]));
    }

    #[test]
    fn test_all_same_all_equal() {
        assert!(all_same(&[1, 1, 1]));
        assert!(all_same(&["x", "x"]));
    }

    #[test]
    fn test_all_same_not_equal() {
        assert!(!all_same(&[1, 2]));
        assert!(!all_same(&[0, 1, 0]));
        assert!(!all_same(&["a", "b", "c"]));
    }

    // ── remove_blank_lines ─────────────────────────────────────────────

    #[test]
    fn test_remove_blank_lines_empty() {
        assert_eq!(remove_blank_lines(b""), b"");
    }

    #[test]
    fn test_remove_blank_lines_no_blanks() {
        assert_eq!(remove_blank_lines(b"content"), b"content");
    }

    #[test]
    fn test_remove_blank_lines_spaces_only() {
        // A line with only spaces is blank by Python Meld semantics
        assert_eq!(remove_blank_lines(b" "), b" ");
    }

    #[test]
    fn test_remove_blank_lines_single_newline() {
        assert_eq!(remove_blank_lines(b"\n"), b"");
    }

    #[test]
    fn test_remove_blank_lines_leading_newline() {
        assert_eq!(remove_blank_lines(b"\ncontent"), b"content");
    }

    #[test]
    fn test_remove_blank_lines_trailing_newline() {
        assert_eq!(remove_blank_lines(b"content\n"), b"content");
    }

    #[test]
    fn test_remove_blank_lines_multiple_leading_newlines() {
        assert_eq!(remove_blank_lines(b"\n\n\ncontent"), b"content");
    }

    #[test]
    fn test_remove_blank_lines_multiple_trailing_newlines() {
        assert_eq!(remove_blank_lines(b"content\n\n\n"), b"content");
    }

    #[test]
    fn test_remove_blank_lines_blank_between_content() {
        assert_eq!(
            remove_blank_lines(b"content\n\ncontent"),
            b"content\ncontent"
        );
    }

    #[test]
    fn test_remove_blank_lines_spaces_on_blank_lines() {
        assert_eq!(remove_blank_lines(b" \ncontent\n "), b"content");
    }

    #[test]
    fn test_remove_blank_lines_mixed_blanks() {
        assert_eq!(remove_blank_lines(b"\n \ncontent\n \n"), b"content");
    }

    #[test]
    fn test_remove_blank_lines_leading_spaces_before_content() {
        // Spaces before content on the same line are preserved
        assert_eq!(remove_blank_lines(b" content"), b" content");
    }

    #[test]
    fn test_remove_blank_lines_multiple_separate_sections() {
        assert_eq!(
            remove_blank_lines(b"\n\n\ncontent\n\n\ncontent\n\n\n"),
            b"content\ncontent"
        );
    }
}
