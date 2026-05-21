//! Recent file management.
//!
//! Tracks recently opened files and comparisons, persisting them
//! via a JSON file so they can be reopened quickly.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Maximum number of recent entries to keep.
const MAX_RECENT: usize = 20;

/// Type of comparison stored in the recent history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecentType {
    /// A file comparison (2 or 3 files).
    File,
    /// A directory comparison.
    Folder,
    /// A 3-way merge.
    Merge,
    /// A version control view.
    VersionControl,
}

/// A single entry in the recent comparisons list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentEntry {
    /// Type of comparison.
    pub comparison_type: RecentType,
    /// File paths involved.
    pub paths: Vec<String>,
    /// Optional display label.
    pub label: Option<String>,
}

/// Manages the list of recently opened comparisons.
#[derive(Debug, Serialize, Deserialize)]
pub struct RecentManager {
    pub entries: VecDeque<RecentEntry>,
}

impl RecentManager {
    /// Load recent entries from disk.
    pub fn load() -> Result<Self, std::io::Error> {
        let path = recent_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            serde_json::from_str(&content)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        } else {
            Ok(Self {
                entries: VecDeque::new(),
            })
        }
    }

    /// Add a new entry to the recent list, moving it to the top if it already exists.
    pub fn add(&mut self, entry: RecentEntry) {
        // Remove duplicate if exists
        self.entries.retain(|e| e.paths != entry.paths);

        self.entries.push_front(entry);

        // Trim to max size
        while self.entries.len() > MAX_RECENT {
            self.entries.pop_back();
        }
    }

    /// Save recent entries to disk.
    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = recent_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, content)
    }

    /// Get all recent entries, most recent first.
    pub fn entries(&self) -> impl Iterator<Item = &RecentEntry> {
        self.entries.iter()
    }

    /// Clear all recent entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

fn recent_path() -> PathBuf {
    let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config"));
    config_dir.join("meld-rs").join("recent.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_entry() {
        let mut mgr = RecentManager {
            entries: VecDeque::new(),
        };
        mgr.add(RecentEntry {
            comparison_type: RecentType::File,
            paths: vec!["a.txt".into(), "b.txt".into()],
            label: None,
        });
        assert_eq!(mgr.entries.len(), 1);
    }

    #[test]
    fn test_dedup_entry() {
        let mut mgr = RecentManager {
            entries: VecDeque::new(),
        };
        let entry = RecentEntry {
            comparison_type: RecentType::File,
            paths: vec!["a.txt".into(), "b.txt".into()],
            label: None,
        };
        mgr.add(entry.clone());
        mgr.add(entry);
        assert_eq!(mgr.entries.len(), 1);
    }
}
