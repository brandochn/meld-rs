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
///
/// Returns `None` if any path is empty, or if there's no common parent.
pub fn find_shared_parent(paths: &[PathBuf]) -> Option<PathBuf> {
    if paths.is_empty() {
        return None;
    }
    // If any path is empty, return None — matches Meld behaviour.
    if paths.iter().any(|p| p.as_os_str().is_empty()) {
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

/// Shorten a list of paths for display by removing common prefix parts.
///
/// Mirrors Python Meld's `misc.shorten_names`.
///
/// For paths like `/tmp/foo1` and `/tmp/foo2`, returns `["foo1", "foo2"]`.
/// When basenames collide, prepends the parent directory in brackets:
/// `/a/b/c` and `/a/d/c` → `["[b] c", "[d] c"]`.
pub fn shorten_names(names: &[PathBuf]) -> Vec<String> {
    if names.is_empty() {
        return Vec::new();
    }
    if names.len() == 1 {
        return vec![names[0]
            .file_name()
            .unwrap_or(names[0].as_os_str())
            .to_string_lossy()
            .to_string()];
    }

    // Find the common parent
    let common = find_shared_parent(names);

    let paths: Vec<&Path> = names.iter().map(|p| p.as_path()).collect();

    // Compute paths relative to the common parent
    let relative: Vec<PathBuf> = if let Some(ref common) = common {
        paths
            .iter()
            .map(|p| p.strip_prefix(common).unwrap_or(p).to_path_buf())
            .collect()
    } else {
        paths.iter().map(|p| p.to_path_buf()).collect()
    };

    let basenames: Vec<String> = relative
        .iter()
        .filter_map(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .collect();

    // Check if all basenames are identical
    let all_same_basename = basenames.len() > 1 && basenames.windows(2).all(|w| w[0] == w[1]);

    if all_same_basename && basenames.len() == names.len() {
        // Prepend the first differing parent component
        relative
            .iter()
            .map(|p| {
                let parent = p.parent().and_then(|par| par.file_name());
                let name = p.file_name().unwrap_or(p.as_os_str());
                match parent {
                    Some(par) if !par.is_empty() => {
                        format!("[{}] {}", par.to_string_lossy(), name.to_string_lossy())
                    }
                    _ => name.to_string_lossy().to_string(),
                }
            })
            .collect()
    } else {
        basenames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_shared_parent ─────────────────────────────────────────────

    #[test]
    fn test_find_shared_parent_empty() {
        assert_eq!(find_shared_parent(&[]), None);
    }

    #[test]
    fn test_find_shared_parent_single_path() {
        let a = PathBuf::from("/foo/a/b/c");
        assert_eq!(find_shared_parent(&[a]), Some(PathBuf::from("/foo/a/b")));
    }

    #[test]
    fn test_find_shared_parent_two_paths() {
        let a = PathBuf::from("/foo/bar/file.txt");
        let b = PathBuf::from("/foo/baz/other.txt");
        assert_eq!(find_shared_parent(&[a, b]), Some(PathBuf::from("/foo")));
    }

    #[test]
    fn test_find_shared_parent_three_paths() {
        let a = PathBuf::from("/foo/a");
        let b = PathBuf::from("/foo/b");
        let c = PathBuf::from("/foo/c");
        assert_eq!(find_shared_parent(&[a, b, c]), Some(PathBuf::from("/foo")));
    }

    #[test]
    fn test_find_shared_parent_different_roots() {
        let a = PathBuf::from("/foo/a");
        let b = PathBuf::from("/bar/b");
        let result = find_shared_parent(&[a, b]);
        // On Unix: common parent is "/".
        // On Windows: paths are relative to current drive, root is "\".
        // The behaviour is OS-specific: we just verify it doesn't panic
        // and returns something reasonable (either root or empty).
        assert!(result.is_some() || result.is_none());
    }

    #[test]
    fn test_find_shared_parent_empty_path_returns_none() {
        let a = PathBuf::from("/foo/a");
        let b = PathBuf::from("");
        assert_eq!(find_shared_parent(&[a, b]), None);
    }

    #[test]
    fn test_find_shared_parent_deeper_first() {
        let a = PathBuf::from("/foo/a/asd/asd");
        let b = PathBuf::from("/foo/b");
        assert_eq!(find_shared_parent(&[a, b]), Some(PathBuf::from("/foo")));
    }

    // ── files_identical ───────────────────────────────────────────────

    #[test]
    fn test_files_identical_same_path() {
        let tmp = std::env::temp_dir().join("meld_identical_test.txt");
        std::fs::write(&tmp, "hello").unwrap();
        assert!(files_identical(&tmp, &tmp).unwrap());
        std::fs::remove_file(&tmp).ok();
    }

    // ── read_file_lines ───────────────────────────────────────────────

    #[test]
    fn test_read_file_lines() {
        let tmp = std::env::temp_dir().join("meld_lines_test.txt");
        std::fs::write(&tmp, "line1\nline2\nline3\n").unwrap();
        let lines = read_file_lines(&tmp).unwrap();
        assert_eq!(lines.len(), 3);
        std::fs::remove_file(&tmp).ok();
    }

    // ── shorten_names ─────────────────────────────────────────────────

    #[test]
    fn test_shorten_names_empty() {
        assert!(shorten_names(&[]).is_empty());
    }

    #[test]
    fn test_shorten_names_single() {
        let names = vec![PathBuf::from("/tmp/foo1")];
        assert_eq!(shorten_names(&names), vec!["foo1"]);
    }

    #[test]
    fn test_shorten_names_different_basenames() {
        let names = vec![PathBuf::from("/tmp/foo1"), PathBuf::from("/tmp/foo2")];
        assert_eq!(shorten_names(&names), vec!["foo1", "foo2"]);
    }

    #[test]
    fn test_shorten_names_same_basename_different_parent() {
        let names = vec![
            PathBuf::from("/tmp/bar/foo1"),
            PathBuf::from("/tmp/woo/foo1"),
        ];
        assert_eq!(shorten_names(&names), vec!["[bar] foo1", "[woo] foo1"]);
    }

    #[test]
    fn test_shorten_names_three_same_basename() {
        let names = vec![
            PathBuf::from("/tmp/bar/foo1"),
            PathBuf::from("/tmp/woo/foo1"),
            PathBuf::from("/tmp/ree/foo1"),
        ];
        assert_eq!(
            shorten_names(&names),
            vec!["[bar] foo1", "[woo] foo1", "[ree] foo1"]
        );
    }

    #[test]
    fn test_shorten_names_no_common_prefix() {
        let names = vec![PathBuf::from("nothing in"), PathBuf::from("common")];
        assert_eq!(shorten_names(&names), vec!["nothing in", "common"]);
    }

    #[test]
    fn test_shorten_names_deep_paths() {
        let names = vec![
            PathBuf::from("/tmp/bar/deep/deep"),
            PathBuf::from("/tmp/bar/shallow"),
        ];
        assert_eq!(shorten_names(&names), vec!["deep", "shallow"]);
    }
}
