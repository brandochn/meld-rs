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

impl LineDiff {
    /// Yield chunks between `from_pane` and `to_pane`, optionally filtered
    /// by a visible line range `(start, end)`.
    ///
    /// Mirrors the original Meld's `Differ.pair_changes()` used by LinkMap
    /// and scroll sync. When `from_pane == 0` and `to_pane == 1`, returns
    /// chunks in left→right orientation; when reversed (1→0), the caller
    /// should swap A/B positions as needed.
    ///
    /// The `visible` range is expressed in `from_pane` line numbers.
    /// Chunks whose range on the from-side overlaps `visible` are included.
    pub fn pair_changes(
        &self,
        from_pane: usize,
        to_pane: usize,
        visible: Option<(usize, usize)>,
    ) -> Vec<(usize, &Chunk)> {
        self.chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                let from_start = if from_pane == 0 { c.start_a } else { c.start_b };
                let from_end = if from_pane == 0 { c.end_a } else { c.end_b };

                // Skip Equal chunks and zero-width chunks with no visual footprint
                if c.op == DiffOp::Equal {
                    return false;
                }
                if from_end <= from_start {
                    return false;
                }

                if let Some((v_start, v_end)) = visible {
                    from_end > v_start && from_start < v_end
                } else {
                    true
                }
            })
            .collect()
    }

    /// Yield changes visible in a single pane, optionally filtered by a
    /// visible line range. Used for per-pane chunk background rendering
    /// and the overview ChunkMap.
    pub fn single_changes(
        &self,
        pane: usize,
        visible: Option<(usize, usize)>,
    ) -> Vec<(usize, &Chunk)> {
        self.pair_changes(pane, if pane == 0 { 1 } else { 0 }, visible)
    }
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
    ///
    /// Chunk construction mirrors Python's `difflib.SequenceMatcher.get_opcodes()`:
    /// consecutive Delete+Insert runs between Equal chunks are grouped into a
    /// single Replace chunk, rather than relying on adjacency detection after
    /// the fact. This handles situations where `similar` produces interleaved
    /// Equal changes within what should be a single Replace gap.
    pub fn compare(&self) -> LineDiff {
        let pre = preprocess_diff(&self.text_a, &self.text_b);

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
        let mut chunks = build_chunks_from_gaps(&diff);

        unprocess_chunks(&mut chunks, &pre);

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
/// (unique to one side) and split the existing (filtered-diff) chunks at
/// these boundaries so that the final chunk list is a clean, non-overlapping
/// partition of the original lines — matching Python Meld's
/// `build_matching_blocks()` partitioning.
///
/// Only scans the middle region `[prefix_len .. len - suffix_len]` because
/// the prefix and suffix are already covered by dedicated Equal chunks.
///
/// After `unprocess_chunks` the filtered-diff chunks span original ranges
/// that may include both kept and non-kept lines.  This function walks
/// through each chunk, detects runs of non-kept positions, and emits
/// Delete / Insert chunks for them, using the chunk's internal A↔B offset
/// to derive correct cross-side indices.
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
    use std::collections::HashSet;
    let kept_a: HashSet<usize> = index_map_a.iter().copied().collect();
    let kept_b: HashSet<usize> = index_map_b.iter().copied().collect();

    // Sort by A position so we walk the original text left-to-right
    chunks.sort_by_key(|c| (c.start_a, c.start_b));

    let mut new_chunks = Vec::with_capacity(chunks.len());

    let mid_start = prefix_len;
    let mid_end_a = text_a.len().saturating_sub(suffix_len);
    let mid_end_b = text_b.len().saturating_sub(suffix_len);

    for chunk in chunks.drain(..) {
        // Pass prefix and suffix chunks through unchanged — they were
        // inserted by `compare()` and represent the stripped common
        // prefix / suffix, not part of the filtered diff.
        if chunk.end_a <= mid_start && chunk.end_b <= mid_start {
            new_chunks.push(chunk);
            continue;
        }
        if chunk.start_a >= mid_end_a && chunk.start_b >= mid_end_b {
            new_chunks.push(chunk);
            continue;
        }

        match chunk.op {
            DiffOp::Equal => {
                split_equal(&mut new_chunks, &chunk, &kept_a, &kept_b);
            }
            DiffOp::Delete => {
                // Delete from the filtered diff — already correct,
                // but may still span non-kept A positions.
                split_delete(&mut new_chunks, &chunk, &kept_a);
            }
            DiffOp::Insert => {
                split_insert(&mut new_chunks, &chunk, &kept_b);
            }
            DiffOp::Replace => {
                split_replace(&mut new_chunks, &chunk, &kept_a, &kept_b);
            }
        }
    }

    *chunks = new_chunks;

    // Now walk through the clean partition and insert any remaining
    // unique-line runs that fall between chunks (i.e. where a gap spans
    // the boundary between two filtered chunks).  Because the filtered
    // diff covers all kept lines, gaps between chunks only exist in the
    // original text when contiguous non-kept runs cross chunk boundaries.

    // Sort by A position again (newly pushed chunks preserve order)
    chunks.sort_by_key(|c| (c.start_a, c.start_b));

    fill_between_gaps(
        chunks,
        &kept_a,
        &kept_b,
        text_a.len(),
        text_b.len(),
        prefix_len,
        suffix_len,
    );
}

