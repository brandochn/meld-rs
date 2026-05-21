#![cfg(feature = "gui")]
//! Emblem cell renderer placeholder — icon tinting for VC file status.
//! Full Cairo-based implementation pending GTK4 CellRenderer support.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcIconState {
    Normal,
    Modified,
    Staged,
    New,
    Conflicted,
    Missing,
    Ignored,
}

impl VcIconState {
    pub fn color(self) -> (f64, f64, f64) {
        match self {
            Self::Normal => (0.5, 0.5, 0.5),
            Self::Modified => (0.2, 0.4, 1.0),
            Self::Staged => (0.1, 0.7, 0.2),
            Self::New => (0.0, 0.7, 0.0),
            Self::Conflicted => (1.0, 0.2, 0.2),
            Self::Missing => (0.8, 0.2, 0.2),
            Self::Ignored => (0.7, 0.7, 0.7),
        }
    }
}

pub struct EmblemCellRenderer;

impl EmblemCellRenderer {
    pub fn new() -> Self {
        Self
    }
}

impl Default for EmblemCellRenderer {
    fn default() -> Self {
        Self::new()
    }
}
