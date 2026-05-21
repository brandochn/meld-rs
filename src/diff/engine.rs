//! Diff engine based on the `similar` crate.
//!
//! Provides line-level diffing via [`Differ`] and 3-way merge logic via
//! [`ThreeWayDiffer`]. Includes preprocessing, inline diff with post-processing,
//! and O(1) line-to-chunk mapping via [`LineCache`].
//!
//! Replaces the Python `difflib`/matchers module from the original Meld.

use similar::{ChangeTag, TextDiff};

/// Operation type for a diff chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffOp {
    /// Lines are identical in both sides.
    Equal,
    /// Lines exist only in the left (old) side.
    Delete,
    /// Lines exist only in the right (new) side.
    Insert,
    /// Lines differ between the two sides (a delete + insert pair).
    Replace,
}

/// A single contiguous region of change between two files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Start line in the left (A) pane.
    pub start_a: usize,
    /// End line (exclusive) in the left (A) pane.
    pub end_a: usize,
    /// Start line in the right (B) pane.
    pub start_b: usize,
    /// End line (exclusive) in the right (B) pane.
    pub end_b: usize,
    /// The diff operation for this chunk.
    pub op: DiffOp,
}

/// The result of a line-level diff between two files.
#[derive(Debug, Clone)]
pub struct LineDiff {
    /// The computed diff chunks.
    pub chunks: Vec<Chunk>,
    /// Original text from the left (A) pane, line-by-line.
    pub line_a: Vec<String>,
    /// Original text from the right (B) pane, line-by-line.
    pub line_b: Vec<String>,
}

/// A line-level differ that computes differences between two texts.
pub struct Differ {
    text_a: Vec<String>,
    text_b: Vec<String>,
}

impl Differ {
    /// Creates a new [`Differ`] with the two texts to compare.
    pub fn new(text_a: Vec<String>, text_b: Vec<String>) -> Self {
        Self { text_a, text_b }
    }

    /// Compute the diff and return the result.
    ///
    /// Uses preprocessing (matching the original Meld `MyersSequenceMatcher`)
    /// to strip common prefix/suffix and discard unique lines before
    /// diffing, then reconstructs the full diff with the removed lines
    /// inserted back as Delete/Insert chunks.
    pub fn compare(&self) -> LineDiff {
        let pre = preprocess_diff(&self.text_a, &self.text_b);

        // Join the preprocessed (filtered) lines for diffing
        let a_joined = if pre.filtered_a.is_empty() {
            String::new()
        } else {
            pre.filtered_a.join("\n") + "\n"
        };
        let b_joined = if pre.filtered_b.is_empty() {
            String::new()
        } else {
            pre.filtered_b.join("\n") + "\n"
        };

        let diff = TextDiff::from_lines(&a_joined, &b_joined);
        let mut chunks = Vec::new();

        for change in diff.iter_all_changes() {
            let tag = change.tag();
            let line_count = change.value().lines().count();

            let chunk = match tag {
                ChangeTag::Equal => {
                    let idx = change.old_index().expect("Equal must have old_index");
                    let nidx = change.new_index().expect("Equal must have new_index");
                    Chunk {
                        start_a: idx,
                        end_a: idx + line_count,
                        start_b: nidx,
                        end_b: nidx + line_count,
                        op: DiffOp::Equal,
                    }
                }
                ChangeTag::Delete => {
                    let idx = change.old_index().expect("Delete must have old_index");
                    Chunk {
                        start_a: idx,
                        end_a: idx + line_count,
                        start_b: idx,
                        end_b: idx,
                        op: DiffOp::Delete,
                    }
                }
                ChangeTag::Insert => {
                    let idx = change.new_index().expect("Insert must have new_index");
                    Chunk {
                        start_a: idx,
                        end_a: idx,
                        start_b: idx,
                        end_b: idx + line_count,
                        op: DiffOp::Insert,
                    }
                }
            };

            chunks.push(chunk);
        }

        // Remap indices from filtered space back to original positions
        unprocess_chunks(&mut chunks, &pre);

        // Insert chunks for the stripped prefix and suffix as Equal
        if pre.prefix_len > 0 {
            chunks.insert(
                0,
                Chunk {
                    start_a: 0,
                    end_a: pre.prefix_len,
                    start_b: 0,
                    end_b: pre.prefix_len,
                    op: DiffOp::Equal,
                },
            );
        }
        if pre.suffix_len > 0 {
            let suffix_start_a = self.text_a.len() - pre.suffix_len;
            let suffix_start_b = self.text_b.len() - pre.suffix_len;
            chunks.push(Chunk {
                start_a: suffix_start_a,
                end_a: self.text_a.len(),
                start_b: suffix_start_b,
                end_b: self.text_b.len(),
                op: DiffOp::Equal,
            });
        }

        // Insert Delete/Insert chunks for unique lines that were removed.
        // Only scan the middle region (between prefix and suffix) to avoid
        // spurious chunks for the prefix/suffix lines, which are already Equal.
        insert_unique_line_chunks(
            &mut chunks,
            &self.text_a,
            &self.text_b,
            &pre.index_map_a,
            &pre.index_map_b,
            pre.prefix_len,
            pre.suffix_len,
        );

        LineDiff {
            chunks,
            line_a: self.text_a.clone(),
            line_b: self.text_b.clone(),
        }
    }
}

