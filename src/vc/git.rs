//! Git integration.
//!
//! Uses `git` CLI commands to obtain file status, diffs, and other
//! version control information.

use std::process::Command;

use crate::vc::{ConflictKind, Vc, VcEntry, VcError, VcFileStatus};

/// Git version control backend.
#[derive(Debug)]
pub struct Git;

impl Git {
    /// Creates a new Git backend.
    pub fn new() -> Self {
        Self
    }

    fn run_git(&self, args: &[&str], cwd: &str) -> Result<String, VcError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .map_err(|e| VcError::ToolNotFound(format!("git: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(VcError::CommandFailed(stderr.into_owned()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl Vc for Git {
    fn list_changed_files(&self, path: &str) -> Result<Vec<VcEntry>, VcError> {
        let output = self.run_git(&["status", "--porcelain"], path)?;
        let mut entries = Vec::new();

        for line in output.lines() {
            if line.len() < 4 {
                continue;
            }

            let status_code = &line[..2];
            let file_path = line[3..].trim().to_owned();

            let status = match status_code {
                " M" | "M " | "MM" => VcFileStatus::Modified,
                "A " | "AM" => VcFileStatus::Staged,
                "??" => VcFileStatus::Untracked,
                " D" | "D " => VcFileStatus::Deleted,
                "R " | "RM" => VcFileStatus::Renamed,
                "UU" | "AA" | "DD" => VcFileStatus::Conflicted,
                _ => VcFileStatus::Unknown,
            };

            entries.push(VcEntry {
                path: file_path,
                status,
                vcs: "Git".into(),
            });
        }

        Ok(entries)
    }

    fn file_diff(&self, path: &str) -> Result<String, VcError> {
        self.run_git(&["diff", "--", path], "")
    }

    fn name(&self) -> &str {
        "Git"
    }

    fn get_repo_file(&self, relative_path: &str, cwd: &str) -> Result<String, VcError> {
        self.run_git(&["show", &format!("HEAD:{relative_path}")], cwd)
    }

    fn get_conflict_path(
        &self,
        relative_path: &str,
        cwd: &str,
        kind: ConflictKind,
    ) -> Result<String, VcError> {
        // Git stores conflict stages:
        //   :1:path = base (ancestor)
        //   :2:path = local (ours / working copy)
        //   :3:path = remote (theirs)
        let stage = match kind {
            ConflictKind::Base => "1",
            ConflictKind::Local => "2",
            ConflictKind::Remote => "3",
        };
        self.run_git(&["show", &format!(":{stage}:{relative_path}")], cwd)
    }
}

impl Default for Git {
    fn default() -> Self {
        Self::new()
    }
}