// ─── Chunk-splitting helpers ─────────────────────────────────────────

/// Internal offset between A and B positions within a chunk: `sb - sa`.
fn ab_offset(chunk: &Chunk) -> isize {
    chunk.start_b as isize - chunk.start_a as isize
}

/// Split an Equal chunk at A-side and B-side non-kept positions.
fn split_equal(
    out: &mut Vec<Chunk>,
    chunk: &Chunk,
    kept_a: &std::collections::HashSet<usize>,
    kept_b: &std::collections::HashSet<usize>,
) {
    let offset = ab_offset(chunk);

    // Walk A positions within the chunk, emitting Equal blocks for kept
    // positions and Delete blocks for non-kept positions.  The B side
    // position of each delete is `a_pos + offset`.
    let mut a = chunk.start_a;
    while a < chunk.end_a {
        if kept_a.contains(&a) {
            // Start of a kept (Equal) sub-range
            let b = (a as isize + offset) as usize;
            let eq_start_a = a;
            let eq_start_b = b;
            while a < chunk.end_a && kept_a.contains(&a) {
                a += 1;
            }
            let eq_end_a = a;
            let eq_end_b = (eq_end_a as isize + offset) as usize;
            if eq_end_a > eq_start_a && eq_end_b <= chunk.end_b {
                out.push(Chunk {
                    start_a: eq_start_a,
                    end_a: eq_end_a,
                    start_b: eq_start_b,
                    end_b: eq_end_b,
                    op: DiffOp::Equal,
                });
            }
        } else {
            // Non-kept A run → Delete
            let del_start = a;
            while a < chunk.end_a && !kept_a.contains(&a) {
                a += 1;
            }
            let del_b = (del_start as isize + offset) as usize;
            out.push(Chunk {
                start_a: del_start,
                end_a: a,
                start_b: del_b,
                end_b: del_b,
                op: DiffOp::Delete,
            });
        }
    }

    // Now handle B-side non-kept positions that were NOT covered by the
    // A walk (this handles the case where B has unique lines but A does
    // not at the same offset).
    let mut b = chunk.start_b;
    while b < chunk.end_b {
        if !kept_b.contains(&b) {
            let ins_start = b;
            while b < chunk.end_b && !kept_b.contains(&b) {
                b += 1;
            }
            let ins_a = (ins_start as isize - offset) as usize;
            out.push(Chunk {
                start_a: ins_a,
                end_a: ins_a,
                start_b: ins_start,
                end_b: b,
                op: DiffOp::Insert,
            });
        } else {
            b += 1;
        }
    }

    // Re-sort the newly appended portion of `out` by A position.
    // We know the indices of the chunks we just pushed: they are the
    // suffix of `out` whose length is at most the original chunk span.
    let old_len = out.len().saturating_sub(
        (chunk.end_a - chunk.start_a)
            .saturating_add(chunk.end_b - chunk.start_b)
            .saturating_add(4),
    );
    let old_len = old_len.min(out.len());
    out[old_len..].sort_by_key(|c| (c.start_a, c.start_b));
}

/// Split a Delete chunk from the filtered diff.
///
/// Emits the original deletion for kept-A positions (these are the diff's
/// legitimate content) *and* extracts any non-kept A positions that were
/// folded into the chunk's range by `unprocess_chunks` into separate
/// Delete chunks with correct cross-side positions.
fn split_delete(out: &mut Vec<Chunk>, chunk: &Chunk, kept_a: &std::collections::HashSet<usize>) {
    let offset = ab_offset(chunk);
    // Emit the kept-position deletions — these ARE the filtered diff's output
    let mut a = chunk.start_a;
    while a < chunk.end_a {
        if kept_a.contains(&a) {
            let del_start = a;
            while a < chunk.end_a && kept_a.contains(&a) {
                a += 1;
            }
            let del_b = (del_start as isize + offset) as usize;
            out.push(Chunk {
                start_a: del_start,
                end_a: a,
                start_b: del_b,
                end_b: del_b,
                op: DiffOp::Delete,
            });
        } else {
            a += 1;
        }
    }
    // Emit non-kept positions as unique-line Deletes
    a = chunk.start_a;
    while a < chunk.end_a {
        if !kept_a.contains(&a) {
            let del_start = a;
            while a < chunk.end_a && !kept_a.contains(&a) {
                a += 1;
            }
            let del_b = (del_start as isize + offset) as usize;
            out.push(Chunk {
                start_a: del_start,
                end_a: a,
                start_b: del_b,
                end_b: del_b,
                op: DiffOp::Delete,
            });
        } else {
            a += 1;
        }
    }
}