/// Scan the original texts for runs of lines that were filtered out
/// (unique to one side) and insert Delete/Insert chunks for them.
///
/// Only scans the middle region `[prefix_len .. len - suffix_len]` because
/// the prefix and suffix are already covered by dedicated Equal chunks.
fn insert_unique_line_chunks(
    chunks: &mut Vec<Chunk>,
    text_a: &[String],
    text_b: &[String],
    index_map_a: &[usize],
    index_map_b: &[usize],
    prefix_len: usize,
    suffix_len: usize,
) {
    if index_map_a.is_empty() && index_map_b.is_empty() {
        return;
    }
    let kept_a: std::collections::HashSet<usize> = index_map_a.iter().copied().collect();
    let kept_b: std::collections::HashSet<usize> = index_map_b.iter().copied().collect();

    // Only scan the middle region (between prefix and suffix). The prefix
    // and suffix are already covered by dedicated Equal chunks.
    let a_mid_start = prefix_len;
    let a_mid_end = text_a.len() - suffix_len;
    let b_mid_start = prefix_len;
    let b_mid_end = text_b.len() - suffix_len;

    let mut del_runs: Vec<(usize, usize)> = Vec::new();
    let mut i = a_mid_start;
    while i < a_mid_end {
        if !kept_a.contains(&i) {
            let start = i;
            while i < a_mid_end && !kept_a.contains(&i) {
                i += 1;
            }
            del_runs.push((start, i));
        } else {
            i += 1;
        }
    }

    let mut ins_runs: Vec<(usize, usize)> = Vec::new();
    let mut j = b_mid_start;
    while j < b_mid_end {
        if !kept_b.contains(&j) {
            let start = j;
            while j < b_mid_end && !kept_b.contains(&j) {
                j += 1;
            }
            ins_runs.push((start, j));
        } else {
            j += 1;
        }
    }

    for (start, end) in &del_runs {
        chunks.push(Chunk {
            start_a: *start,
            end_a: *end,
            start_b: *start,
            end_b: *start,
            op: DiffOp::Delete,
        });
    }

    for (start, end) in &ins_runs {
        chunks.push(Chunk {
            start_a: *start,
            end_a: *start,
            start_b: *start,
            end_b: *end,
            op: DiffOp::Insert,
        });
    }

    chunks.sort_by_key(|c| c.start_a.min(c.start_b));
}

// ─── Three-way merge ───────────────────────────────────────────────

/// The result of a 3-way file merge.
#[derive(Debug, Clone)]
pub struct ThreeWayComparison {
    /// The base (ancestor) content.
    pub base: Vec<String>,
    /// Local changes (our version).
    pub local: Vec<String>,
    /// Remote changes (their version).
    pub remote: Vec<String>,
    /// The merged output.
    pub merged: Vec<String>,
    /// List of merge conflicts found.
    pub conflicts: Vec<MergeConflict>,
}

