//! Application settings matching all keys from the original Meld gschema.
//!
//! Covers `org.gnome.meld` (33 keys) and `org.gnome.meld.WindowState` (3 keys).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(#[from] serde_json::Error),
}

/// A single filter entry with a user-visible name, enabled state, and pattern.
///
/// Matches the `(name, enabled, pattern)` tuple format from the original
/// Meld GSchema (`a(sbs)` GVariant type).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilterEntry {
    /// Human-readable label shown in the preferences UI.
    pub name: String,
    /// Whether this filter is currently active.
    pub enabled: bool,
    /// The glob or regex pattern for matching files/text.
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeldSettings {
    // ── GtkSettings overrides ──
    #[serde(default)]
    pub prefer_dark_theme: bool,

    // ── File loading ──
    #[serde(default)]
    pub detect_encodings: Vec<String>,

    // ── Editor properties ──
    #[serde(default = "default_indent_width")]
    pub indent_width: i32,
    #[serde(default)]
    pub insert_spaces_instead_of_tabs: bool,
    #[serde(default)]
    pub show_line_numbers: bool,
    #[serde(default)]
    pub highlight_syntax: bool,
    #[serde(default = "default_style_scheme")]
    pub style_scheme: String,
    #[serde(default)]
    pub enable_space_drawer: bool,
    #[serde(default)]
    pub wrap_mode: String,
    #[serde(default)]
    pub highlight_current_line: bool,
    #[serde(default = "default_true")]
    pub use_system_font: bool,
    #[serde(default = "default_custom_font")]
    pub custom_font: String,

    // ── Overview map ──
    #[serde(default = "default_true")]
    pub show_overview_map: bool,
    #[serde(default = "default_map_style")]
    pub overview_map_style: String,

    // ── File comparison ──
    #[serde(default)]
    pub ignore_blank_lines: bool,

    // ── Diff visualization ──
    #[serde(default = "default_true")]
    pub show_connectors: bool,
    #[serde(default = "default_inline_mode")]
    pub inline_diff_mode: String,
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: f64,

    // ── External helpers ──
    #[serde(default = "default_true")]
    pub use_system_editor: bool,
    #[serde(default)]
    pub custom_editor_command: String,

    // ── Folder comparison ──
    #[serde(default = "default_folder_columns")]
    pub folder_columns: Vec<(String, bool)>,
    #[serde(default)]
    pub folder_ignore_symlinks: bool,
    #[serde(default)]
    pub folder_shallow_comparison: bool,
    #[serde(default = "default_time_resolution")]
    pub folder_time_resolution: i32,
    #[serde(default = "default_true")]
    pub folder_filter_text: bool,
    #[serde(default = "default_folder_status_filters")]
    pub folder_status_filters: Vec<String>,

    // ── VC properties ──
    #[serde(default)]
    pub vc_console_visible: bool,
    #[serde(default = "default_console_pane")]
    pub vc_console_pane_position: i32,
    #[serde(default)]
    pub vc_left_is_local: bool,
    #[serde(default = "default_merge_order")]
    pub vc_merge_file_order: String,
    #[serde(default = "default_true")]
    pub vc_show_commit_margin: bool,
    #[serde(default = "default_commit_margin")]
    pub vc_commit_margin: i32,
    #[serde(default)]
    pub vc_break_commit_message: bool,
    #[serde(default = "default_vc_status_filters")]
    pub vc_status_filters: Vec<String>,

    // ── Filters (predefined named filters with enabled state) ──
    #[serde(default = "default_filename_filters")]
    pub filename_filters: Vec<FilterEntry>,
    #[serde(default = "default_text_filters")]
    pub text_filters: Vec<FilterEntry>,

    // ── Window state ──
    #[serde(default = "default_neg1")]
    pub window_width: i32,
    #[serde(default = "default_neg1")]
    pub window_height: i32,
    #[serde(default)]
    pub window_is_maximized: bool,
}