/// Split an Insert chunk from the filtered diff.
///
/// Like `split_delete` but for B-side non-kept positions.
fn split_insert(out: &mut Vec<Chunk>, chunk: &Chunk, kept_b: &std::collections::HashSet<usize>) {
    let offset = ab_offset(chunk);
    // Emit kept-position insertions
    let mut b = chunk.start_b;
    while b < chunk.end_b {
        if kept_b.contains(&b) {
            let ins_start = b;
            while b < chunk.end_b && kept_b.contains(&b) {
                b += 1;
            }
            let ins_a = (ins_start as isize - offset) as usize;
            out.push(Chunk {
                start_a: ins_a,
                end_a: ins_a,
                start_b: ins_start,
                end_b: b,
                op: DiffOp::Insert,
            });
        } else {
            b += 1;
        }
    }
    // Emit non-kept positions as unique-line Inserts
    b = chunk.start_b;
    while b < chunk.end_b {
        if !kept_b.contains(&b) {
            let ins_start = b;
            while b < chunk.end_b && !kept_b.contains(&b) {
                b += 1;
            }
            let ins_a = (ins_start as isize - offset) as usize;
            out.push(Chunk {
                start_a: ins_a,
                end_a: ins_a,
                start_b: ins_start,
                end_b: b,
                op: DiffOp::Insert,
            });
        } else {
            b += 1;
        }
    }
}

/// Split a Replace chunk from the filtered diff.
///
/// Emits kept-position blocks as the original Replace (plus any contained
/// unique-line Deletes/Inserts) after `merge_adjacent_replace_chunks`
/// reassembles them downstream.
fn split_replace(
    out: &mut Vec<Chunk>,
    chunk: &Chunk,
    kept_a: &std::collections::HashSet<usize>,
    kept_b: &std::collections::HashSet<usize>,
) {
    let offset = ab_offset(chunk);

    // Kept A-positions → Delete (will merge into Replace downstream)
    let mut a = chunk.start_a;
    while a < chunk.end_a {
        if kept_a.contains(&a) {
            let del_start = a;
            while a < chunk.end_a && kept_a.contains(&a) {
                a += 1;
            }
            let del_b = (del_start as isize + offset) as usize;
            out.push(Chunk {
                start_a: del_start,
                end_a: a,
                start_b: del_b,
                end_b: del_b,
                op: DiffOp::Delete,
            });
        } else {
            a += 1;
        }
    }

    // Non-kept A-positions → extra unique-line Deletes
    a = chunk.start_a;
    while a < chunk.end_a {
        if !kept_a.contains(&a) {
            let del_start = a;
            while a < chunk.end_a && !kept_a.contains(&a) {
                a += 1;
            }
            let del_b = (del_start as isize + offset) as usize;
            out.push(Chunk {
                start_a: del_start,
                end_a: a,
                start_b: del_b,
                end_b: del_b,
                op: DiffOp::Delete,
            });
        } else {
            a += 1;
        }
    }

    // Kept B-positions → Insert (will merge into Replace downstream)
    let mut b = chunk.start_b;
    while b < chunk.end_b {
        if kept_b.contains(&b) {
            let ins_start = b;
            while b < chunk.end_b && kept_b.contains(&b) {
                b += 1;
            }
            let ins_a = (ins_start as isize - offset) as usize;
            out.push(Chunk {
                start_a: ins_a,
                end_a: ins_a,
                start_b: ins_start,
                end_b: b,
                op: DiffOp::Insert,
            });
        } else {
            b += 1;
        }
    }

    // Non-kept B-positions → extra unique-line Inserts
    b = chunk.start_b;
    while b < chunk.end_b {
        if !kept_b.contains(&b) {
            let ins_start = b;
            while b < chunk.end_b && !kept_b.contains(&b) {
                b += 1;
            }
            let ins_a = (ins_start as isize - offset) as usize;
            out.push(Chunk {
                start_a: ins_a,
                end_a: ins_a,
                start_b: ins_start,
                end_b: b,
                op: DiffOp::Insert,
            });
        } else {
            b += 1;
        }
    }
}

/// After splitting all chunks at internal gaps, scan the resulting chunk
/// list for gaps that remain between consecutive chunks and insert
/// Delete / Insert for any remaining non-kept lines that cross chunk
/// boundaries.
fn fill_between_gaps(
    chunks: &mut Vec<Chunk>,
    kept_a: &std::collections::HashSet<usize>,
    kept_b: &std::collections::HashSet<usize>,
    text_a_len: usize,
    text_b_len: usize,
    prefix_len: usize,
    suffix_len: usize,
) {
    let a_end = text_a_len - suffix_len;
    let b_end = text_b_len - suffix_len;

    // Insert a sentinel chunk at the end to simplify the loop
    let n = chunks.len();

    let mut extra = Vec::new();

    for idx in 0..=n {
        let prev_end_a = if idx == 0 {
            prefix_len
        } else {
            chunks[idx - 1].end_a
        };
        let prev_end_b = if idx == 0 {
            prefix_len
        } else {
            chunks[idx - 1].end_b
        };
        let next_start_a = if idx < n { chunks[idx].start_a } else { a_end };
        let next_start_b = if idx < n { chunks[idx].start_b } else { b_end };

        // A-side gap
        let mut a_pos = prev_end_a;
        while a_pos < next_start_a && a_pos < a_end {
            if !kept_a.contains(&a_pos) {
                let start = a_pos;
                while a_pos < next_start_a && a_pos < a_end && !kept_a.contains(&a_pos) {
                    a_pos += 1;
                }
                extra.push(Chunk {
                    start_a: start,
                    end_a: a_pos,
                    start_b: prev_end_b,
                    end_b: prev_end_b,
                    op: DiffOp::Delete,
                });
            } else {
                a_pos += 1;
            }
        }

        // B-side gap
        let mut b_pos = prev_end_b;
        while b_pos < next_start_b && b_pos < b_end {
            if !kept_b.contains(&b_pos) {
                let start = b_pos;
                while b_pos < next_start_b && b_pos < b_end && !kept_b.contains(&b_pos) {
                    b_pos += 1;
                }
                extra.push(Chunk {
                    start_a: prev_end_a,
                    end_a: prev_end_a,
                    start_b: start,
                    end_b: b_pos,
                    op: DiffOp::Insert,
                });
            } else {
                b_pos += 1;
            }
        }
    }

    chunks.extend(extra);
    chunks.sort_by_key(|c| (c.start_a, c.start_b));
}