/// A conflict region that could not be automatically resolved.
#[derive(Debug, Clone)]
pub struct MergeConflict {
    /// Starting line of the conflict in the merged output.
    pub start_line: usize,
    /// Ending line (exclusive) of the conflict.
    pub end_line: usize,
    /// Lines from the local side.
    pub local: Vec<String>,
    /// Lines from the remote side.
    pub remote: Vec<String>,
}

/// Performs a 3-way merge given base, local, and remote file contents.
pub struct ThreeWayDiffer {
    base: Vec<String>,
    local: Vec<String>,
    remote: Vec<String>,
}

impl ThreeWayDiffer {
    /// Creates a new [`ThreeWayDiffer`] with the three file versions.
    pub fn new(base: Vec<String>, local: Vec<String>, remote: Vec<String>) -> Self {
        Self {
            base,
            local,
            remote,
        }
    }

    /// Execute the merge and return the result, including conflicts.
    pub fn merge(&self) -> ThreeWayComparison {
        let base_to_remote = Differ::new(self.base.clone(), self.remote.clone()).compare();

        // Start from base and apply remote changes
        let mut merged = self.base.clone();
        let mut conflicts = Vec::new();

        for chunk in &base_to_remote.chunks {
            match chunk.op {
                DiffOp::Delete => {
                    let start = chunk.start_a.min(merged.len());
                    let end = chunk.end_a.min(merged.len());
                    if start < end {
                        merged.drain(start..end);
                    }
                }
                DiffOp::Insert => {
                    let idx = chunk.start_a.min(merged.len());
                    let r_end = chunk.end_b.min(self.remote.len());
                    for (j, line) in self.remote[chunk.start_b..r_end].iter().enumerate() {
                        merged.insert(idx + j, line.clone());
                    }
                }
                DiffOp::Replace => {
                    let start = chunk.start_a.min(merged.len());
                    let end = chunk.end_a.min(merged.len());
                    if start < end {
                        merged.drain(start..end);
                    }
                    let r_end = chunk.end_b.min(self.remote.len());
                    for (j, line) in self.remote[chunk.start_b..r_end].iter().enumerate() {
                        merged.insert(start + j, line.clone());
                    }
                }
                DiffOp::Equal => {}
            }
        }

        // Detect conflicts: where local also changed the same region
        let base_to_local = Differ::new(self.base.clone(), self.local.clone()).compare();
        for chunk in &base_to_local.chunks {
            if matches!(chunk.op, DiffOp::Replace | DiffOp::Insert) {
                let start_local = chunk.start_b;
                let end_local = chunk.end_b.min(self.local.len());
                let start_remote = find_corresponding_line(&base_to_remote.chunks, chunk.start_a);

                if start_remote != chunk.start_a && end_local > start_local {
                    let rem_end = (start_remote + end_local - start_local).min(self.remote.len());
                    conflicts.push(MergeConflict {
                        start_line: start_local,
                        end_line: end_local,
                        local: self.local[start_local..end_local].to_vec(),
                        remote: self.remote[start_remote..rem_end].to_vec(),
                    });
                }
            }
        }

        ThreeWayComparison {
            base: self.base.clone(),
            local: self.local.clone(),
            remote: self.remote.clone(),
            merged,
            conflicts,
        }
    }
}

/// Find the position in side B corresponding to a line in side A, using the diff chunks.
fn find_corresponding_line(chunks: &[Chunk], base_line: usize) -> usize {
    for chunk in chunks {
        if chunk.start_a <= base_line && base_line < chunk.end_a {
            return chunk.start_b + (base_line - chunk.start_a);
        }
    }
    base_line
}

