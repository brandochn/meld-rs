//! User interface components — GTK4 widgets for all Meld views.

pub mod action_gutter;
pub mod commit_dialog;
pub mod diff_grid;
pub mod diff_map;
pub mod diff_view;
pub mod dir_view;
pub mod emblem_cell_renderer;
pub mod file_button;
pub mod find_bar;
pub mod link_map;
pub mod merge_view;
pub mod msgarea;
pub mod new_diff_tab;
pub mod patch_dialog;
pub mod pathlabel;
pub mod preferences;
pub mod push_dialog;
pub mod recent_selector;
pub mod revert_dialog;
pub mod save_confirm_dialog;
pub mod statusbar;
pub mod tab_manager;
pub mod vc_view;

use crate::window::MeldPage;

/// Factory function that creates a "New Comparison" tab.
pub fn new_diff_tab() -> impl MeldPage {
    new_diff_tab::NewDiffTab::new()
}