// ─── Gap-based chunk construction ────────────────────────────────────

/// A lightweight snapshot of a single change from the `similar` crate,
/// used to build chunk groups without lifetime constraints.
#[derive(Debug, Clone)]
struct RawChange {
    tag: similar::ChangeTag,
    old_index: Option<usize>,
    new_index: Option<usize>,
    line_count: usize,
}

/// Build diff chunks from `similar`'s change stream using the same gap-based
/// logic as Python's `difflib.SequenceMatcher.get_opcodes()`.
///
/// Consecutive Delete + Insert runs that sit in the same gap between Equal
/// regions are merged into a single [`DiffOp::Replace`] chunk. Standalone
/// Deletes or Inserts (i.e., only one side changed in a gap) remain as their
/// respective operation types.
///
/// This avoids the adjacency-detection problem where `similar` may produce
/// interleaved Equal changes that separate Delete/Insert pairs that belong
/// to the same logical Replace.
fn build_chunks_from_gaps<'a>(diff: &TextDiff<'a, 'a, 'a, str>) -> Vec<Chunk> {
    let raw: Vec<RawChange> = diff
        .iter_all_changes()
        .map(|c| RawChange {
            tag: c.tag(),
            old_index: c.old_index(),
            new_index: c.new_index(),
            line_count: c.value().lines().count(),
        })
        .collect();

    let mut chunks = Vec::new();
    let mut i = 0;
    let n = raw.len();

    while i < n {
        match raw[i].tag {
            similar::ChangeTag::Equal => {
                let ri = &raw[i];
                let oi = ri.old_index.unwrap();
                let ni = ri.new_index.unwrap();
                chunks.push(Chunk {
                    start_a: oi,
                    end_a: oi + ri.line_count,
                    start_b: ni,
                    end_b: ni + ri.line_count,
                    op: DiffOp::Equal,
                });
                i += 1;
            }
            _ => {
                let gap_start = i;
                while i < n && raw[i].tag != similar::ChangeTag::Equal {
                    i += 1;
                }
                let gap = &raw[gap_start..i];

                let has_delete = gap.iter().any(|r| r.tag == similar::ChangeTag::Delete);
                let has_insert = gap.iter().any(|r| r.tag == similar::ChangeTag::Insert);

                if has_delete && has_insert {
                    let start_a = gap
                        .iter()
                        .filter(|r| r.tag == similar::ChangeTag::Delete)
                        .map(|r| r.old_index.unwrap())
                        .min()
                        .unwrap();
                    let end_a = gap
                        .iter()
                        .filter(|r| r.tag == similar::ChangeTag::Delete)
                        .map(|r| r.old_index.unwrap() + r.line_count)
                        .max()
                        .unwrap();
                    let start_b = gap
                        .iter()
                        .filter(|r| r.tag == similar::ChangeTag::Insert)
                        .map(|r| r.new_index.unwrap())
                        .min()
                        .unwrap();
                    let end_b = gap
                        .iter()
                        .filter(|r| r.tag == similar::ChangeTag::Insert)
                        .map(|r| r.new_index.unwrap() + r.line_count)
                        .max()
                        .unwrap();
                    chunks.push(Chunk {
                        start_a,
                        end_a,
                        start_b,
                        end_b,
                        op: DiffOp::Replace,
                    });
                } else if has_delete {
                    for r in gap.iter().filter(|r| r.tag == similar::ChangeTag::Delete) {
                        let idx = r.old_index.unwrap();
                        chunks.push(Chunk {
                            start_a: idx,
                            end_a: idx + r.line_count,
                            start_b: idx,
                            end_b: idx,
                            op: DiffOp::Delete,
                        });
                    }
                } else {
                    for r in gap.iter().filter(|r| r.tag == similar::ChangeTag::Insert) {
                        let idx = r.new_index.unwrap();
                        chunks.push(Chunk {
                            start_a: idx,
                            end_a: idx,
                            start_b: idx,
                            end_b: idx + r.line_count,
                            op: DiffOp::Insert,
                        });
                    }
                }
            }
        }
    }

    chunks
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

/// Adjust chunk boundaries and tags when `ignore_blank_lines` is active.
///
/// Mirrors the original Meld's `consume_blank_lines()` in `diffutil.py`.
/// Trims leading and trailing blank lines from each chunk. If a `Replace`
/// chunk's A or B side becomes empty after trimming, its tag is demoted
/// to `Delete` or `Insert` respectively, and empty chunks are removed.
pub fn consume_blank_lines(chunks: &mut Vec<Chunk>, text_a: &[String], text_b: &[String]) {
    chunks.retain_mut(|chunk| {
        let mut a_start = chunk.start_a;
        let mut a_end = chunk.end_a;
        let mut b_start = chunk.start_b;
        let mut b_end = chunk.end_b;

        // Trim leading blank lines from A side
        while a_start < a_end && a_start < text_a.len() && text_a[a_start].trim().is_empty() {
            a_start += 1;
        }
        // Trim trailing blank lines from A side
        while a_end > a_start && a_end <= text_a.len() && text_a[a_end - 1].trim().is_empty() {
            a_end -= 1;
        }
        // Trim leading blank lines from B side
        while b_start < b_end && b_start < text_b.len() && text_b[b_start].trim().is_empty() {
            b_start += 1;
        }
        // Trim trailing blank lines from B side
        while b_end > b_start && b_end <= text_b.len() && text_b[b_end - 1].trim().is_empty() {
            b_end -= 1;
        }

        let a_has_content = a_end > a_start;
        let b_has_content = b_end > b_start;

        // Adjust tag based on what remains
        if chunk.op == DiffOp::Replace {
            if a_has_content && b_has_content {
                // Still a replace — update bounds
                chunk.start_a = a_start;
                chunk.end_a = a_end;
                chunk.start_b = b_start;
                chunk.end_b = b_end;
                true
            } else if a_has_content {
                // Only A remains → demote to Delete
                chunk.op = DiffOp::Delete;
                chunk.start_a = a_start;
                chunk.end_a = a_end;
                chunk.start_b = b_start; // zero-width
                chunk.end_b = b_start;
                a_end > a_start
            } else if b_has_content {
                // Only B remains → demote to Insert
                chunk.op = DiffOp::Insert;
                chunk.start_a = a_start; // zero-width
                chunk.end_a = a_start;
                chunk.start_b = b_start;
                chunk.end_b = b_end;
                b_end > b_start
            } else {
                // Both empty → remove
                false
            }
        } else if a_has_content || b_has_content {
            chunk.start_a = a_start;
            chunk.end_a = a_end;
            chunk.start_b = b_start;
            chunk.end_b = b_end;
            true
        } else {
            false
        }
    });
}

// ─── Tokenization helper ───────────────────────────────────────────

/// Split a line into tokens at whitespace, punctuation, and
/// CamelCase/snake_case boundaries. Returns (tokens, start_offsets).
///
/// Each token is a contiguous run of alphanumeric characters or an
/// individual punctuation/whitespace character. CamelCase identifiers
/// like `FooBar` are split into `["Foo", "Bar"]`.
fn tokenize_with_offsets(line: &str) -> (Vec<String>, Vec<usize>) {
    let mut tokens = Vec::new();
    let mut offsets = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];
        let start = i;

        if ch.is_alphanumeric() || ch == '_' {
            // Start of an identifier/word — split on camelCase and
            // snake_case boundaries.
            let mut word_start = i;
            let mut prev_kind = char_kind(ch);

            i += 1;
            while i < len {
                let c = chars[i];
                let kind = char_kind(c);

                if c == '_' {
                    // snake_case boundary
                    tokens.push(line[word_start..i].to_string());
                    offsets.push(word_start);
                    // Emit underscore as its own token
                    tokens.push("_".to_string());
                    offsets.push(i);
                    i += 1;
                    word_start = i;
                    // Find next alphanumeric start
                    while i < len && !chars[i].is_alphanumeric() && chars[i] != '_' {
                        tokens.push(chars[i].to_string());
                        offsets.push(i);
                        i += 1;
                    }
                    word_start = i;
                    if i < len {
                        prev_kind = char_kind(chars[i]);
                        i += 1;
                    }
                } else if kind != prev_kind
                    && prev_kind == CharKind::Lower
                    && kind == CharKind::Upper
                {
                    // camelCase boundary: "fooBar" -> "foo", "Bar"
                    tokens.push(line[word_start..i].to_string());
                    offsets.push(word_start);
                    word_start = i;
                    prev_kind = kind;
                    i += 1;
                } else if !c.is_alphanumeric() {
                    // End of word at punctuation/whitespace
                    tokens.push(line[word_start..i].to_string());
                    offsets.push(word_start);
                    word_start = i;
                    break;
                } else {
                    prev_kind = kind;
                    i += 1;
                }
            }

            // Emit the remaining word
            if word_start < i.min(len) {
                tokens.push(line[word_start..i.min(len)].to_string());
                offsets.push(word_start);
            }
        } else {
            // Punctuation/whitespace — each character is its own token
            tokens.push(ch.to_string());
            offsets.push(start);
            i += 1;
        }
    }

    (tokens, offsets)
}