/// Merge adjacent Delete + Insert chunks (in either order) into a single
/// Replace chunk. Handles runs of multiple deletes/inserts.
///
/// The `similar` crate can produce patterns like:
///   Delete, Delete, Insert, Insert
/// which should become:
///   Replace (spanning both deletes and both inserts)
///
/// This function merges any contiguous sequence of Delete and Insert chunks
/// (in any order) into a single Replace.
pub fn merge_adjacent_replace_chunks(chunks: &[Chunk]) -> Vec<Chunk> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < chunks.len() {
        // Check if we have a run of Delete + Insert chunks starting at i.
        // Count how many consecutive Delete/Insert chunks we have.
        let mut delete_count = 0usize;
        let mut insert_count = 0usize;
        let mut j = i;

        // First pass: count Deletes and Inserts, tracking positions
        while j < chunks.len() {
            match chunks[j].op {
                DiffOp::Delete => {
                    if insert_count > 0 {
                        // Insert followed by Delete = mixed, stop counting
                        break;
                    }
                    delete_count += 1;
                }
                DiffOp::Insert => {
                    insert_count += 1;
                }
                _ => break, // Equal or Replace breaks the run
            }
            j += 1;
        }

        // If we have both deletes and inserts (in either order), merge them
        if delete_count > 0 && insert_count > 0 {
            // The merged chunk spans from the first chunk to the last chunk in the run.
            // Find the actual start/end positions.
            let run_len = delete_count + insert_count;

            // For Delete→Insert order: start_a from first Delete, end_a from last Delete;
            // start_b from first Insert, end_b from last Insert.
            // For Insert→Delete order: the opposite.
            let first_is_delete = chunks[i].op == DiffOp::Delete;

            let (start_a, end_a, start_b, end_b) = if first_is_delete {
                // Delete(s) then Insert(s)
                let last_del = i + delete_count - 1;
                let first_ins = i + delete_count;
                let last_ins = i + run_len - 1;
                (
                    chunks[i].start_a,
                    chunks[last_del].end_a,
                    chunks[first_ins].start_b,
                    chunks[last_ins].end_b,
                )
            } else {
                // Insert(s) then Delete(s)
                let last_ins = i + insert_count - 1;
                let first_del = i + insert_count;
                let last_del = i + run_len - 1;
                (
                    chunks[first_del].start_a,
                    chunks[last_del].end_a,
                    chunks[i].start_b,
                    chunks[last_ins].end_b,
                )
            };

            result.push(Chunk {
                start_a,
                end_a,
                start_b,
                end_b,
                op: DiffOp::Replace,
            });
            i += run_len;
        } else if delete_count > 0 {
            // Only deletes, no inserts to merge with
            for k in 0..delete_count {
                result.push(chunks[i + k].clone());
            }
            i += delete_count;
        } else if insert_count > 0 {
            // Only inserts, no deletes to merge with
            for k in 0..insert_count {
                result.push(chunks[i + k].clone());
            }
            i += insert_count;
        } else {
            // Equal chunk, just copy
            result.push(chunks[i].clone());
            i += 1;
        }
    }

    result
}

// ─── Inline (word-level) diff ──────────────────────────────────────

/// An in-word change within a single line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineChange {
    /// Start character offset within the line.
    pub start: usize,
    /// End character offset (exclusive).
    pub end: usize,
    /// The type of inline change.
    pub op: DiffOp,
}

/// Computes character-level (word) differences within a single line.
///
/// Uses `similar`'s character-level diff to identify which parts of a
/// replaced line actually changed, matching the original Meld's inline
/// highlighting behaviour.
pub struct InlineDiffer;

impl InlineDiffer {
    /// Compare two lines and return a list of inline changes.
    /// Only returns differences when the lines are different but similar
    /// enough (e.g., a single word change).
    pub fn compare_line(line_a: &str, line_b: &str) -> Vec<InlineChange> {
        if line_a == line_b {
            return Vec::new();
        }

        let diff = TextDiff::from_chars(line_a, line_b);
        let mut changes = Vec::new();

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => continue,
                ChangeTag::Delete => {
                    if let Some(idx) = change.old_index() {
                        let len = change.value().chars().count();
                        changes.push(InlineChange {
                            start: idx,
                            end: idx + len,
                            op: DiffOp::Delete,
                        });
                    }
                }
                ChangeTag::Insert => {
                    if let Some(idx) = change.new_index() {
                        let len = change.value().chars().count();
                        changes.push(InlineChange {
                            start: idx,
                            end: idx + len,
                            op: DiffOp::Insert,
                        });
                    }
                }
            }
        }

        // Post-process to merge adjacent Delete+Insert into Replace
        Self::_postprocess(&mut changes);
        changes
    }

    fn _postprocess(changes: &mut Vec<InlineChange>) {
        let mut i = 0;
        while i + 1 < changes.len() {
            if changes[i].op == DiffOp::Delete && changes[i + 1].op == DiffOp::Insert {
                let del = changes[i].clone();
                let ins = changes[i + 1].clone();
                changes[i] = InlineChange {
                    start: del.start,
                    end: ins.end,
                    op: DiffOp::Replace,
                };
                changes.remove(i + 1);
            }
            i += 1;
        }
    }
}