fn default_indent_width() -> i32 {
    8
}
fn default_style_scheme() -> String {
    "classic".into()
}
fn default_true() -> bool {
    true
}
fn default_custom_font() -> String {
    "monospace, 14".into()
}
fn default_map_style() -> String {
    "chunkmap".into()
}
fn default_folder_columns() -> Vec<(String, bool)> {
    vec![
        ("size".into(), true),
        ("modification time".into(), true),
        ("permissions".into(), false),
    ]
}
fn default_time_resolution() -> i32 {
    100
}
fn default_folder_status_filters() -> Vec<String> {
    vec!["normal".into(), "modified".into(), "new".into()]
}
fn default_console_pane() -> i32 {
    300
}
fn default_merge_order() -> String {
    "remote-merge-local".into()
}
fn default_commit_margin() -> i32 {
    72
}
fn default_vc_status_filters() -> Vec<String> {
    vec!["flatten".into(), "modified".into()]
}
fn default_filename_filters() -> Vec<FilterEntry> {
    vec![
        FilterEntry {
            name: "Backups".into(),
            enabled: true,
            pattern: "#*# .#* ~* *~ *.{orig,bak,swp}".into(),
        },
        FilterEntry {
            name: "OS-Specific Metadata".into(),
            enabled: true,
            pattern: ".DS_Store ._* .Spotlight-V100 .Trashes Thumbs.db Desktop.ini".into(),
        },
        FilterEntry {
            name: "Version Control".into(),
            enabled: true,
            pattern: ".git .svn .hg CVS".into(),
        },
        FilterEntry {
            name: "Binaries".into(),
            enabled: true,
            pattern: "*.{pyc,a,obj,o,so,la,lib,dll,exe}".into(),
        },
        FilterEntry {
            name: "Media".into(),
            enabled: false,
            pattern: "*.{jpg,jpeg,gif,png,bmp,tif,tiff,wav,mp3,ogg,avi,mp4,mov,psd,xcf}".into(),
        },
    ]
}
fn default_text_filters() -> Vec<FilterEntry> {
    vec![
        FilterEntry {
            name: "CVS/SVN Keywords".into(),
            enabled: false,
            pattern: r"\$\w+(:[^\n$]+)?\$".into(),
        },
        FilterEntry {
            name: "C++ Comment".into(),
            enabled: false,
            pattern: "//.*".into(),
        },
        FilterEntry {
            name: "C Comment".into(),
            enabled: false,
            pattern: r"/\*.*?\*/".into(),
        },
        FilterEntry {
            name: "All Whitespace".into(),
            enabled: false,
            pattern: r"[ \t\r\f\v]*".into(),
        },
        FilterEntry {
            name: "Leading Whitespace".into(),
            enabled: false,
            pattern: r"^[ \t\r\f\v]*".into(),
        },
        FilterEntry {
            name: "Trailing Whitespace".into(),
            enabled: false,
            pattern: r"[ \t\r\f\v]*$".into(),
        },
        FilterEntry {
            name: "Script Comment".into(),
            enabled: false,
            pattern: "#.*".into(),
        },
    ]
}
fn default_neg1() -> i32 {
    -1
}

fn default_inline_mode() -> String {
    "characters".into()
}

fn default_similarity_threshold() -> f64 {
    0.6
}

impl Default for MeldSettings {
    fn default() -> Self {
        Self {
            prefer_dark_theme: false,
            detect_encodings: Vec::new(),
            indent_width: 8,
            insert_spaces_instead_of_tabs: false,
            show_line_numbers: false,
            highlight_syntax: false,
            style_scheme: "classic".into(),
            enable_space_drawer: false,
            wrap_mode: "none".into(),
            highlight_current_line: false,
            use_system_font: true,
            custom_font: "monospace, 14".into(),
            show_overview_map: true,
            overview_map_style: "chunkmap".into(),
            ignore_blank_lines: false,
            show_connectors: true,
            inline_diff_mode: "tokens".into(),
            similarity_threshold: 0.6,
            use_system_editor: true,
            custom_editor_command: String::new(),
            folder_columns: default_folder_columns(),
            folder_ignore_symlinks: false,
            folder_shallow_comparison: false,
            folder_time_resolution: 100,
            folder_filter_text: true,
            folder_status_filters: default_folder_status_filters(),
            vc_console_visible: false,
            vc_console_pane_position: 300,
            vc_left_is_local: false,
            vc_merge_file_order: "remote-merge-local".into(),
            vc_show_commit_margin: true,
            vc_commit_margin: 72,
            vc_break_commit_message: false,
            vc_status_filters: default_vc_status_filters(),
            filename_filters: default_filename_filters(),
            text_filters: default_text_filters(),
            window_width: -1,
            window_height: -1,
            window_is_maximized: false,
        }
    }
}