/// Classify a character for camelCase boundary detection.
#[derive(Debug, PartialEq, Eq)]
enum CharKind {
    Lower,
    Upper,
    Digit,
    Other,
}

fn char_kind(ch: char) -> CharKind {
    if ch.is_lowercase() {
        CharKind::Lower
    } else if ch.is_uppercase() {
        CharKind::Upper
    } else if ch.is_ascii_digit() {
        CharKind::Digit
    } else {
        CharKind::Other
    }
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
    ///
    /// Mirrors Python Meld's character-level diff pipeline:
    /// 1. Compute char-level diff via `similar::TextDiff::from_chars`
    /// 2. Filter out tiny equal segments (< 3 chars, except at boundaries)
    ///    to reduce visual noise
    /// 3. Merge consecutive Delete+Insert runs into Replace chunks
    pub fn compare_line(line_a: &str, line_b: &str) -> Vec<InlineChange> {
        if line_a == line_b {
            return Vec::new();
        }

        let diff = TextDiff::from_chars(line_a, line_b);

        // Collect all changes so we can filter small equals
        struct RawCharChange {
            tag: ChangeTag,
            old_index: Option<usize>,
            new_index: Option<usize>,
            len: usize,
        }
        let raw: Vec<RawCharChange> = diff
            .iter_all_changes()
            .map(|c| RawCharChange {
                tag: c.tag(),
                old_index: c.old_index(),
                new_index: c.new_index(),
                len: c.value().chars().count(),
            })
            .collect();

        // Find the full span of non-equal changes for boundary detection
        let first_change = raw.iter().position(|r| r.tag != ChangeTag::Equal);
        let last_change = raw.iter().rposition(|r| r.tag != ChangeTag::Equal);

        // Build the filtered change list, removing tiny equal segments
        let mut changes = Vec::new();
        for (i, r) in raw.iter().enumerate() {
            match r.tag {
                ChangeTag::Equal => {
                    // Skip equal segments smaller than 3 chars that are not
                    // at the very start or end of the changed span (mirroring
                    // Python Meld's process_matches() filter).
                    let at_start = first_change.map_or(false, |fc| i < fc);
                    let at_end = last_change.map_or(false, |lc| i > lc);
                    if r.len < 3 && !at_start && !at_end {
                        continue;
                    }
                    // Larger equal segments are simply omitted from inline
                    // highlighting (we only emit changed characters).
                }
                ChangeTag::Delete => {
                    if let Some(idx) = r.old_index {
                        changes.push(InlineChange {
                            start: idx,
                            end: idx + r.len,
                            op: DiffOp::Delete,
                        });
                    }
                }
                ChangeTag::Insert => {
                    if let Some(idx) = r.new_index {
                        changes.push(InlineChange {
                            start: idx,
                            end: idx + r.len,
                            op: DiffOp::Insert,
                        });
                    }
                }
            }
        }

        // Merge consecutive Delete+Insert runs into Replace chunks.
        // Handles patterns like Delete, Delete, Insert, Insert → one Replace.
        Self::_postprocess_multi(&mut changes);
        changes
    }

    /// Merge all consecutive runs of Delete and Insert (in any order)
    /// into a single Replace chunk, matching the line-level gap-based
    /// logic used by `merge_adjacent_replace_chunks`.
    fn _postprocess_multi(changes: &mut Vec<InlineChange>) {
        let mut i = 0;
        while i < changes.len() {
            let mut j = i;
            let mut has_delete = false;
            let mut has_insert = false;
            while j < changes.len() {
                match changes[j].op {
                    DiffOp::Delete => {
                        has_delete = true;
                    }
                    DiffOp::Insert => {
                        has_insert = true;
                    }
                    _ => break,
                }
                j += 1;
            }
            if has_delete && has_insert {
                // Merge [i..j) into a single Replace spanning the full range
                let start = changes[i].start;
                let end = changes[j - 1].end;
                changes.drain(i..j);
                changes.insert(
                    i,
                    InlineChange {
                        start,
                        end,
                        op: DiffOp::Replace,
                    },
                );
            }
            i += 1;
        }
    }

    /// Compare two lines at the token (word) level.
    ///
    /// Tokenizes each line into words split by whitespace, punctuation, and
    /// CamelCase/snake_case boundaries. Then computes a diff over the token
    /// sequences and maps the result back to character offsets in the
    /// original lines.
    ///
    /// This produces more meaningful diffs for code than raw character-level
    /// comparison, especially for identifier renames and import changes.
    pub fn compare_line_tokens(line_a: &str, line_b: &str) -> Vec<InlineChange> {
        if line_a == line_b {
            return Vec::new();
        }

        let (tokens_a, offsets_a) = tokenize_with_offsets(line_a);
        let (tokens_b, offsets_b) = tokenize_with_offsets(line_b);

        let token_strs_a: Vec<&str> = tokens_a.iter().map(|t| t.as_str()).collect();
        let token_strs_b: Vec<&str> = tokens_b.iter().map(|t| t.as_str()).collect();

        let joined_a = token_strs_a.join("\n");
        let joined_b = token_strs_b.join("\n");

        let diff = TextDiff::from_lines(&joined_a, &joined_b);
        let mut changes = Vec::new();

        for change in diff.iter_all_changes() {
            match change.tag() {
                ChangeTag::Equal => {}
                ChangeTag::Delete => {
                    if let Some(idx) = change.old_index() {
                        let end_idx = idx + change.value().lines().count();
                        let start = offsets_a[idx];
                        let end = if end_idx < offsets_a.len() {
                            offsets_a[end_idx]
                        } else {
                            line_a.len()
                        };
                        if start < end {
                            changes.push(InlineChange {
                                start,
                                end,
                                op: DiffOp::Delete,
                            });
                        }
                    }
                }
                ChangeTag::Insert => {
                    if let Some(idx) = change.new_index() {
                        let end_idx = idx + change.value().lines().count();
                        let start = offsets_b[idx];
                        let end = if end_idx < offsets_b.len() {
                            offsets_b[end_idx]
                        } else {
                            line_b.len()
                        };
                        if start < end {
                            changes.push(InlineChange {
                                start,
                                end,
                                op: DiffOp::Insert,
                            });
                        }
                    }
                }
            }
        }

        // Token-level diffs treat each identifier/punctuation group as an
        // atomic unit.  Unlike character-level diffs, we do NOT call
        // _postprocess_multi here: merging consecutive Delete+Insert runs
        // would collapse distinct token changes (e.g., deleting
        // "EnvironmentContext" and inserting "notifySuccess" at a different
        // position) into a single over-broad Replace, losing the per-token
        // granularity that Meld users expect.
        changes
    }

    pub fn compare_import_lines(line_a: &str, line_b: &str) -> Vec<InlineChange> {
        let ids_a = Self::extract_import_specifiers(line_a);
        let ids_b = Self::extract_import_specifiers(line_b);
        if ids_a.is_empty() || ids_b.is_empty() {
            return Vec::new();
        }
        use std::collections::HashSet;
        let set_a: HashSet<&str> = ids_a.iter().map(|(id, _)| id.as_str()).collect();
        let set_b: HashSet<&str> = ids_b.iter().map(|(id, _)| id.as_str()).collect();
        let mut changes = Vec::new();
        for (id, (start, end)) in &ids_a {
            if !set_b.contains(id.as_str()) {
                changes.push(InlineChange {
                    start: *start,
                    end: *end,
                    op: DiffOp::Delete,
                });
            }
        }
        for (id, (start, end)) in &ids_b {
            if !set_a.contains(id.as_str()) {
                changes.push(InlineChange {
                    start: *start,
                    end: *end,
                    op: DiffOp::Insert,
                });
            }
        }
        changes
    }

    /// `other_sets` maps module -> all identifiers on the other buffer.
    /// `missing_op` specifies the op for identifiers missing from the other
    /// side: `DiffOp::Delete` for the left (old) pane, `DiffOp::Insert` for
    /// the right (new) pane.
    pub fn compare_import_lines_grouped(
        line_this: &str,
        _line_other: &str,
        other_sets: &std::collections::HashMap<String, std::collections::HashSet<String>>,
        missing_op: DiffOp,
    ) -> Vec<InlineChange> {
        let (module, ids_this) = match Self::parse_import_line(line_this) {
            Some(p) => p,
            None => return Vec::new(),
        };
        let empty_set = std::collections::HashSet::new();
        let other_ids = other_sets.get(&module).unwrap_or(&empty_set);
        let mut changes = Vec::new();
        for (id, (start, end)) in &ids_this {
            if !other_ids.contains(id) {
                changes.push(InlineChange {
                    start: *start,
                    end: *end,
                    op: missing_op,
                });
            }
        }
        changes
    }
    fn extract_import_specifiers(line: &str) -> Vec<(String, (usize, usize))> {
        match Self::parse_import_line(line) {
            Some((_module, ids)) => ids,
            None => Vec::new(),
        }
    }

    pub fn parse_import_line(line: &str) -> Option<(String, Vec<(String, (usize, usize))>)> {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("import") {
            return None;
        }
        let brace_start = match line.find('{') {
            Some(p) => p,
            None => return None,
        };
        let brace_end = match line.rfind('}') {
            Some(p) => p,
            None => return None,
        };
        if brace_end <= brace_start {
            return None;
        }
        let after_brace = &line[brace_end + 1..];
        if !after_brace.contains("from") {
            return None;
        }
        let module = match Self::extract_module_string(after_brace) {
            Some(m) => m,
            None => return None,
        };
        let inner = &line[brace_start + 1..brace_end];
        let mut results = Vec::new();
        let mut search_pos = brace_start + 1;
        for part in inner.split(',') {
            let trimmed_id = part.trim();
            if !trimmed_id.is_empty() && Self::is_identifier(trimmed_id) {
                if let Some(rel_pos) = line[search_pos..].find(trimmed_id) {
                    let start = search_pos + rel_pos;
                    let end = start + trimmed_id.len();
                    let before_ok = start == 0
                        || !line.as_bytes()[start - 1].is_ascii_alphanumeric()
                            && line.as_bytes()[start - 1] != b'_';
                    let after_ok = end >= line.len()
                        || !line.as_bytes()[end].is_ascii_alphanumeric()
                            && line.as_bytes()[end] != b'_';
                    if before_ok && after_ok {
                        results.push((trimmed_id.to_string(), (start, end)));
                    }
                    search_pos = end;
                }
            }
        }
        Some((module, results))
    }

    fn extract_module_string(after_brace: &str) -> Option<String> {
        let from_pos = after_brace.find("from")?;
        let after_from = &after_brace[from_pos + 4..];
        let trimmed = after_from.trim_start();
        let quote = trimmed.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let inner = &trimmed[1..];
        let close_pos = inner.find(quote)?;
        Some(inner[..close_pos].to_string())
    }

    fn is_identifier(s: &str) -> bool {
        let mut chars = s.chars();
        match chars.next() {
            Some(c) if c.is_alphabetic() || c == '_' || c == '$' => {}
            _ => return false,
        }
        chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    }
}