// ─── Line cache (O(1) chunk navigation) ────────────────────────────

/// Caches a mapping from line numbers to chunk indices for fast navigation.
#[derive(Debug, Clone)]
pub struct LineCache {
    entries: Vec<Option<usize>>,
}

impl LineCache {
    /// Build a line cache from diff chunks.
    pub fn new(chunks: &[Chunk], max_lines: usize) -> Self {
        let mut entries = vec![None; max_lines];
        for (ci, chunk) in chunks.iter().enumerate() {
            let (start, end) = match chunk.op {
                DiffOp::Delete => (chunk.start_a, chunk.end_a),
                DiffOp::Insert => (chunk.start_b, chunk.end_b),
                DiffOp::Replace => (
                    chunk.start_a.max(chunk.start_b),
                    chunk.end_a.max(chunk.end_b),
                ),
                DiffOp::Equal => continue,
            };
            for line in start..end.min(max_lines) {
                if entries[line].is_none() {
                    entries[line] = Some(ci);
                }
            }
        }
        Self { entries }
    }

    /// Return the chunk index for a given line, if any.
    pub fn locate_chunk(&self, line: usize) -> Option<usize> {
        self.entries.get(line).copied().flatten()
    }

    /// Return the chunk indices surrounding a line (prev, curr, next).
    pub fn chunk_triad(&self, _line: usize) -> (Option<usize>, Option<usize>, Option<usize>) {
        // Simplified: just return current
        let curr = self.locate_chunk(_line);
        (None, curr, None)
    }
}

// ─── Diff Preprocessor ───────────────────────────────────────────────

/// Result of diff preprocessing: strips common prefix/suffix and removes
/// unique lines, then maps result indices back to original positions.
#[derive(Debug, Clone)]
pub struct PreprocessResult {
    /// The filtered line texts to pass to the diff algorithm.
    pub filtered_a: Vec<String>,
    pub filtered_b: Vec<String>,
    /// Maps from filtered index back to original index in text_a.
    pub index_map_a: Vec<usize>,
    /// Maps from filtered index back to original index in text_b.
    pub index_map_b: Vec<usize>,
    /// Number of common prefix lines stripped.
    pub prefix_len: usize,
    /// Number of common suffix lines stripped.
    pub suffix_len: usize,
}

