//! File-level comparison logic.
//!
//! Mirrors Meld's `dirdiff._files_same` and supporting types.
//! Provides stat-based shallow comparison and content-based deep
//! comparison with configurable blank-line stripping and text filtering.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;

use crate::utils::remove_blank_lines;

/// Result of comparing two files (or lists of files).
///
/// Values match Python Meld's `Same`, `SameFiltered`, `DodgySame`,
/// `DodgyDifferent`, `Different`, `FileError` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCompareResult {
    /// Files are byte-identical.
    Same = 0,
    /// Files became identical after applying filters / blank-line removal.
    SameFiltered = 1,
    /// Shallow (stat-based) comparison says they are the same.
    DodgySame = 2,
    /// Shallow (stat-based) comparison says they differ.
    DodgyDifferent = 3,
    /// Content differs.
    Different = 4,
    /// An error occurred reading one of the files.
    FileError = 5,
}

/// Options controlling how two files are compared.
#[derive(Debug, Clone)]
pub struct FileCompareOptions {
    /// If true, only compare size and timestamp (shallow comparison).
    pub shallow_comparison: bool,
    /// Time resolution in nanoseconds for timestamp comparison.
    /// A value of `-1` means "ignore timestamp entirely".
    pub time_resolution_ns: i64,
    /// If true, strip leading/trailing blank lines before comparing content.
    pub ignore_blank_lines: bool,
    /// If true, apply text filters before comparing content.
    pub apply_text_filters: bool,
    /// Compiled regex patterns for text filtering (only used when
    /// `apply_text_filters` is true).  Empty ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ no filters.
    pub text_filter_patterns: Vec<regex::bytes::Regex>,
}

impl Default for FileCompareOptions {
    fn default() -> Self {
        Self {
            shallow_comparison: false,
            time_resolution_ns: 10_000_000_000, // 10 s ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â Meld's default
            ignore_blank_lines: true,
            apply_text_filters: true,
            text_filter_patterns: Vec::new(),
        }
    }
}

/// File metadata used for shallow (stat-based) comparison.
///
/// Mirrors Python Meld's `StatItem`.
#[derive(Debug, Clone, PartialEq)]
pub struct StatItem {
    /// File mode (type bits only, from `st_mode & S_IFMT`).
    pub mode: u32,
    /// File size in bytes.
    pub size: u64,
    /// Modification time as nanoseconds since the Unix epoch.
    pub mtime_ns: i64,
}

impl StatItem {
    /// Read a `StatItem` from the filesystem.
    pub fn from_path(path: &Path) -> std::io::Result<Self> {
        let meta = fs::metadata(path)?;
        Ok(Self {
            mode: mode_bits(&meta),
            size: meta.len(),
            mtime_ns: mtime_nanos(&meta),
        })
    }

    /// Shallow equality check using size and (optionally) timestamp.
    ///
    /// * `time_resolution_ns == -1` ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ ignore timestamp (size-only check).
    /// * Otherwise, timestamps are quantised to the given resolution and
    ///   compared.
    ///
    /// Mirrors `StatItem.shallow_equal` from Python Meld.
    pub fn shallow_equal(&self, other: &StatItem, time_resolution_ns: i64) -> bool {
        if self.size != other.size {
            return false;
        }
        if time_resolution_ns == -1 {
            return true;
        }
        // Fast-path: if difference > 2 seconds, they're different
        // (avoids expensive division for clearly-different files).
        let diff_ns = (self.mtime_ns - other.mtime_ns).abs();
        if diff_ns > 2_000_000_000 {
            return false;
        }
        let t1 = self.mtime_ns / time_resolution_ns;
        let t2 = other.mtime_ns / time_resolution_ns;
        t1 == t2
    }
}

