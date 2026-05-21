//! File system utilities.
//!
//! Provides helper functions for common file operations like reading
//! file metadata, finding shared parent directories, and resolving paths.

use std::path::{Path, PathBuf};

/// Errors that can occur during file utility operations.
#[derive(Debug, thiserror::Error)]
pub enum FileUtilError {
    /// The file could not be read or accessed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The path does not exist.
    #[error("Path does not exist: {0}")]
    NotFound(String),
}

/// Find the nearest common parent directory shared by all given paths.
pub fn find_shared_parent(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    if paths.len() == 1 {
        return paths[0].parent().map(|p| p.to_path_buf());
    }

    let mut common = paths[0].clone();
    for path in &paths[1..] {
        common = common_ancestor(&common, path);
        if common.as_os_str().is_empty() {
            return None;
        }
    }

    Some(common)
}

/// Find the common ancestor directory of two paths.
fn common_ancestor(a: &Path, b: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    let a_comps: Vec<_> = a.components().collect();
    let b_comps: Vec<_> = b.components().collect();

    for (ac, bc) in a_comps.iter().zip(b_comps.iter()) {
        if ac == bc {
            result.push(ac);
        } else {
            break;
        }
    }

    result
}

/// Check if two files have the same content by comparing their sizes
/// and modification timestamps.
pub fn files_identical(a: &Path, b: &Path) -> Result<bool, FileUtilError> {
    let meta_a = std::fs::metadata(a)?;
    let meta_b = std::fs::metadata(b)?;

    if meta_a.len() != meta_b.len() {
        return Ok(false);
    }

    if meta_a.modified()? == meta_b.modified()? {
        return Ok(true);
    }

    // Fall back to byte-by-byte comparison
    Ok(std::fs::read(a)? == std::fs::read(b)?)
}

/// Read a file and return its contents as a vector of lines.
pub fn read_file_lines(path: &Path) -> Result<Vec<String>, FileUtilError> {
    let content = std::fs::read_to_string(path)?;
    Ok(content.lines().map(|l| l.to_owned()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_shared_parent_two_paths() {
        let a = PathBuf::from("/foo/bar/file.txt");
        let b = PathBuf::from("/foo/baz/other.txt");
        let parent = find_shared_parent(&[a, b]);
        assert_eq!(parent, Some(PathBuf::from("/foo")));
    }

    #[test]
    fn test_find_shared_parent_empty() {
        assert_eq!(find_shared_parent(&[]), None);
    }

    #[test]
    fn test_files_identical_same_path() {
        let tmp = std::env::temp_dir().join("meld_identical_test.txt");
        std::fs::write(&tmp, "hello").unwrap();
        assert!(files_identical(&tmp, &tmp).unwrap());
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_read_file_lines() {
        let tmp = std::env::temp_dir().join("meld_lines_test.txt");
        std::fs::write(&tmp, "line1\nline2\nline3\n").unwrap();
        let lines = read_file_lines(&tmp).unwrap();
        assert_eq!(lines.len(), 3);
        std::fs::remove_file(&tmp).ok();
    }
}