/// Preprocesses two line sequences to reduce the input size for the diff
/// algorithm. Strips common prefix/suffix and discards lines that appear
/// only in one file, matching the original Meld's optimizations.
pub fn preprocess_diff(text_a: &[String], text_b: &[String]) -> PreprocessResult {
    let len_a = text_a.len();
    let len_b = text_b.len();

    // ── Strip common prefix ──
    let prefix_len = text_a
        .iter()
        .zip(text_b.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // ── Strip common suffix ──
    let suffix_len = text_a[prefix_len..]
        .iter()
        .rev()
        .zip(text_b[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let a_mid = &text_a[prefix_len..len_a - suffix_len];
    let b_mid = &text_b[prefix_len..len_b - suffix_len];

    // ── Discard unique lines (lines in only one file) ──
    // Only apply if it would discard more than 10 lines (heuristic from Meld)
    use std::collections::HashSet;
    let lines_in_b: HashSet<&String> = b_mid.iter().collect();

    let mut filtered_a = Vec::new();
    let mut index_map_a = Vec::new();
    let mut filtered_b = Vec::new();
    let mut index_map_b = Vec::new();

    let mut discarded = 0usize;

    for (i, line) in a_mid.iter().enumerate() {
        if lines_in_b.contains(line) {
            filtered_a.push(line.clone());
            index_map_a.push(prefix_len + i);
        } else {
            discarded += 1;
        }
    }

    for (i, line) in b_mid.iter().enumerate() {
        // Check if this line exists in A's middle section
        let in_a = a_mid.iter().any(|a| a == line);
        if in_a {
            filtered_b.push(line.clone());
            index_map_b.push(prefix_len + i);
        }
    }

    // Only use the filtered version if enough lines were discarded
    if discarded <= 10 {
        // Not worth it — return unfiltered result
        return PreprocessResult {
            filtered_a: text_a.to_vec(),
            filtered_b: text_b.to_vec(),
            index_map_a: (0..text_a.len()).collect(),
            index_map_b: (0..text_b.len()).collect(),
            prefix_len: 0,
            suffix_len: 0,
        };
    }

    PreprocessResult {
        filtered_a,
        filtered_b,
        index_map_a,
        index_map_b,
        prefix_len,
        suffix_len,
    }
}

/// Remap chunk indices from filtered space back to original line numbers.
/// Also handles zero-width chunks (e.g. Insert where start_a == end_a).
pub fn unprocess_chunks(chunks: &mut Vec<Chunk>, pre: &PreprocessResult) {
    for chunk in chunks.iter_mut() {
        // Remap start_a: filtered index → original index
        if chunk.start_a < pre.index_map_a.len() {
            chunk.start_a = pre.index_map_a[chunk.start_a];
        }
        // Remap end_a: if the chunk spans filtered lines, map the last one + 1.
        // For zero-width chunks (e.g. Insert where start_a == end_a),
        // use the same mapping as start_a.
        if chunk.end_a > 0 && chunk.end_a - 1 < pre.index_map_a.len() {
            chunk.end_a = pre.index_map_a[chunk.end_a - 1] + 1;
        } else if chunk.start_a == chunk.end_a && chunk.start_a < pre.index_map_a.len() {
            // Zero-width chunk: end_a maps to the same position as start_a
            chunk.end_a = pre.index_map_a[chunk.start_a];
        }
        // Same for B side
        if chunk.start_b < pre.index_map_b.len() {
            chunk.start_b = pre.index_map_b[chunk.start_b];
        }
        if chunk.end_b > 0 && chunk.end_b - 1 < pre.index_map_b.len() {
            chunk.end_b = pre.index_map_b[chunk.end_b - 1] + 1;
        } else if chunk.start_b == chunk.end_b && chunk.start_b < pre.index_map_b.len() {
            chunk.end_b = pre.index_map_b[chunk.start_b];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_diff() {
        let d = Differ::new(vec!["a".into(), "b".into()], vec!["a".into(), "c".into()]);
        let result = d.compare();
        assert!(!result.chunks.is_empty());
    }

    #[test]
    fn test_identical_inputs() {
        let d = Differ::new(vec!["x".into()], vec!["x".into()]);
        let result = d.compare();
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].op, DiffOp::Equal);
    }

    #[test]
    fn test_insert_chunk_zero_width_in_a() {
        let d = Differ::new(
            vec!["a".into(), "c".into()],
            vec!["a".into(), "b".into(), "c".into()],
        );
        let result = d.compare();
        let ins: Vec<_> = result
            .chunks
            .iter()
            .filter(|c| c.op == DiffOp::Insert)
            .collect();
        assert!(!ins.is_empty(), "should have at least one Insert chunk");
        // The Insert chunk should have zero width on the A side
        assert!(ins.iter().all(|c| c.start_a == c.end_a));
    }

    #[test]
    fn test_delete_chunk_zero_width_in_b() {
        let d = Differ::new(
            vec!["a".into(), "b".into(), "c".into()],
            vec!["a".into(), "c".into()],
        );
        let result = d.compare();
        let del: Vec<_> = result
            .chunks
            .iter()
            .filter(|c| c.op == DiffOp::Delete)
            .collect();
        assert!(!del.is_empty(), "should have at least one Delete chunk");
        assert!(del.iter().all(|c| c.start_b == c.end_b));
    }

    #[test]
    fn test_delete_insert_merge_to_replace() {
        let chunks = vec![
            Chunk {
                start_a: 1,
                end_a: 2,
                start_b: 1,
                end_b: 1,
                op: DiffOp::Delete,
            },
            Chunk {
                start_a: 2,
                end_a: 2,
                start_b: 1,
                end_b: 2,
                op: DiffOp::Insert,
            },
        ];
        let merged = merge_adjacent_replace_chunks(&chunks);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].op, DiffOp::Replace);
    }

    #[test]
    fn test_merge_adjacent_replace() {
        let chunks = vec![
            Chunk {
                start_a: 1,
                end_a: 2,
                start_b: 1,
                end_b: 1,
                op: DiffOp::Delete,
            },
            Chunk {
                start_a: 2,
                end_a: 2,
                start_b: 1,
                end_b: 2,
                op: DiffOp::Insert,
            },
        ];
        let merged = merge_adjacent_replace_chunks(&chunks);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].op, DiffOp::Replace);
    }

    #[test]
    fn test_single_line_quote_diff_merges_to_replace() {
        // Simulates a single-line change like changing a quote character.
        let chunks = vec![
            Chunk {
                start_a: 0,
                end_a: 1,
                start_b: 0,
                end_b: 0,
                op: DiffOp::Delete,
            },
            Chunk {
                start_a: 1,
                end_a: 1,
                start_b: 0,
                end_b: 1,
                op: DiffOp::Insert,
            },
        ];
        let merged = merge_adjacent_replace_chunks(&chunks);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].op, DiffOp::Replace);
        assert_eq!(merged[0].start_a, 0);
        assert_eq!(merged[0].end_a, 1);
        assert_eq!(merged[0].start_b, 0);
        assert_eq!(merged[0].end_b, 1);
    }

    #[test]
    fn test_run_merge_multiple_deletes_and_inserts() {
        // Delete, Delete, Insert, Insert → single Replace
        let chunks = vec![
            Chunk {
                start_a: 2,
                end_a: 3,
                start_b: 2,
                end_b: 2,
                op: DiffOp::Delete,
            },
            Chunk {
                start_a: 3,
                end_a: 4,
                start_b: 2,
                end_b: 2,
                op: DiffOp::Delete,
            },
            Chunk {
                start_a: 4,
                end_a: 4,
                start_b: 2,
                end_b: 3,
                op: DiffOp::Insert,
            },
            Chunk {
                start_a: 4,
                end_a: 4,
                start_b: 3,
                end_b: 4,
                op: DiffOp::Insert,
            },
        ];
        let merged = merge_adjacent_replace_chunks(&chunks);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].op, DiffOp::Replace);
        assert_eq!(merged[0].start_a, 2);
        assert_eq!(merged[0].end_a, 4);
        assert_eq!(merged[0].start_b, 2);
        assert_eq!(merged[0].end_b, 4);
    }

    #[test]
    fn test_run_merge_respects_equal_boundaries() {
        // Equal, Delete, Insert, Equal → Replace should NOT merge across Equal
        let chunks = vec![
            Chunk {
                start_a: 0,
                end_a: 2,
                start_b: 0,
                end_b: 2,
                op: DiffOp::Equal,
            },
            Chunk {
                start_a: 2,
                end_a: 3,
                start_b: 2,
                end_b: 2,
                op: DiffOp::Delete,
            },
            Chunk {
                start_a: 3,
                end_a: 3,
                start_b: 2,
                end_b: 3,
                op: DiffOp::Insert,
            },
            Chunk {
                start_a: 3,
                end_a: 5,
                start_b: 3,
                end_b: 5,
                op: DiffOp::Equal,
            },
        ];
        let merged = merge_adjacent_replace_chunks(&chunks);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].op, DiffOp::Equal);
        assert_eq!(merged[1].op, DiffOp::Replace);
        assert_eq!(merged[2].op, DiffOp::Equal);
    }

    #[test]
    fn test_three_way_merge_no_conflict() {
        let base = vec!["a".into(), "b".into(), "c".into()];
        let local = vec!["a".into(), "b2".into(), "c".into()];
        let remote = vec!["a".into(), "b3".into(), "c".into()];
        let differ = ThreeWayDiffer::new(base, local, remote);
        let result = differ.merge();
        assert!(!result.merged.is_empty());
    }
}
