//! Version control system integrations (Git, SVN, Mercurial).
//!
//! Provides a unified interface for interacting with different VCS backends
//! and a factory function [`get_vc`] that auto-detects the VCS in use.

pub mod git;
pub mod hg;
pub mod svn;

use std::path::Path;

use self::git::Git;
use self::hg::Mercurial;
use self::svn::Svn;

/// Errors that can occur during VCS operations.
#[derive(Debug, thiserror::Error)]
pub enum VcError {
    /// The VCS binary was not found on the system PATH.
    #[error("VCS tool '{0}' not found")]
    ToolNotFound(String),

    /// The VCS command returned a non-zero exit code.
    #[error("VCS command failed: {0}")]
    CommandFailed(String),

    /// The output could not be parsed.
    #[error("VCS parse error: {0}")]
    ParseError(String),

    /// No VCS was detected in the given directory.
    #[error("No VCS detected in '{0}'")]
    NoVcsDetected(String),
}

/// Status of a file under version control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VcFileStatus {
    /// File is under version control and unmodified.
    Unmodified,
    /// File has been modified but not yet staged.
    Modified,
    /// File has been staged for commit.
    Staged,
    /// File is new and untracked.
    Untracked,
    /// File is missing from the working tree.
    Missing,
    /// File has merge conflicts.
    Conflicted,
    /// File has been deleted.
    Deleted,
    /// File has been renamed.
    Renamed,
    /// Unknown or error state.
    Unknown,
}

impl VcFileStatus {
    /// Human-readable label.
    pub fn as_str(&self) -> &'static str {
        match self {
            VcFileStatus::Unmodified => "Unmodified",
            VcFileStatus::Modified => "Modified",
            VcFileStatus::Staged => "Staged",
            VcFileStatus::Untracked => "Untracked",
            VcFileStatus::Missing => "Missing",
            VcFileStatus::Conflicted => "Conflicted",
            VcFileStatus::Deleted => "Deleted",
            VcFileStatus::Renamed => "Renamed",
            VcFileStatus::Unknown => "Unknown",
        }
    }
}

/// A single VCS-managed file entry.
#[derive(Debug, Clone)]
pub struct VcEntry {
    /// Relative path of the file.
    pub path: String,
    /// File status.
    pub status: VcFileStatus,
    /// Name of the VCS.
    pub vcs: String,
}

/// Which side of a version-control conflict this represents.
///
/// Used to request the appropriate file version from the VCS backend
/// when resolving merge conflicts in a 3-way comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// The local / working-copy version (CONFLICT_THIS).
    Local,
    /// The common ancestor version.
    Base,
    /// The remote / repository version (CONFLICT_OTHER).
    Remote,
}

/// Trait for version control system backends.
pub trait Vc {
    /// List all changed/interesting files in the given directory.
    fn list_changed_files(&self, path: &str) -> Result<Vec<VcEntry>, VcError>;

    /// Get the diff of a specific file.
    fn file_diff(&self, path: &str) -> Result<String, VcError>;

    /// Get the name of this VCS (e.g. "Git", "SVN").
    fn name(&self) -> &str;

    /// Get the repository (remote) version of a file for comparison.
    /// `cwd` is the repository root, `relative_path` is the file path
    /// relative to the repository root.
    fn get_repo_file(&self, relative_path: &str, cwd: &str) -> Result<String, VcError>;

    /// Get the file path for a specific conflict side.
    /// Returns the path to the extracted file content (may be a temp file or
    /// the working copy itself for the local side).
    fn get_conflict_path(
        &self,
        relative_path: &str,
        cwd: &str,
        kind: ConflictKind,
    ) -> Result<String, VcError>;
}

/// Auto-detect the VCS in the given directory and return the appropriate backend.
pub fn get_vc(path: &str) -> Result<Box<dyn Vc>, VcError> {
    let p = Path::new(path);

    // Check for Git
    if p.join(".git").exists() || has_git_parent(p) {
        return Ok(Box::new(Git::new()));
    }

    // Check for Mercurial
    if p.join(".hg").exists() || has_hg_parent(p) {
        return Ok(Box::new(Mercurial::new()));
    }

    // Check for SVN
    if p.join(".svn").exists() || has_svn_parent(p) {
        return Ok(Box::new(Svn::new()));
    }

    // Default to Git for wider compatibility
    Ok(Box::new(Git::new()))
}

fn has_git_parent(path: &Path) -> bool {
    path.ancestors().any(|a| a.join(".git").exists())
}

fn has_hg_parent(path: &Path) -> bool {
    path.ancestors().any(|a| a.join(".hg").exists())
}

fn has_svn_parent(path: &Path) -> bool {
    path.ancestors().any(|a| a.join(".svn").exists())
}
