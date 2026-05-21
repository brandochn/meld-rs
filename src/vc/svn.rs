//! SVN (Subversion) integration.
//!
//! Uses `svn` CLI commands for version control operations.

use std::process::Command;

use crate::vc::{ConflictKind, Vc, VcEntry, VcError, VcFileStatus};

/// SVN version control backend.
#[derive(Debug)]
pub struct Svn;

impl Svn {
    /// Creates a new SVN backend.
    pub fn new() -> Self {
        Self
    }

    fn run_svn(&self, args: &[&str], cwd: &str) -> Result<String, VcError> {
        let output = Command::new("svn")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| VcError::ToolNotFound(format!("svn: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcError::CommandFailed(stderr.into_owned()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Vc for Svn {
    fn list_changed_files(&self, path: &str) -> Result<Vec<VcEntry>, VcError> {
        let output = self.run_svn(&["status"], path)?;
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
                'D' => VcFileStatus::Deleted,
                'C' => VcFileStatus::Conflicted,
                '!' => VcFileStatus::Missing,
                _ => VcFileStatus::Unknown,
            };

            entries.push(VcEntry {
                path: file_path,
                status,
                vcs: "SVN".into(),
            });
        }

        Ok(entries)
    }

    fn file_diff(&self, path: &str) -> Result<String, VcError> {
        self.run_svn(&["diff", "--", path], "")
    }

    fn name(&self) -> &str {
        "SVN"
    }

    fn get_repo_file(&self, relative_path: &str, cwd: &str) -> Result<String, VcError> {
        self.run_svn(&["cat", "--", relative_path], cwd)
    }

    fn get_conflict_path(
        &self,
        _relative_path: &str,
        _cwd: &str,
        kind: ConflictKind,
    ) -> Result<String, VcError> {
        let _ = kind;
        Err(VcError::CommandFailed(
            "SVN conflict path resolution not yet implemented".into(),
        ))
    }
}

impl Default for Svn {
    fn default() -> Self {
        Self::new()
    }
}