/// Determine whether two files are the same, using the given options.
///
/// The comparison order is:
/// 1. Stat both files ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â if either fails, return `FileError`
/// 2. If `shallow_comparison`: use `StatItem::shallow_equal` only
/// 3. If sizes differ and no filters ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ return `Different`
/// 4. Read content, optionally strip blank lines / apply text filters
/// 5. Compare filtered content
///
/// Mirrors Python Meld's `_files_same`.
pub fn files_same(path_a: &Path, path_b: &Path, options: &FileCompareOptions) -> FileCompareResult {
    // 1. Stat
    let stat_a = match StatItem::from_path(path_a) {
        Ok(s) => s,
        Err(_) => return FileCompareResult::FileError,
    };
    let stat_b = match StatItem::from_path(path_b) {
        Ok(s) => s,
        Err(_) => return FileCompareResult::FileError,
    };

    // 2. Shallow comparison
    if options.shallow_comparison {
        if stat_a.shallow_equal(&stat_b, options.time_resolution_ns) {
            return FileCompareResult::DodgySame;
        }
        return FileCompareResult::DodgyDifferent;
    }

    // 3. Fast size check (without filters, different size ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ different)
    if stat_a.size != stat_b.size && !options.ignore_blank_lines && !options.apply_text_filters {
        return FileCompareResult::Different;
    }

    // 4. Read content
    let content_a = match read_file_bytes(path_a) {
        Ok(c) => c,
        Err(_) => return FileCompareResult::FileError,
    };
    let content_b = match read_file_bytes(path_b) {
        Ok(c) => c,
        Err(_) => return FileCompareResult::FileError,
    };

    // Fast path: empty files are always same
    if content_a.is_empty() && content_b.is_empty() {
        return FileCompareResult::Same;
    }

    // 5. Apply blank-line removal and/or text filters
    let (filtered_a, filtered_a_was_modified) = apply_options(&content_a, options);
    let (filtered_b, filtered_b_was_modified) = apply_options(&content_b, options);

    let was_filtered = filtered_a_was_modified || filtered_b_was_modified;

    if filtered_a == filtered_b {
        if was_filtered {
            FileCompareResult::SameFiltered
        } else {
            FileCompareResult::Same
        }
    } else {
        FileCompareResult::Different
    }
}

// ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ DirDiff cache ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

/// Cache key for `files_same` results, combining file stats and options.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    size_a: u64,
    mtime_a: i64,
    size_b: u64,
    mtime_b: i64,
    options_hash: u64,
}

/// A thread-local cache for `files_same` results.
/// A thread-safe cache for `files_same` results.
///
/// Uses `Mutex` internally so it can be shared across the background
/// scan thread and the UI thread.
pub struct DirDiffCache {
    cache: Mutex<std::collections::HashMap<CacheKey, FileCompareResult>>,
}

impl DirDiffCache {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn files_same(
        &self,
        path_a: &Path,
        path_b: &Path,
        options: &FileCompareOptions,
    ) -> FileCompareResult {
        let stat_a = match StatItem::from_path(path_a) {
            Ok(s) => s,
            Err(_) => return FileCompareResult::FileError,
        };
        let stat_b = match StatItem::from_path(path_b) {
            Ok(s) => s,
            Err(_) => return FileCompareResult::FileError,
        };
        let key = CacheKey {
            size_a: stat_a.size,
            mtime_a: stat_a.mtime_ns,
            size_b: stat_b.size,
            mtime_b: stat_b.mtime_ns,
            options_hash: options.hash_key(),
        };
        {
            let cache = self.cache.lock().unwrap();
            if let Some(&result) = cache.get(&key) {
                return result;
            }
        }
        let result = files_same(path_a, path_b, options);
        self.cache.lock().unwrap().insert(key, result);
        result
    }

    pub fn clear(&self) {
        self.cache.lock().unwrap().clear();
    }
}

impl Default for DirDiffCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FileCompareOptions {
    fn hash_key(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.shallow_comparison.hash(&mut h);
        self.time_resolution_ns.hash(&mut h);
        self.ignore_blank_lines.hash(&mut h);
        self.apply_text_filters.hash(&mut h);
        self.text_filter_patterns.len().hash(&mut h);
        h.finish()
    }
}

/// Read an entire file into a byte vector, with a size cap to avoid OOM.
fn read_file_bytes(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = fs::File::open(path)?;
    let meta = f.metadata()?;
    let cap = meta.len().min(128 * 1024 * 1024) as usize; // 128 MiB cap
    let mut buf = Vec::with_capacity(cap);
    f.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Apply blank-line removal and text filters. Returns the filtered bytes
/// and a flag indicating whether any modification occurred.
fn apply_options(content: &[u8], options: &FileCompareOptions) -> (Vec<u8>, bool) {
    let mut result = content.to_vec();
    let mut modified = false;

    if options.ignore_blank_lines {
        let stripped = remove_blank_lines(&result);
        if stripped.len() != result.len() {
            modified = true;
        }
        result = stripped;
    }

    if options.apply_text_filters && !options.text_filter_patterns.is_empty() {
        let (filtered, _dims) =
            crate::utils::text_filter::apply_text_filters(&result, &options.text_filter_patterns);
        if filtered.len() != result.len() {
            modified = true;
        }
        result = filtered;
    }

    (result, modified)
}

/// Return the file-type bits from a `Metadata` (mirrors `stat.S_IFMT`).
pub(crate) fn mode_bits(meta: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (meta.mode() & 0o170000) as u32
    }
    #[cfg(not(unix))]
    {
        if meta.is_dir() {
            0o040000
        } else if meta.is_file() {
            0o100000
        } else {
            0
        }
    }
}

