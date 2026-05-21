//! Mercurial (hg) integration.
//!
//! Uses `hg` CLI commands for version control operations.

use std::process::Command;

use crate::vc::{ConflictKind, Vc, VcEntry, VcError, VcFileStatus};

/// Mercurial version control backend.
#[derive(Debug)]
pub struct Mercurial;

impl Mercurial {
    /// Creates a new Mercurial backend.
    pub fn new() -> Self {
        Self
    }

    fn run_hg(&self, args: &[&str], cwd: &str) -> Result<String, VcError> {
        let output = Command::new("hg")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| VcError::ToolNotFound(format!("hg: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcError::CommandFailed(stderr.into_owned()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Vc for Mercurial {
    fn list_changed_files(&self, path: &str) -> Result<Vec<VcEntry>, VcError> {
        let output = self.run_hg(&["status"], path)?;
        let mut entries = Vec::new();

        for line in output.lines() {
            if line.len() < 2 {
                continue;
            }

            let status_char = line.chars().next().unwrap_or(' ');
            let file_path = line[1..].trim().to_owned();

            let status = match status_char {
                'M' => VcFileStatus::Modified,
                'A' => VcFileStatus::Staged,
                '?' => VcFileStatus::Untracked,
                'R' => VcFileStatus::Deleted,
                '!' => VcFileStatus::Missing,
                _ => VcFileStatus::Unknown,
            };

            entries.push(VcEntry {
                path: file_path,
                status,
                vcs: "Mercurial".into(),
            });
        }

        Ok(entries)
    }

    fn file_diff(&self, path: &str) -> Result<String, VcError> {
        self.run_hg(&["diff", "--", path], "")
    }

    fn name(&self) -> &str {
        "Mercurial"
    }

    fn get_repo_file(&self, relative_path: &str, cwd: &str) -> Result<String, VcError> {
        self.run_hg(&["cat", "-r", "tip", "--", relative_path], cwd)
    }

    fn get_conflict_path(
        &self,
        relative_path: &str,
        _cwd: &str,
        kind: ConflictKind,
    ) -> Result<String, VcError> {
        // Mercurial stores conflict info in .hg/merge/
        // For now, return the repo version as a fallback.
        // Full conflict resolution would use hg resolve --tool internal:dump.
        let _ = kind;
        Err(VcError::CommandFailed(
            "Mercurial conflict path resolution not yet implemented".into(),
        ))
    }
}

impl Default for Mercurial {
    fn default() -> Self {
        Self::new()
    }
}
