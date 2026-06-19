#![cfg(feature = "gui")]
//! Centralised diff colour theme.
//!
//! Single source of truth for the diff highlighting colours used across the
//! link map, action gutter, chunk overview map and the per-pane chunk
//! overlays.  Mirrors the original Meld `meld/style.py::get_common_theme()`
//! and the `meld-base` / `meld-dark` GtkSourceView style schemes:
//!
//! ```text
//!   light (meld-base)            dark (meld-dark)
//!   meld:insert  bg=#d0ffa3      bg=#123806
//!                line=#a5ff4c         line=#245515
//!   meld:replace bg=#bdddff      bg=#003266
//!                line=#65b2ff         line=#0053a6
//!   meld:conflict bg=#ffa5a3     bg=#7a2a28
//!                 line=#ff4f4c        line=#ac3b39
//!   meld:inline  bg=#8ac2ff      bg=#24527e
//! ```
//!
//! In Meld terminology the **fill** colour is the light paragraph background
//! drawn behind a whole chunk, and the **line** colour is the more saturated
//! shade used for the 1px outline at the top and bottom of each chunk (and as
//! the stroke for the link-map curves and overview-map blocks).
//!
//! All accessors are theme-aware: they return the dark variants when the GTK
//! application is using a dark theme (see [`is_dark`]).

use gtk4 as gtk;

use crate::diff::engine::DiffOp;

/// An RGB colour with components in the `0.0..=1.0` range.
pub type Rgb = (f64, f64, f64);

// ── meld-base style scheme (light) ──────────────────────────────────
const INSERT_FILL_LIGHT: Rgb = (0.8157, 1.0, 0.6392); // #d0ffa3
const INSERT_LINE_LIGHT: Rgb = (0.6471, 1.0, 0.2980); // #a5ff4c
const REPLACE_FILL_LIGHT: Rgb = (0.7412, 0.8667, 1.0); // #bdddff
const REPLACE_LINE_LIGHT: Rgb = (0.3961, 0.6980, 1.0); // #65b2ff
const CONFLICT_FILL_LIGHT: Rgb = (1.0, 0.6471, 0.6392); // #ffa5a3
const CONFLICT_LINE_LIGHT: Rgb = (1.0, 0.3098, 0.2980); // #ff4f4c

// ── meld-dark style scheme (dark) ───────────────────────────────────
const INSERT_FILL_DARK: Rgb = (0.0706, 0.2196, 0.0235); // #123806
const INSERT_LINE_DARK: Rgb = (0.1412, 0.3333, 0.0824); // #245515
const REPLACE_FILL_DARK: Rgb = (0.0, 0.1961, 0.4); // #003266
const REPLACE_LINE_DARK: Rgb = (0.0, 0.3255, 0.6510); // #0053a6
const CONFLICT_FILL_DARK: Rgb = (0.4784, 0.1647, 0.1569); // #7a2a28
const CONFLICT_LINE_DARK: Rgb = (0.6745, 0.2314, 0.2235); // #ac3b39

/// Whether the GTK application is currently using a dark theme.
pub fn is_dark() -> bool {
    gtk::Settings::default()
        .map(|s| s.is_gtk_application_prefer_dark_theme())
        .unwrap_or(false)
}

/// Fill (light paragraph-background) colour for a chunk of the given op.
///
/// Returns `None` for [`DiffOp::Equal`], which has no highlight.
pub fn fill_color(op: DiffOp) -> Option<Rgb> {
    let dark = is_dark();
    match op {
        // Meld maps `delete` to the same colour as `insert` (see
        // `get_common_theme`), so deleted regions appear green too.
        DiffOp::Insert | DiffOp::Delete => Some(if dark {
            INSERT_FILL_DARK
        } else {
            INSERT_FILL_LIGHT
        }),
        DiffOp::Replace => Some(if dark {
            REPLACE_FILL_DARK
        } else {
            REPLACE_FILL_LIGHT
        }),
        DiffOp::Equal => None,
    }
}

/// Outline (saturated line-background) colour for a chunk of the given op.
///
/// Returns `None` for [`DiffOp::Equal`].
pub fn line_color(op: DiffOp) -> Option<Rgb> {
    let dark = is_dark();
    match op {
        DiffOp::Insert | DiffOp::Delete => Some(if dark {
            INSERT_LINE_DARK
        } else {
            INSERT_LINE_LIGHT
        }),
        DiffOp::Replace => Some(if dark {
            REPLACE_LINE_DARK
        } else {
            REPLACE_LINE_LIGHT
        }),
        DiffOp::Equal => None,
    }
}

/// Conflict fill colour (3-way merge), theme-aware.
pub fn conflict_fill() -> Rgb {
    if is_dark() {
        CONFLICT_FILL_DARK
    } else {
        CONFLICT_FILL_LIGHT
    }
}

/// Conflict outline colour (3-way merge), theme-aware.
pub fn conflict_line() -> Rgb {
    if is_dark() {
        CONFLICT_LINE_DARK
    } else {
        CONFLICT_LINE_LIGHT
    }
}

/// Hex string for the fill colour, suitable for a `GtkTextTag`
/// `paragraph-background` property.  Returns `None` for [`DiffOp::Equal`].
pub fn fill_hex(op: DiffOp) -> Option<&'static str> {
    let dark = is_dark();
    match op {
        DiffOp::Insert | DiffOp::Delete => Some(if dark { "#123806" } else { "#d0ffa3" }),
        DiffOp::Replace => Some(if dark { "#003266" } else { "#bdddff" }),
        DiffOp::Equal => None,
    }
}

/// Hex string for the conflict fill colour.
pub fn conflict_fill_hex() -> &'static str {
    if is_dark() {
        "#7a2a28"
    } else {
        "#ffa5a3"
    }
}

/// Hex string for the `meld:inline` highlight (intra-line changes).
pub fn inline_hex() -> &'static str {
    if is_dark() {
        "#24527e"
    } else {
        "#8ac2ff"
    }
}

/// Hex strings for the differentiated inline tags (delete / insert / replace).
pub fn inline_delete_hex() -> &'static str {
    if is_dark() {
        "#7a2828"
    } else {
        "#ff6666"
    }
}

pub fn inline_insert_hex() -> &'static str {
    if is_dark() {
        "#285728"
    } else {
        "#66ff66"
    }
}

pub fn inline_replace_hex() -> &'static str {
    if is_dark() {
        "#28487a"
    } else {
        "#4488ff"
    }
}

/// Select the GtkSourceView style-scheme id to use for syntax highlighting,
/// preferring Meld's own schemes and falling back to common built-ins.
///
/// `available` should test whether a scheme id is known to the manager.
pub fn preferred_scheme_id(available: impl Fn(&str) -> bool) -> Option<&'static str> {
    let candidates: &[&str] = if is_dark() {
        &["meld-dark", "Adwaita-dark", "solarized-dark", "classic"]
    } else {
        &["meld-base", "Adwaita", "classic"]
    };
    candidates.iter().copied().find(|id| available(id))
}