/// Return the modification time in nanoseconds since the Unix epoch.
pub(crate) fn mtime_nanos(meta: &fs::Metadata) -> i64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.mtime() * 1_000_000_000 + meta.mtime_nsec()
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        const WINDOWS_EPOCH_OFFSET: u64 = 11_644_473_600_000_000_000;
        let ft = meta.last_write_time();
        if ft == 0 {
            return 0;
        }
        // last_write_time is in 100-nanosecond intervals since 1601.
        // Convert to nanoseconds, then subtract the Unix epoch offset.
        let ft_ns = ft.checked_mul(100).unwrap_or(0);
        if ft_ns <= WINDOWS_EPOCH_OFFSET {
            return 0;
        }
        (ft_ns - WINDOWS_EPOCH_OFFSET) as i64
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        // Fallback: use modified() which returns SystemTime
        if let Ok(t) = meta.modified() {
            if let Ok(d) = t.duration_since(std::time::UNIX_EPOCH) {
                return d.as_nanos() as i64;
            }
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Helper: create a temp file with the given content
    fn temp_file(name: &str, content: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("meld_rs_fc_tests");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content).unwrap();
        path
    }

    fn default_opts() -> FileCompareOptions {
        FileCompareOptions::default()
    }

    fn no_filter_opts() -> FileCompareOptions {
        FileCompareOptions {
            ignore_blank_lines: false,
            apply_text_filters: false,
            ..Default::default()
        }
    }

    fn shallow_opts() -> FileCompareOptions {
        FileCompareOptions {
            shallow_comparison: true,
            ..Default::default()
        }
    }

    // ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ StatItem tests ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

    #[test]
    fn test_stat_item_from_path() {
        let p = temp_file("stat_test.txt", b"hello");
        let stat = StatItem::from_path(&p).unwrap();
        assert_eq!(stat.size, 5);
    }

    #[test]
    fn test_shallow_equal_same_size_and_time() {
        let p = temp_file("sh1.txt", b"abc");
        let s = StatItem::from_path(&p).unwrap();
        assert!(s.shallow_equal(&s, 10_000_000_000));
    }

    #[test]
    fn test_shallow_equal_different_size() {
        let p1 = temp_file("sh2a.txt", b"abc");
        let p2 = temp_file("sh2b.txt", b"abcd");
        let s1 = StatItem::from_path(&p1).unwrap();
        let s2 = StatItem::from_path(&p2).unwrap();
        assert!(!s1.shallow_equal(&s2, 10_000_000_000));
    }

    #[test]
    fn test_shallow_equal_ignore_timestamp() {
        // time_resolution_ns == -1 means ignore timestamp ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â only size matters
        let p1 = temp_file("sh3a.txt", b"xyz");
        // Sleep briefly to get different timestamps
        std::thread::sleep(std::time::Duration::from_millis(10));
        let p2 = temp_file("sh3b.txt", b"xyz");
        let s1 = StatItem::from_path(&p1).unwrap();
        let s2 = StatItem::from_path(&p2).unwrap();
        // With default resolution they might differ; with -1 they're equal
        assert!(s1.shallow_equal(&s2, -1));
    }

    // ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ files_same tests ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬ÃƒÂ¢Ã¢â‚¬ÂÃ¢â€šÂ¬

    #[test]
    fn test_files_same_empty_files() {
        let a = temp_file("empty_a.txt", b"");
        let b = temp_file("empty_b.txt", b"");
        assert_eq!(files_same(&a, &b, &default_opts()), FileCompareResult::Same);
    }

    #[test]
    fn test_files_same_identical() {
        let a = temp_file("id_a.txt", b"hello world");
        let b = temp_file("id_b.txt", b"hello world");
        assert_eq!(
            files_same(&a, &b, &no_filter_opts()),
            FileCompareResult::Same
        );
    }

    #[test]
    fn test_files_same_different() {
        let a = temp_file("diff_a.txt", b"hello world");
        let b = temp_file("diff_b.txt", b"hello WORLD");
        assert_eq!(
            files_same(&a, &b, &no_filter_opts()),
            FileCompareResult::Different
        );
    }

    #[test]
    fn test_files_same_shallow_same_size() {
        // Same size ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ DodgySame with shallow comparison
        let a = temp_file("sh_a.txt", b"AAAA");
        let b = temp_file("sh_b.txt", b"BBBB");
        assert_eq!(
            files_same(&a, &b, &shallow_opts()),
            FileCompareResult::DodgySame
        );
    }

    #[test]
    fn test_files_same_shallow_different_size() {
        let a = temp_file("shd_a.txt", b"AAA");
        let b = temp_file("shd_b.txt", b"BBBB");
        assert_eq!(
            files_same(&a, &b, &shallow_opts()),
            FileCompareResult::DodgyDifferent
        );
    }

    #[test]
    fn test_files_same_nonexistent_file() {
        let a = temp_file("exists.txt", b"x");
        let b = std::path::PathBuf::from("/nonexistent/file.txt");
        assert_eq!(
            files_same(&a, &b, &default_opts()),
            FileCompareResult::FileError
        );
    }

    #[test]
    fn test_files_same_crlf_vs_lf_with_blank_ignore() {
        // CRLF file vs LF file ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ SameFiltered when ignoring blank lines
        let a = temp_file("crlf.txt", b"foo\r\nbar\r\n");
        let b = temp_file("lf.txt", b"foo\nbar\n");
        assert_eq!(
            files_same(&a, &b, &default_opts()),
            FileCompareResult::SameFiltered
        );
    }

    #[test]
    fn test_files_same_crlf_vs_lf_without_blank_ignore() {
        let a = temp_file("crlf2.txt", b"foo\r\nbar\r\n");
        let b = temp_file("lf2.txt", b"foo\nbar\n");
        assert_eq!(
            files_same(&a, &b, &no_filter_opts()),
            FileCompareResult::Different
        );
    }

    #[test]
    fn test_files_same_trailing_blank_lines_filtered() {
        // With trailing blanks on one side ÃƒÂ¢Ã¢â‚¬Â Ã¢â‚¬â„¢ SameFiltered
        let a = temp_file("trail_a.txt", b"foo\nbar\n\n");
        let b = temp_file("trail_b.txt", b"foo\nbar\n");
        assert_eq!(
            files_same(&a, &b, &default_opts()),
            FileCompareResult::SameFiltered
        );
    }

    #[test]
    fn test_files_same_different_after_filters() {
        let a = temp_file("df_a.txt", b"hello world");
        let b = temp_file("df_b.txt", b"hello WORLD");
        // With blank-line ignore only, still different
        assert_eq!(
            files_same(&a, &b, &default_opts()),
            FileCompareResult::Different
        );
    }

    #[test]
    fn test_files_same_large_file_same() {
        // ~40 KB file ÃƒÂ¢Ã¢â€šÂ¬Ã¢â‚¬Â same content
        let chunk = b"d".repeat(4096 * 10 + 1);
        let a = temp_file("big_a.bin", &chunk);
        let b = temp_file("big_b.bin", &chunk);
        assert_eq!(
            files_same(&a, &b, &no_filter_opts()),
            FileCompareResult::Same
        );
    }

    #[test]
    fn test_files_same_large_file_different_first_chunk() {
        let chunk_same = b"d".repeat(4096 * 10 + 1);
        let chunk_different = b"D".repeat(4096 * 10 + 1);
        let a = temp_file("big_c.bin", &chunk_same);
        let b = temp_file("big_d.bin", &chunk_different);
        assert_eq!(
            files_same(&a, &b, &no_filter_opts()),
            FileCompareResult::Different
        );
    }

    #[test]
    fn test_files_same_empty_vs_nonempty() {
        let a = temp_file("empty.txt", b"");
        let b = temp_file("nonempty.txt", b"x");
        assert_eq!(
            files_same(&a, &b, &no_filter_opts()),
            FileCompareResult::Different
        );
    }

    // â”€â”€ DirDiffCache â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_cache_hit_and_miss() {
        let a = temp_file("cache_a.txt", b"cached content");
        let b = temp_file("cache_b.txt", b"cached content");
        let opts = no_filter_opts();
        let cache = DirDiffCache::new();
        let r1 = cache.files_same(&a, &b, &opts);
        assert_eq!(r1, FileCompareResult::Same);
        let r2 = cache.files_same(&a, &b, &opts);
        assert_eq!(r2, FileCompareResult::Same);
    }

    #[test]
    fn test_cache_different_files() {
        let a = temp_file("cd_a.txt", b"AAA");
        let b = temp_file("cd_b.txt", b"BBB");
        let opts = no_filter_opts();
        let cache = DirDiffCache::new();
        let r = cache.files_same(&a, &b, &opts);
        assert_eq!(r, FileCompareResult::Different);
    }

    #[test]
    fn test_cache_clear() {
        let a = temp_file("cc_a.txt", b"X");
        let b = temp_file("cc_b.txt", b"X");
        let opts = no_filter_opts();
        let cache = DirDiffCache::new();
        let _ = cache.files_same(&a, &b, &opts);
        cache.clear();
        let r = cache.files_same(&a, &b, &opts);
        assert_eq!(r, FileCompareResult::Same);
    }
}