// ─── Line cache (O(1) chunk navigation) ────────────────────────────

/// Caches a mapping from line numbers to chunk indices for fast navigation.
///
/// Stores `(prev, curr, next)` chunk indices per line so that navigation
/// (next/previous change) and hover sync can operate in O(1) time.
/// Only non-Equal chunks are tracked; Equal chunks return `None` for all three.
#[derive(Debug, Clone)]
pub struct LineCache {
    /// The chunk index containing each line, if the chunk is non-Equal.
    entries: Vec<Option<usize>>,
    /// The previous non-Equal chunk index for each line (for prev-change nav).
    prevs: Vec<Option<usize>>,
    /// The next non-Equal chunk index for each line (for next-change nav).
    nexts: Vec<Option<usize>>,
}

impl LineCache {
    /// Build a line cache from diff chunks.
    ///
    /// Computes `(prev, curr, next)` for every line in O(chunks * lines)
    /// by scanning forward for `prev`/`curr` and backward for `next`.
    pub fn new(chunks: &[Chunk], max_lines: usize) -> Self {
        let mut entries = vec![None; max_lines];
        let mut prevs = vec![None; max_lines];
        let mut nexts = vec![None; max_lines];

        // Forward pass: fill `entries` and `prevs`
        let mut prev_non_equal: Option<usize> = None;
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
                prevs[line] = prev_non_equal;
            }
            prev_non_equal = Some(ci);
        }

        // Backward pass: fill `nexts`
        let mut next_non_equal: Option<usize> = None;
        for (ci, chunk) in chunks.iter().enumerate().rev() {
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
                nexts[line] = next_non_equal;
            }
            next_non_equal = Some(ci);
        }

        Self {
            entries,
            prevs,
            nexts,
        }
    }

    /// Return the chunk index for a given line, if any.
    pub fn locate_chunk(&self, line: usize) -> Option<usize> {
        self.entries.get(line).copied().flatten()
    }

    /// Return the chunk indices surrounding a line `(prev, curr, next)`.
    ///
    /// Only non-Equal chunks are returned — Equal chunks are skipped
    /// to match navigation expectations (jumping between actual changes).
    /// All three may be `None` if the line is in an Equal region.
    pub fn chunk_triad(&self, line: usize) -> (Option<usize>, Option<usize>, Option<usize>) {
        let curr = self.locate_chunk(line);
        let prev = self.prevs.get(line).copied().flatten();
        let next = self.nexts.get(line).copied().flatten();
        (prev, curr, next)
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
        // Not worth it — return the full middle section (prefix/suffix
        // already stripped so the diff only processes middle lines).
        // Prefix and suffix themselves are re-inserted by `compare()`.
        // The index maps must map filtered (middle-section) positions to
        // original (full-text) positions.
        let full_a = text_a[prefix_len..len_a - suffix_len].to_vec();
        let full_b = text_b[prefix_len..len_b - suffix_len].to_vec();
        let map_a: Vec<usize> = (0..full_a.len()).map(|i| prefix_len + i).collect();
        let map_b: Vec<usize> = (0..full_b.len()).map(|i| prefix_len + i).collect();
        return PreprocessResult {
            filtered_a: full_a,
            filtered_b: full_b,
            index_map_a: map_a,
            index_map_b: map_b,
            prefix_len,
            suffix_len,
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
///
/// For chunks that span multiple filtered lines (non-zero-width), the end
/// position is computed from the last mapped index + 1.
///
/// For zero-width chunks (e.g. Insert where start_a == end_a, or Delete
/// where start_b == end_b), the end position must equal the start position
/// to maintain the zero-width invariant.  The first-branch fallthrough bug
/// would otherwise produce invalid ranges like `[599..598)` when gaps exist
/// in the index maps (caused by the unique-line discard optimisation).
pub fn unprocess_chunks(chunks: &mut Vec<Chunk>, pre: &PreprocessResult) {
    for chunk in chunks.iter_mut() {
        // Snapshot original (filtered-space) indices before any remapping
        let f_start_a = chunk.start_a;
        let f_end_a = chunk.end_a;
        let f_start_b = chunk.start_b;
        let f_end_b = chunk.end_b;

        // ── A side ──
        if f_start_a < pre.index_map_a.len() {
            chunk.start_a = pre.index_map_a[f_start_a];
        }
        if f_start_a == f_end_a {
            // Zero-width chunk: end maps to the same original position as start
            chunk.end_a = chunk.start_a;
        } else if f_end_a > 0 && f_end_a - 1 < pre.index_map_a.len() {
            chunk.end_a = pre.index_map_a[f_end_a - 1] + 1;
        }

        // ── B side ──
        if f_start_b < pre.index_map_b.len() {
            chunk.start_b = pre.index_map_b[f_start_b];
        }
        if f_start_b == f_end_b {
            // Zero-width chunk: end maps to the same original position as start
            chunk.end_b = chunk.start_b;
        } else if f_end_b > 0 && f_end_b - 1 < pre.index_map_b.len() {
            chunk.end_b = pre.index_map_b[f_end_b - 1] + 1;
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