/// Result of resolving pane order from VC settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneOrder {
    /// 2-pane: left = local (working copy), right = remote (repository)
    LocalRemote,
    /// 2-pane: left = remote (repository), right = local (working copy)
    RemoteLocal,
    /// 3-pane: local, merge, remote
    LocalMergeRemote,
    /// 3-pane: remote, merge, local
    RemoteMergeLocal,
}

impl MeldSettings {
    /// Resolve the 2-pane order for VC file comparisons.
    ///
    /// Matches the original Meld `left_is_local` behaviour:
    /// - `true`  → left = local, right = remote
    /// - `false` (default) → left = remote, right = local
    pub fn resolve_two_pane_order(&self) -> PaneOrder {
        if self.vc_left_is_local {
            PaneOrder::LocalRemote
        } else {
            PaneOrder::RemoteLocal
        }
    }

    /// Resolve the 3-pane order for VC merge/conflict comparisons.
    ///
    /// Matches the original Meld `vc-merge-file-order` behaviour:
    /// - `"local-merge-remote"` → local, merge, remote
    /// - `"remote-merge-local"` (default) → remote, merge, local
    pub fn resolve_merge_order(&self) -> PaneOrder {
        match self.vc_merge_file_order.as_str() {
            "local-merge-remote" => PaneOrder::LocalMergeRemote,
            _ => PaneOrder::RemoteMergeLocal,
        }
    }

    /// Return only the enabled filename filter patterns as a `Vec<String>`.
    ///
    /// This is a convenience method for code that only needs the active
    /// patterns (e.g., the diff engine).
    pub fn active_filename_filters(&self) -> Vec<&str> {
        self.filename_filters
            .iter()
            .filter(|f| f.enabled)
            .map(|f| f.pattern.as_str())
            .collect()
    }

    /// Return only the enabled text filter patterns as a `Vec<String>`.
    ///
    /// This is a convenience method for code that only needs the active
    /// patterns (e.g., the diff engine).
    pub fn active_text_filters(&self) -> Vec<&str> {
        self.text_filters
            .iter()
            .filter(|f| f.enabled)
            .map(|f| f.pattern.as_str())
            .collect()
    }

    pub fn load() -> Result<Self, SettingsError> {
        let path = settings_path()?;
        if path.exists() {
            Ok(serde_json::from_str(&std::fs::read_to_string(&path)?)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self) -> Result<(), SettingsError> {
        let path = settings_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn settings_path() -> Result<PathBuf, SettingsError> {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    Ok(config_dir.join("meld-rs").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_match_gschema() {
        let s = MeldSettings::default();
        assert_eq!(s.indent_width, 8);
        assert!(s.use_system_font);
        assert!(s.show_overview_map);
        assert_eq!(s.vc_commit_margin, 72);
        assert_eq!(s.folder_time_resolution, 100);
    }

    #[test]
    fn test_filename_filters_defaults() {
        let s = MeldSettings::default();
        assert_eq!(s.filename_filters.len(), 5);
        assert_eq!(s.filename_filters[0].name, "Backups");
        assert!(s.filename_filters[0].enabled);
        assert_eq!(s.filename_filters[4].name, "Media");
        assert!(!s.filename_filters[4].enabled);
    }

    #[test]
    fn test_text_filters_defaults() {
        let s = MeldSettings::default();
        assert_eq!(s.text_filters.len(), 7);
        assert_eq!(s.text_filters[0].name, "CVS/SVN Keywords");
        assert!(!s.text_filters[0].enabled);
        assert_eq!(s.text_filters[1].name, "C++ Comment");
        assert!(!s.text_filters[1].enabled);
    }

    #[test]
    fn test_active_filters_only_return_enabled() {
        let s = MeldSettings::default();
        // Only the first 4 filename filters are enabled by default
        let active = s.active_filename_filters();
        assert_eq!(active.len(), 4);
        // All text filters are disabled by default
        let active_text = s.active_text_filters();
        assert_eq!(active_text.len(), 0);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let s = MeldSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        let s2: MeldSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(s.indent_width, s2.indent_width);
        assert_eq!(s.style_scheme, s2.style_scheme);
        assert_eq!(s.filename_filters, s2.filename_filters);
        assert_eq!(s.text_filters, s2.text_filters);
    }
}
