//! Pure rules for resolving where a dropped file list should go.
//!
//! This module deliberately has no GTK dependency so the decision table can be
//! unit-tested without a display server.

/// Region of a comparison page that received the drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDropArea {
    /// A text pane (view or its scrolled container).
    Pane(usize),
    /// An overview map associated with one pane.
    Overview(usize),
    /// A shared page area: middle bar, page background, or tab label.
    Common,
    /// An application-level area: tab-bar background, toolbar, or window.
    App,
}

/// Action to take for a resolved drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDropAction {
    /// Replace the file in a single pane.
    ReplacePane(usize),
    /// Replace every pane in the comparison.
    ReplaceAll,
    /// Delegate to the application-level handler, which opens a new comparison.
    OpenNew,
}

/// Resolve a drop based on where it landed and how many files it contains.
///
/// `None` means the drop is empty and should be ignored. `OpenNew` is the
/// fallback for in-page drops that cannot be handled in place; the application
/// handler is responsible for validating file counts and types before opening.
pub fn resolve_file_drop(
    area: FileDropArea,
    file_count: usize,
    num_panes: usize,
) -> Option<FileDropAction> {
    if file_count == 0 {
        return None;
    }

    match area {
        FileDropArea::Pane(pane) | FileDropArea::Overview(pane) => {
            if file_count == 1 {
                Some(FileDropAction::ReplacePane(pane))
            } else if file_count == num_panes {
                Some(FileDropAction::ReplaceAll)
            } else {
                Some(FileDropAction::OpenNew)
            }
        }
        FileDropArea::Common => {
            if file_count == num_panes {
                Some(FileDropAction::ReplaceAll)
            } else {
                Some(FileDropAction::OpenNew)
            }
        }
        FileDropArea::App => {
            if file_count <= 3 {
                Some(FileDropAction::OpenNew)
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_file_drop, FileDropAction, FileDropArea};

    #[test]
    fn empty_drop_is_ignored() {
        for area in [
            FileDropArea::Pane(0),
            FileDropArea::Overview(0),
            FileDropArea::Common,
            FileDropArea::App,
        ] {
            assert_eq!(resolve_file_drop(area, 0, 2), None);
        }
    }

    #[test]
    fn single_file_replaces_pane_or_falls_back() {
        assert_eq!(
            resolve_file_drop(FileDropArea::Pane(1), 1, 2),
            Some(FileDropAction::ReplacePane(1))
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::Overview(0), 1, 3),
            Some(FileDropAction::ReplacePane(0))
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::Common, 1, 2),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::Common, 1, 3),
            Some(FileDropAction::OpenNew)
        );
    }

    #[test]
    fn pane_count_replaces_all() {
        for area in [
            FileDropArea::Pane(0),
            FileDropArea::Pane(1),
            FileDropArea::Overview(0),
            FileDropArea::Common,
        ] {
            assert_eq!(
                resolve_file_drop(area, 2, 2),
                Some(FileDropAction::ReplaceAll)
            );
            assert_eq!(
                resolve_file_drop(area, 3, 3),
                Some(FileDropAction::ReplaceAll)
            );
        }
    }

    #[test]
    fn mismatched_counts_fall_back_to_new_comparison() {
        assert_eq!(
            resolve_file_drop(FileDropArea::Pane(0), 2, 3),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::Pane(0), 3, 2),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::Common, 2, 3),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::Common, 3, 2),
            Some(FileDropAction::OpenNew)
        );
    }

    #[test]
    fn app_area_accepts_up_to_three_files() {
        assert_eq!(
            resolve_file_drop(FileDropArea::App, 1, 2),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::App, 2, 2),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(
            resolve_file_drop(FileDropArea::App, 3, 3),
            Some(FileDropAction::OpenNew)
        );
        assert_eq!(resolve_file_drop(FileDropArea::App, 4, 2), None);
    }
}
