//! Diff engine based on the `similar` crate.
//!
//! Provides line-level diffing via [`Differ`] and 3-way merge logic via
//! [`ThreeWayDiffer`]. Includes preprocessing, inline diff with post-processing,
//! and O(1) line-to-chunk mapping via [`LineCache`].
//!
//! Replaces the Python `difflib`/matchers module from the original Meld.

use similar::{ChangeTag, TextDiff};
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// chunks in leftÃ¢â€ â€™right orientation; when reversed (1Ã¢â€ â€™0), the caller
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
    sync_points: Vec<(usize, usize)>,
}

impl Differ {
    /// Creates a new [`Differ`] with the two texts to compare.
    pub fn new(text_a: Vec<String>, text_b: Vec<String>) -> Self {
        Self {
            text_a,
            text_b,
            sync_points: Vec::new(),
        }
    }

    /// Attach forced-alignment sync points `(line_in_a, line_in_b)`.
    ///
    /// Each pair marks a position where the diff algorithm **must** split
    /// the input.  Points are filtered (out-of-range entries dropped),
    /// sorted, and deduplicated automatically.
    pub fn with_sync_points(mut self, points: Vec<(usize, usize)>) -> Self {
        let len_a = self.text_a.len();
        let len_b = self.text_b.len();
        let mut pts: Vec<(usize, usize)> = points
            .into_iter()
            .filter(|&(a, b)| a <= len_a && b <= len_b)
            .collect();
        pts.sort_unstable();
        pts.dedup();
        self.sync_points = pts;
        self
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
        if !self.sync_points.is_empty() {
            return self.compare_segmented();
        }
        self.compare_single(&self.text_a, &self.text_b)
    }

    pub fn compare_with_cancel(&self, cancel: &AtomicBool) -> Option<LineDiff> {
        if cancel.load(Ordering::SeqCst) {
            return None;
        }
        // Cancellation between segments would be ideal, but for now
        // delegate to the full compare.  Sync-point segments are
        // typically small.
        Some(self.compare())
    }

    /// Core diff routine for a single contiguous segment of text.
    ///
    /// All output chunk coordinates are relative to the segment start
    /// (i.e. zero-based within the segment).
    fn compare_single(&self, text_a: &[String], text_b: &[String]) -> LineDiff {
        let pre = preprocess_diff(text_a, text_b);

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
            chunks.push(Chunk {
                start_a: text_a.len() - pre.suffix_len,
                end_a: text_a.len(),
                start_b: text_b.len() - pre.suffix_len,
                end_b: text_b.len(),
                op: DiffOp::Equal,
            });
        }

        insert_unique_line_chunks(
            &mut chunks,
            text_a,
            text_b,
            &pre.index_map_a,
            &pre.index_map_b,
            pre.prefix_len,
            pre.suffix_len,
        );

        LineDiff {
            chunks,
            line_a: text_a.to_vec(),
            line_b: text_b.to_vec(),
        }
    }

    /// Diff the full text split at each sync-point pair, merging
    /// per-segment results into a single sorted chunk list.
    ///
    /// Mirrors Python Meld's `SyncPointMyersSequenceMatcher`: each
    /// segment between consecutive sync points is diffed independently.
    fn compare_segmented(&self) -> LineDiff {
        let mut all_chunks: Vec<Chunk> = Vec::new();
        let mut seg_base_a = 0usize;
        let mut seg_base_b = 0usize;

        for &(sa, sb) in &self.sync_points {
            if sa > seg_base_a || sb > seg_base_b {
                let seg_a = &self.text_a[seg_base_a..sa];
                let seg_b = &self.text_b[seg_base_b..sb];
                let mut seg = self.compare_single(seg_a, seg_b);
                for chunk in &mut seg.chunks {
                    chunk.start_a += seg_base_a;
                    chunk.end_a += seg_base_a;
                    chunk.start_b += seg_base_b;
                    chunk.end_b += seg_base_b;
                }
                all_chunks.extend(seg.chunks);
            }
            seg_base_a = sa;
            seg_base_b = sb;
        }

        if seg_base_a < self.text_a.len() || seg_base_b < self.text_b.len() {
            let seg_a = &self.text_a[seg_base_a..];
            let seg_b = &self.text_b[seg_base_b..];
            let mut seg = self.compare_single(seg_a, seg_b);
            for chunk in &mut seg.chunks {
                chunk.start_a += seg_base_a;
                chunk.end_a += seg_base_a;
                chunk.start_b += seg_base_b;
                chunk.end_b += seg_base_b;
            }
            all_chunks.extend(seg.chunks);
        }

        let mut all_chunks = merge_adjacent_replace_chunks(&all_chunks);
        all_chunks.sort_by_key(|c| (c.start_a, c.start_b));

        LineDiff {
            chunks: all_chunks,
            line_a: self.text_a.clone(),
            line_b: self.text_b.clone(),
        }
    }
}

/// that may include both kept and non-kept lines.  This function walks
/// through each chunk, detects runs of non-kept positions, and emits
/// Delete / Insert chunks for them, using the chunk's internal AÃ¢â€ â€B offset
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
        // Pass prefix and suffix chunks through unchanged Ã¢â‚¬â€ they were
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
                // Delete from the filtered diff Ã¢â‚¬â€ already correct,
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

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Chunk-splitting helpers Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
            // Non-kept A run Ã¢â€ â€™ Delete
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
    // Emit the kept-position deletions Ã¢â‚¬â€ these ARE the filtered diff's output
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

    // Kept A-positions Ã¢â€ â€™ Delete (will merge into Replace downstream)
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

    // Non-kept A-positions Ã¢â€ â€™ extra unique-line Deletes
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

    // Kept B-positions Ã¢â€ â€™ Insert (will merge into Replace downstream)
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

    // Non-kept B-positions Ã¢â€ â€™ extra unique-line Inserts
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

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Gap-based chunk construction Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
/// Tracks the current A/B context position (`cur_a`, `cur_b`) between gaps
/// so that standalone Inserts and Deletes receive the correct cross-side
/// position. Without this, a standalone Insert would incorrectly use
/// `new_index` for its A-side position, and a standalone Delete would
/// incorrectly use `old_index` for its B-side position Ã¢â‚¬â€ producing chunks
/// whose positions overlap or contradict adjacent Equal blocks.
///
/// This mirrors difflib's opcode semantics where:
///   - Insert `(i1, i1, j1, j2)`: `i1` = current old context position
///   - Delete `(i1, i2, j1, j1)`: `j1` = current new context position
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

    // Track the current (end-of-last-Equal) position in both sequences.
    // These represent the insertion/deletion context: where a standalone
    // Insert or Delete sits in the *other* side.
    let mut cur_a = 0usize;
    let mut cur_b = 0usize;

    while i < n {
        match raw[i].tag {
            similar::ChangeTag::Equal => {
                let ri = &raw[i];
                let oi = ri.old_index.unwrap();
                let ni = ri.new_index.unwrap();
                let len = ri.line_count;
                chunks.push(Chunk {
                    start_a: oi,
                    end_a: oi + len,
                    start_b: ni,
                    end_b: ni + len,
                    op: DiffOp::Equal,
                });
                cur_a = oi + len;
                cur_b = ni + len;
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
                    // Mixed gap: Delete + Insert Ã¢â€ â€™ single Replace chunk.
                    // A-side positions come from the Delete entries (old_index).
                    // B-side positions come from the Insert entries (new_index).
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
                    cur_a = end_a;
                    cur_b = end_b;
                } else if has_delete {
                    // Standalone Delete gap: lines removed from A.
                    // A-side positions come from old_index (the deleted lines).
                    // B-side position is the current context (`cur_b`): where
                    // in B these deletions are tracked.  This mirrors difflib's
                    // `('delete', i1, i2, j1, j1)` where `j1` ~ `cur_b`.
                    for r in gap.iter().filter(|r| r.tag == similar::ChangeTag::Delete) {
                        let a_idx = r.old_index.unwrap();
                        let a_len = r.line_count;
                        chunks.push(Chunk {
                            start_a: a_idx,
                            end_a: a_idx + a_len,
                            start_b: cur_b,
                            end_b: cur_b,
                            op: DiffOp::Delete,
                        });
                        // cur_a advances past the deleted range
                        cur_a = a_idx + a_len;
                    }
                    // cur_b stays unchanged (no new lines consumed)
                } else {
                    // Standalone Insert gap: lines added to B.
                    // B-side positions come from new_index (the inserted lines).
                    // A-side position is the current context (`cur_a`): where
                    // in A this insertion sits.  This mirrors difflib's
                    // `('insert', i1, i1, j1, j2)` where `i1` ~ `cur_a`.
                    for r in gap.iter().filter(|r| r.tag == similar::ChangeTag::Insert) {
                        let b_idx = r.new_index.unwrap();
                        let b_len = r.line_count;
                        chunks.push(Chunk {
                            start_a: cur_a,
                            end_a: cur_a,
                            start_b: b_idx,
                            end_b: b_idx + b_len,
                            op: DiffOp::Insert,
                        });
                        // cur_b advances past the inserted range
                        cur_b = b_idx + b_len;
                    }
                    // cur_a stays unchanged (no old lines consumed)
                }
            }
        }
    }

    chunks
}

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Three-way merge Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
    ///
    /// Applies both local and remote changes on top of the base.
    /// Detects conflicts when both sides modified the same base lines differently.
    pub fn merge(&self) -> ThreeWayComparison {
        let mut base_to_local = Differ::new(self.base.clone(), self.local.clone()).compare();
        let mut base_to_remote = Differ::new(self.base.clone(), self.remote.clone()).compare();

        // Merge adjacent Delete+Insert into Replace chunks for cleaner
        // conflict detection (otherwise they appear as separate chunks).
        base_to_local.chunks = merge_adjacent_replace_chunks(&base_to_local.chunks);
        base_to_remote.chunks = merge_adjacent_replace_chunks(&base_to_remote.chunks);

        let mut conflicts = Vec::new();

        use std::collections::HashSet;

        // Collect base lines that local changed (Delete/Replace on A side)
        let local_del_rep: HashSet<usize> = base_to_local
            .chunks
            .iter()
            .filter(|c| matches!(c.op, DiffOp::Delete | DiffOp::Replace))
            .flat_map(|c| c.start_a..c.end_a)
            .collect();

        let remote_del_rep: HashSet<usize> = base_to_remote
            .chunks
            .iter()
            .filter(|c| matches!(c.op, DiffOp::Delete | DiffOp::Replace))
            .flat_map(|c| c.start_a..c.end_a)
            .collect();

        // Base lines where both sides changed Ã¢â€ â€™ potential conflict
        let conflicting_base: HashSet<usize> = local_del_rep
            .intersection(&remote_del_rep)
            .copied()
            .collect();

        // Build the merged output:
        // 1. Apply local-only changes (non-conflicting)
        // 2. Apply remote-only changes (don't overlap local changes)
        let mut merged = self.base.clone();

        for chunk in &base_to_local.chunks {
            let has_conflict =
                (chunk.start_a..chunk.end_a).any(|bl| conflicting_base.contains(&bl));
            if has_conflict || chunk.op == DiffOp::Equal {
                // Conflict regions are replaced with local version below
                continue;
            }
            apply_single_chunk(&mut merged, &self.local, chunk);
        }

        let local_all: HashSet<usize> = base_to_local
            .chunks
            .iter()
            .filter(|c| !matches!(c.op, DiffOp::Equal))
            .flat_map(|c| c.start_a..c.end_a.max(c.start_a + 1))
            .collect();

        for chunk in &base_to_remote.chunks {
            let overlaps_local = (chunk.start_a..chunk.end_a).any(|bl| local_all.contains(&bl));
            let is_conflict = (chunk.start_a..chunk.end_a).any(|bl| conflicting_base.contains(&bl));
            if overlaps_local || is_conflict || chunk.op == DiffOp::Equal {
                continue;
            }
            apply_single_chunk(&mut merged, &self.remote, chunk);
        }

        // Detect actual conflicts: both changed the same base lines differently
        for &base_line in &conflicting_base {
            let lc = base_to_local.chunks.iter().find(|c| {
                matches!(c.op, DiffOp::Delete | DiffOp::Replace)
                    && c.start_a <= base_line
                    && base_line < c.end_a
            });
            let rc = base_to_remote.chunks.iter().find(|c| {
                matches!(c.op, DiffOp::Delete | DiffOp::Replace)
                    && c.start_a <= base_line
                    && base_line < c.end_a
            });
            if let (Some(lc), Some(rc)) = (lc, rc) {
                let local_lines = self.local[lc.start_b..lc.end_b.min(self.local.len())].to_vec();
                let remote_lines =
                    self.remote[rc.start_b..rc.end_b.min(self.remote.len())].to_vec();
                // If both sides made exactly the same change, no conflict
                if local_lines != remote_lines {
                    // Compute position in merged (after local-only changes applied)
                    let pos = merged.len();
                    conflicts.push(MergeConflict {
                        start_line: pos,
                        end_line: pos + local_lines.len(),
                        local: local_lines,
                        remote: remote_lines,
                    });
                    // Append local version as placeholder in merged
                    merged.extend(
                        self.local[lc.start_b..lc.end_b.min(self.local.len())]
                            .iter()
                            .cloned(),
                    );
                } else {
                    // Same change on both sides Ã¢â€ â€™ merge it
                    apply_single_chunk(&mut merged, &self.local, lc);
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

/// Apply a single diff chunk to `target`, using `source` as the replacement content.
fn apply_single_chunk(target: &mut Vec<String>, source: &[String], chunk: &Chunk) {
    match chunk.op {
        DiffOp::Delete => {
            let start = chunk.start_a.min(target.len());
            let end = chunk.end_a.min(target.len());
            if start < end {
                target.drain(start..end);
            }
        }
        DiffOp::Insert => {
            let idx = chunk.start_a.min(target.len());
            let r_end = chunk.end_b.min(source.len());
            for (j, line) in source[chunk.start_b..r_end].iter().enumerate() {
                target.insert(idx + j, line.clone());
            }
        }
        DiffOp::Replace => {
            let start = chunk.start_a.min(target.len());
            let end = chunk.end_a.min(target.len());
            if start < end {
                target.drain(start..end);
            }
            let r_end = chunk.end_b.min(source.len());
            for (j, line) in source[chunk.start_b..r_end].iter().enumerate() {
                target.insert(start + j, line.clone());
            }
        }
        DiffOp::Equal => {}
    }
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

            // For DeleteÃ¢â€ â€™Insert order: start_a from first Delete, end_a from last Delete;
            // start_b from first Insert, end_b from last Insert.
            // For InsertÃ¢â€ â€™Delete order: the opposite.
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
                // Still a replace Ã¢â‚¬â€ update bounds
                chunk.start_a = a_start;
                chunk.end_a = a_end;
                chunk.start_b = b_start;
                chunk.end_b = b_end;
                true
            } else if a_has_content {
                // Only A remains Ã¢â€ â€™ demote to Delete
                chunk.op = DiffOp::Delete;
                chunk.start_a = a_start;
                chunk.end_a = a_end;
                chunk.start_b = b_start; // zero-width
                chunk.end_b = b_start;
                a_end > a_start
            } else if b_has_content {
                // Only B remains Ã¢â€ â€™ demote to Insert
                chunk.op = DiffOp::Insert;
                chunk.start_a = a_start; // zero-width
                chunk.end_a = a_start;
                chunk.start_b = b_start;
                chunk.end_b = b_end;
                b_end > b_start
            } else {
                // Both empty Ã¢â€ â€™ remove
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

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Tokenization helper Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
            // Start of an identifier/word Ã¢â‚¬â€ split on camelCase and
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
            // Punctuation/whitespace Ã¢â‚¬â€ each character is its own token
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

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Inline (word-level) diff Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

        // Delete and Insert changes use coordinates from different
        // buffers Ã¢â‚¬â€ merging them into a single Replace produces wrong
        // bounds on one pane.  Keep them separate for per-pane accuracy.
        changes
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

    /// Compare two import lines semantically.
    ///
    /// If both lines are import statements from the same module, only
    /// the identifiers inside the braces are compared.  Otherwise falls
    /// back to token-level diffing.
    pub fn compare_imports(line_a: &str, line_b: &str) -> Vec<InlineChange> {
        let parsed_a = Self::parse_import_line(line_a);
        let parsed_b = Self::parse_import_line(line_b);

        match (parsed_a, parsed_b) {
            (Some((mod_a, ids_a)), Some((mod_b, ids_b))) if mod_a == mod_b => {
                // Build sets of identifier names for quick lookup
                let names_a: std::collections::HashSet<&str> =
                    ids_a.iter().map(|(name, _)| name.as_str()).collect();
                let names_b: std::collections::HashSet<&str> =
                    ids_b.iter().map(|(name, _)| name.as_str()).collect();

                let mut changes = Vec::new();

                // Identifiers in A but not in B are deletions
                for (name, (start, end)) in &ids_a {
                    if !names_b.contains(name.as_str()) {
                        changes.push(InlineChange {
                            start: *start,
                            end: *end,
                            op: DiffOp::Delete,
                        });
                    }
                }

                // Identifiers in B but not in A are insertions
                for (name, (start, end)) in &ids_b {
                    if !names_a.contains(name.as_str()) {
                        changes.push(InlineChange {
                            start: *start,
                            end: *end,
                            op: DiffOp::Insert,
                        });
                    }
                }

                changes
            }
            _ => {
                // Fall back to token-level diff for non-imports or
                // imports from different modules
                Self::compare_line_tokens(line_a, line_b)
            }
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

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Line cache (O(1) chunk navigation) Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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
    /// Only non-Equal chunks are returned Ã¢â‚¬â€ Equal chunks are skipped
    /// to match navigation expectations (jumping between actual changes).
    /// All three may be `None` if the line is in an Equal region.
    pub fn chunk_triad(&self, line: usize) -> (Option<usize>, Option<usize>, Option<usize>) {
        let curr = self.locate_chunk(line);
        let prev = self.prevs.get(line).copied().flatten();
        let next = self.nexts.get(line).copied().flatten();
        (prev, curr, next)
    }
}

// Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬ Diff Preprocessor Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

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

    // Ã¢â€â‚¬Ã¢â€â‚¬ Strip common prefix Ã¢â€â‚¬Ã¢â€â‚¬
    let prefix_len = text_a
        .iter()
        .zip(text_b.iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Ã¢â€â‚¬Ã¢â€â‚¬ Strip common suffix Ã¢â€â‚¬Ã¢â€â‚¬
    let suffix_len = text_a[prefix_len..]
        .iter()
        .rev()
        .zip(text_b[prefix_len..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    let a_mid = &text_a[prefix_len..len_a - suffix_len];
    let b_mid = &text_b[prefix_len..len_b - suffix_len];

    // Ã¢â€â‚¬Ã¢â€â‚¬ Discard unique lines (lines in only one file) Ã¢â€â‚¬Ã¢â€â‚¬
    // Only apply if either side discards more than 10 lines
    // (heuristic from Meld's MyersSequenceMatcher).
    use std::collections::HashSet;
    let lines_in_b: HashSet<&String> = b_mid.iter().collect();
    let lines_in_a: HashSet<&String> = a_mid.iter().collect();

    let mut filtered_a = Vec::new();
    let mut index_map_a = Vec::new();
    let mut filtered_b = Vec::new();
    let mut index_map_b = Vec::new();

    let mut discarded_a = 0usize;
    let mut discarded_b = 0usize;

    for (i, line) in a_mid.iter().enumerate() {
        if lines_in_b.contains(line) || line.is_empty() {
            filtered_a.push(line.clone());
            index_map_a.push(prefix_len + i);
        } else {
            discarded_a += 1;
        }
    }

    for (i, line) in b_mid.iter().enumerate() {
        if lines_in_a.contains(line) || line.is_empty() {
            filtered_b.push(line.clone());
            index_map_b.push(prefix_len + i);
        } else {
            discarded_b += 1;
        }
    }

    // Only use the filtered version if enough lines were discarded
    // on either side.  Mirrors Python Meld's heuristic.
    if discarded_a <= 10 && discarded_b <= 10 {
        // Not worth it Ã¢â‚¬â€ return the full middle section (prefix/suffix
        // already stripped so the diff only processes middle lines).
        // Prefix and suffix themselves are re-inserted by `compare()`.
        // The index maps must map filtered (middle-section) positions to
        // original (full-text) positions.
        let full_a = text_a[prefix_len..len_a - suffix_len].to_vec();
        let full_b = text_b[prefix_len..len_b - suffix_len].to_vec();
        // Include one extra sentinel entry so that a chunk referencing
        // the position just past the last filtered line (e.g., a Delete
        // whose B-side position equals the length of the filtered B
        // sequence) can be remapped to the correct original position.
        let map_a: Vec<usize> = (0..=full_a.len()).map(|i| prefix_len + i).collect();
        let map_b: Vec<usize> = (0..=full_b.len()).map(|i| prefix_len + i).collect();
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
        // Include sentinel entry: position `filtered_a.len()` (one past
        // the last filtered line) maps to the end of the middle section
        // in the original A text.
        index_map_a: {
            let mut m = index_map_a;
            m.push(len_a - suffix_len);
            m
        },
        index_map_b: {
            let mut m = index_map_b;
            m.push(len_b - suffix_len);
            m
        },
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

        // Ã¢â€â‚¬Ã¢â€â‚¬ A side Ã¢â€â‚¬Ã¢â€â‚¬
        if f_start_a < pre.index_map_a.len() {
            chunk.start_a = pre.index_map_a[f_start_a];
        }
        if f_start_a == f_end_a {
            // Zero-width chunk: end maps to the same original position as start
            chunk.end_a = chunk.start_a;
        } else if f_end_a > 0 && f_end_a - 1 < pre.index_map_a.len() {
            chunk.end_a = pre.index_map_a[f_end_a - 1] + 1;
        }

        // Ã¢â€â‚¬Ã¢â€â‚¬ B side Ã¢â€â‚¬Ã¢â€â‚¬
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
        // Delete, Delete, Insert, Insert Ã¢â€ â€™ single Replace
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
        // Equal, Delete, Insert, Equal Ã¢â€ â€™ Replace should NOT merge across Equal
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

    /// Reproduce the issue from the user's bug report:
    /// Left panel has 13 import lines, right panel has 14 import lines
    /// with `import { useAlert } from 'react-alert';` added.
    /// Meld Python detects this as an Insert; meld-rs should too.
    #[test]
    fn test_detect_inserted_import_line() {
        let left: Vec<String> = vec![
            "import { useOidcAccessToken } from '@axa-fr/react-oidc';",
            "import { isAny } from 'bpmn-js/lib/features/modeling/util/ModelingUtil';",
            "import { ModdleElement } from 'bpmn-js/lib/model/Types';",
            "import { getBusinessObject, is } from 'bpmn-js/lib/util/ModelUtil';",
            "import { IRouteParams } from 'rits-shared';",
            "import { EnvironmentContext, isViewPermissionOnly, getAccountGuid, useSwallowNotification } from 'rits-ui-shared';",
            "import BpmnModeler from 'camunda-bpmn-js/lib/camunda-cloud/Modeler';",
            "import isEmpty from 'lodash/isEmpty';",
            "import { useBeforeunload } from 'react-beforeunload';",
            "import { useTranslation } from 'react-i18next';",
            "import { useHistory, useParams, Prompt, Link } from 'react-router-dom';",
            "import { useRecoilState, useRecoilValue, useSetRecoilState } from 'recoil';",
            "import { v4 as uuidv4 } from 'uuid';",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();

        let right: Vec<String> = vec![
            "import { useOidcAccessToken } from '@axa-fr/react-oidc';",
            "import { isAny } from 'bpmn-js/lib/features/modeling/util/ModelingUtil';",
            "import { ModdleElement } from 'bpmn-js/lib/model/Types';",
            "import { getBusinessObject, is } from 'bpmn-js/lib/util/ModelUtil';",
            "import { IRouteParams } from 'rits-shared';",
            "import { isViewPermissionOnly, notifySuccess, notifyWarning, notifyError, getAccountGuid } from 'rits-ui-shared';",
            "import { EnvironmentContext } from 'rits-ui-shared';",
            "import BpmnModeler from 'camunda-bpmn-js/lib/camunda-cloud/Modeler';",
            "import isEmpty from 'lodash/isEmpty';",
            "import { useAlert } from 'react-alert';",
            "import { useBeforeunload } from 'react-beforeunload';",
            "import { useTranslation } from 'react-i18next';",
            "import { useHistory, useParams, Prompt, Link } from 'react-router-dom';",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();

        let differ = Differ::new(left, right);
        let result = differ.compare();

        // Merge adjacent replace chunks (as done in compute_diff)
        let merged = merge_adjacent_replace_chunks(&result.chunks);

        // We expect at least one Insert chunk representing the added import line
        let inserts: Vec<&Chunk> = merged.iter().filter(|c| c.op == DiffOp::Insert).collect();

        assert!(
            !inserts.is_empty(),
            "Expected at least one Insert chunk for the added 'useAlert' import line"
        );

        // The Insert should be at the correct line: right line 9 (0-indexed)
        // is "import { useAlert } from 'react-alert';"
        let use_alert_insert = inserts.iter().find(|c| c.start_b == 9 && c.end_b == 10);
        assert!(
            use_alert_insert.is_some(),
            "Expected an Insert chunk at B[9..10) for 'useAlert' line, \
             got inserts: {:?}",
            inserts
                .iter()
                .map(|c| format!("B[{}..{})", c.start_b, c.end_b))
                .collect::<Vec<_>>()
        );

        // Verify the A-side position is correct: the Insert should be
        // placed AT the position where the insertion occurs in A.
        // A[7]="isEmpty" = B[8], and A[8]="useBeforeunload" = B[10].
        // The unique B line B[9]="useAlert" sits between them.
        // In difflib semantics, Insert(i1, i1, j1, j2) means: at A position
        // i1, B has extra lines. Here i1=8 (after A[7], before A[8]).
        // The next Equal block shares the same A start (8), which is the
        // standard difflib convention for zero-width Insert chunks.
        if let Some(c) = use_alert_insert {
            assert!(
                c.start_a == c.end_a,
                "Insert chunk should be zero-width on A side, got A[{}..{})",
                c.start_a,
                c.end_a
            );
            assert_eq!(
                c.start_a, 8,
                "Insert start_a should be 8 (between 'isEmpty' L7 and 'useBeforeunload' L8), got {}",
                c.start_a
            );
        }

        // Also verify that the delete of the two remaining left-only lines
        // (recoil at L11, uuid at L12) are at the correct B-side position.
        let deletes: Vec<&Chunk> = merged.iter().filter(|c| c.op == DiffOp::Delete).collect();
        assert_eq!(
            deletes.len(),
            2,
            "Expected 2 Delete chunks for recoil and uuid lines"
        );
        // Both deletes should be at the end of B (position 13, after all 13 B lines)
        for del in &deletes {
            assert_eq!(
                del.start_b, 13,
                "Delete chunks should be at B position 13 (end of B), got B[{}..{})",
                del.start_b, del.end_b
            );
            assert_eq!(
                del.start_b, del.end_b,
                "Delete should be zero-width on B side"
            );
        }
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ consume_blank_lines tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn test_consume_blank_lines_removes_empty_chunk() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 2,
            start_b: 1,
            end_b: 2,
            op: DiffOp::Replace,
        }];
        let text_a = vec!["x".into(), "".into(), "y".into()];
        let text_b = vec!["x".into(), "".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        // Both sides contain only a blank line Ã¢â€ â€™ chunk is removed
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_consume_blank_lines_demote_replace_to_delete() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 2,
            start_b: 1,
            end_b: 2,
            op: DiffOp::Replace,
        }];
        let text_a = vec!["x".into(), "real".into(), "y".into()];
        let text_b = vec!["x".into(), "".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        // A has content, B is blank Ã¢â€ â€™ demote to Delete
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Delete);
        assert_eq!(chunks[0].start_a, 1);
        assert_eq!(chunks[0].end_a, 2);
    }

    #[test]
    fn test_consume_blank_lines_demote_replace_to_insert() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 2,
            start_b: 1,
            end_b: 2,
            op: DiffOp::Replace,
        }];
        let text_a = vec!["x".into(), "".into(), "y".into()];
        let text_b = vec!["x".into(), "real".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        // B has content, A is blank Ã¢â€ â€™ demote to Insert
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Insert);
        assert_eq!(chunks[0].start_b, 1);
        assert_eq!(chunks[0].end_b, 2);
    }

    #[test]
    fn test_consume_blank_lines_preserves_replace_with_content_on_both_sides() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 3,
            start_b: 1,
            end_b: 3,
            op: DiffOp::Replace,
        }];
        let text_a = vec!["x".into(), "a".into(), "b".into(), "y".into()];
        let text_b = vec!["x".into(), "c".into(), "d".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Replace);
    }

    #[test]
    fn test_consume_blank_lines_trims_leading_blanks_in_replace() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 4,
            start_b: 1,
            end_b: 4,
            op: DiffOp::Replace,
        }];
        // A: ["", "", "hello", x]  B: ["", "", "world", x]
        // Leading blanks should be trimmed
        let text_a = vec!["x".into(), "".into(), "".into(), "hello".into(), "y".into()];
        let text_b = vec!["x".into(), "".into(), "".into(), "world".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Replace);
        // Should have trimmed the two leading blanks from both sides
        assert_eq!(chunks[0].start_a, 3);
        assert_eq!(chunks[0].start_b, 3);
    }

    #[test]
    fn test_consume_blank_lines_trims_trailing_blanks_in_replace() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 4,
            start_b: 1,
            end_b: 4,
            op: DiffOp::Replace,
        }];
        let text_a = vec!["x".into(), "hello".into(), "".into(), "".into(), "y".into()];
        let text_b = vec!["x".into(), "world".into(), "".into(), "".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Replace);
        assert_eq!(chunks[0].end_a, 2);
        assert_eq!(chunks[0].end_b, 2);
    }

    #[test]
    fn test_consume_blank_lines_does_not_trim_blanks_in_middle() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 4,
            start_b: 1,
            end_b: 4,
            op: DiffOp::Replace,
        }];
        // Blank in the middle should stay
        let text_a = vec![
            "x".into(),
            "hello".into(),
            "".into(),
            "world".into(),
            "y".into(),
        ];
        let text_b = vec!["x".into(), "a".into(), "".into(), "b".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Replace);
        assert_eq!(chunks[0].start_a, 1);
        assert_eq!(chunks[0].end_a, 4);
    }

    #[test]
    fn test_consume_blank_lines_preserves_delete_with_content() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 3,
            start_b: 1,
            end_b: 1,
            op: DiffOp::Delete,
        }];
        let text_a = vec!["x".into(), "".into(), "real".into(), "y".into()];
        let text_b = vec!["x".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        // Delete with leaning blank but real content should survive
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].op, DiffOp::Delete);
        assert_eq!(chunks[0].start_a, 2);
        assert_eq!(chunks[0].end_a, 3);
    }

    #[test]
    fn test_consume_blank_lines_removes_pure_blank_delete() {
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 2,
            start_b: 1,
            end_b: 1,
            op: DiffOp::Delete,
        }];
        let text_a = vec!["x".into(), "".into(), "y".into()];
        let text_b = vec!["x".into(), "y".into()];
        consume_blank_lines(&mut chunks, &text_a, &text_b);
        assert!(chunks.is_empty());
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ preprocess_diff / unprocess_chunks round-trip tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn test_preprocess_identical_texts() {
        let a: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let b = a.clone();
        let pre = preprocess_diff(&a, &b);
        // All lines are common Ã¢â€ â€™ prefix covers everything, mid is empty
        assert_eq!(pre.prefix_len, 3);
        assert_eq!(pre.suffix_len, 0);
        assert!(pre.filtered_a.is_empty());
        assert!(pre.filtered_b.is_empty());
    }

    #[test]
    fn test_preprocess_strips_common_prefix_and_suffix() {
        let a: Vec<String> = vec!["A".into(), "B".into(), "X".into(), "C".into(), "D".into()];
        let b: Vec<String> = vec!["A".into(), "B".into(), "Y".into(), "C".into(), "D".into()];
        let pre = preprocess_diff(&a, &b);
        assert_eq!(pre.prefix_len, 2);
        assert_eq!(pre.suffix_len, 2);
        assert_eq!(pre.filtered_a, vec!["X"]);
        assert_eq!(pre.filtered_b, vec!["Y"]);
    }

    #[test]
    fn test_preprocess_discards_unique_lines_beyond_threshold() {
        // 12 unique lines on A side Ã¢â€ â€™ should trigger discard
        let a: Vec<String> = (0..12).map(|i| format!("unique_a_{}", i)).collect();
        let b: Vec<String> = vec!["common".into()];
        let pre = preprocess_diff(&a, &b);
        assert!(pre.filtered_a.len() < a.len());
    }

    #[test]
    fn test_preprocess_keeps_unique_lines_below_threshold() {
        // Only 5 unique lines on A side Ã¢â€ â€™ heuristic says keep them
        let a: Vec<String> = (0..5).map(|i| format!("unique_a_{}", i)).collect();
        let b: Vec<String> = vec!["common".into()];
        let pre = preprocess_diff(&a, &b);
        // Since <10 lines discarded, the full input is kept
        assert_eq!(pre.filtered_a.len(), a.len());
        assert_eq!(pre.filtered_b.len(), b.len());
    }

    #[test]
    fn test_preprocess_empty_inputs() {
        let a: Vec<String> = vec![];
        let b: Vec<String> = vec![];
        let pre = preprocess_diff(&a, &b);
        assert_eq!(pre.prefix_len, 0);
        assert_eq!(pre.suffix_len, 0);
        assert!(pre.filtered_a.is_empty());
        assert!(pre.filtered_b.is_empty());
    }

    #[test]
    fn test_unprocess_chunks_round_trip() {
        // Full round-trip: preprocess Ã¢â€ â€™ mock diff Ã¢â€ â€™ unprocess
        let a: Vec<String> = vec!["P".into(), "A".into(), "B".into(), "C".into(), "S".into()];
        let b: Vec<String> = vec!["P".into(), "A".into(), "X".into(), "C".into(), "S".into()];
        let pre = preprocess_diff(&a, &b);
        // A[2]="B" vs B[2]="X" Ã¢â€ â€™ Replace
        let mut chunks = vec![Chunk {
            start_a: 0,
            end_a: 1,
            start_b: 0,
            end_b: 1,
            op: DiffOp::Replace,
        }];
        unprocess_chunks(&mut chunks, &pre);
        // After unprocessing with prefix=2, suffix=2, the chunk should map
        // filtered[0]=A[2] Ã¢â€ â€™ original index 2
        assert_eq!(chunks[0].start_a, 2);
        assert_eq!(chunks[0].end_a, 3);
        assert_eq!(chunks[0].start_b, 2);
        assert_eq!(chunks[0].end_b, 3);
    }

    #[test]
    fn test_unprocess_chunks_zero_width_insert() {
        let a: Vec<String> = vec!["A".into(), "C".into()];
        let b: Vec<String> = vec!["A".into(), "B".into(), "C".into()];
        let pre = preprocess_diff(&a, &b);
        // Insert chunk: zero-width on A side
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 1,
            start_b: 1,
            end_b: 2,
            op: DiffOp::Insert,
        }];
        unprocess_chunks(&mut chunks, &pre);
        assert_eq!(
            chunks[0].start_a, chunks[0].end_a,
            "Insert should stay zero-width on A"
        );
    }

    #[test]
    fn test_unprocess_chunks_zero_width_delete() {
        let a: Vec<String> = vec!["A".into(), "B".into(), "C".into()];
        let b: Vec<String> = vec!["A".into(), "C".into()];
        let pre = preprocess_diff(&a, &b);
        // Delete chunk: zero-width on B side
        let mut chunks = vec![Chunk {
            start_a: 1,
            end_a: 2,
            start_b: 1,
            end_b: 1,
            op: DiffOp::Delete,
        }];
        unprocess_chunks(&mut chunks, &pre);
        assert_eq!(
            chunks[0].start_b, chunks[0].end_b,
            "Delete should stay zero-width on B"
        );
    }

    #[test]
    fn test_preprocess_unprocess_full_diff_round_trip() {
        // Complete diff pipeline: preprocess Ã¢â€ â€™ diff Ã¢â€ â€™ unprocess
        // should produce correct chunks for the original text
        let a: Vec<String> = vec!["common".into(), "old".into(), "end".into()];
        let b: Vec<String> = vec!["common".into(), "new".into(), "end".into()];
        let pre = preprocess_diff(&a, &b);
        // Middle: ["old"] vs ["new"]

        let a_joined = pre.filtered_a.join("\n") + "\n";
        let b_joined = pre.filtered_b.join("\n") + "\n";
        let diff = similar::TextDiff::from_lines(&a_joined, &b_joined);
        let mut chunks = build_chunks_from_gaps(&diff);

        unprocess_chunks(&mut chunks, &pre);

        assert!(!chunks.is_empty());
        // Should have a Replace chunk at index 1 for both sides
        let replace: Vec<_> = chunks.iter().filter(|c| c.op == DiffOp::Replace).collect();
        assert_eq!(replace.len(), 1);
        assert_eq!(replace[0].start_a, 1);
        assert_eq!(replace[0].end_a, 2);
        assert_eq!(replace[0].start_b, 1);
        assert_eq!(replace[0].end_b, 2);
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ InlineDiffer tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn test_compare_line_identical() {
        let changes = InlineDiffer::compare_line("hello", "hello");
        assert!(changes.is_empty());
    }

    #[test]
    fn test_compare_line_single_word_change() {
        let changes = InlineDiffer::compare_line("hello world", "hello there");
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_compare_line_completely_different() {
        let changes = InlineDiffer::compare_line("abcdef", "ghijkl");
        // Should still produce changes, even if everything is different
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_compare_line_tokens_camelcase_split() {
        let changes = InlineDiffer::compare_line_tokens("fooBarBaz", "fooBarQux");
        assert!(!changes.is_empty());
        // The differing token should be "Baz" vs "Qux"
    }

    #[test]
    fn test_compare_line_tokens_same_string() {
        let changes = InlineDiffer::compare_line_tokens("same", "same");
        assert!(changes.is_empty());
    }

    #[test]
    fn test_compare_line_tokens_import_change() {
        let a = "import { useAlert } from 'react-alert';";
        let b = "import { useOidcAccessToken } from '@axa-fr/react-oidc';";
        let changes = InlineDiffer::compare_line_tokens(a, b);
        // Should detect differences in both the import names and module path
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_parse_import_line_basic() {
        let line = "import { Foo, Bar } from 'my-module';";
        let result = InlineDiffer::parse_import_line(line);
        assert!(result.is_some());
        let (module, identifiers) = result.unwrap();
        assert_eq!(module, "my-module");
        assert_eq!(identifiers.len(), 2);
        assert_eq!(identifiers[0].0, "Foo");
        assert_eq!(identifiers[1].0, "Bar");
    }

    #[test]
    fn test_parse_import_line_not_an_import() {
        let line = "const x = 42;";
        assert!(InlineDiffer::parse_import_line(line).is_none());
    }

    #[test]
    fn test_parse_import_line_no_braces() {
        let line = "import something from 'module';";
        assert!(InlineDiffer::parse_import_line(line).is_none());
    }

    // Ã¢â€â‚¬Ã¢â€â‚¬ Three-way merge tests Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬Ã¢â€â‚¬

    #[test]
    fn test_compare_imports_same_module_added_identifier() {
        let a = "import { useAlert } from 'react-alert';";
        let b = "import { useAlert, useConfirm } from 'react-alert';";
        let changes = InlineDiffer::compare_imports(a, b);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].op, DiffOp::Insert);
    }

    #[test]
    fn test_compare_imports_same_module_removed_identifier() {
        let a = "import { useAlert, useConfirm } from 'react-alert';";
        let b = "import { useAlert } from 'react-alert';";
        let changes = InlineDiffer::compare_imports(a, b);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].op, DiffOp::Delete);
    }

    #[test]
    fn test_compare_imports_multiple_changes() {
        let a = "import { Foo, Bar, Baz } from 'mylib';";
        let b = "import { Bar, Qux } from 'mylib';";
        let changes = InlineDiffer::compare_imports(a, b);
        let deletes: Vec<_> = changes.iter().filter(|c| c.op == DiffOp::Delete).collect();
        let inserts: Vec<_> = changes.iter().filter(|c| c.op == DiffOp::Insert).collect();
        assert_eq!(deletes.len(), 2);
        assert_eq!(inserts.len(), 1);
    }

    #[test]
    fn test_compare_imports_different_modules_falls_back() {
        let a = "import { Foo } from 'lib-a';";
        let b = "import { Foo } from 'lib-b';";
        let changes = InlineDiffer::compare_imports(a, b);
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_compare_imports_not_imports_falls_back() {
        let a = "const x = 42;";
        let b = "const y = 42;";
        let changes = InlineDiffer::compare_imports(a, b);
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_compare_imports_identical() {
        let a = "import { Foo } from 'lib';";
        let changes = InlineDiffer::compare_imports(a, a);
        assert!(changes.is_empty());
    }

    #[test]
    fn test_three_way_merge_conflicting_changes() {
        let base = vec!["a".into(), "b".into(), "c".into()];
        let local = vec!["a".into(), "b_local".into(), "c".into()];
        let remote = vec!["a".into(), "b_remote".into(), "c".into()];
        let differ = ThreeWayDiffer::new(base, local, remote);
        let result = differ.merge();
        assert!(!result.merged.is_empty());
        // Should have conflicts since both sides changed the same line
        assert!(!result.conflicts.is_empty());
    }

    #[test]
    fn test_three_way_merge_same_changes_no_conflict() {
        let base = vec!["a".into(), "b".into(), "c".into()];
        let local = vec!["a".into(), "b2".into(), "c".into()];
        let remote = vec!["a".into(), "b2".into(), "c".into()];
        let differ = ThreeWayDiffer::new(base, local, remote);
        let result = differ.merge();
        assert!(!result.merged.is_empty());
        assert!(result.conflicts.is_empty());
        assert_eq!(result.merged[1], "b2");
    }

    #[test]
    fn test_three_way_merge_local_only_change() {
        let base = vec!["a".into(), "b".into(), "c".into()];
        let local = vec!["a".into(), "b_local".into(), "c".into()];
        let remote = base.clone();
        let differ = ThreeWayDiffer::new(base, local, remote);
        let result = differ.merge();
        assert!(!result.merged.is_empty());
        assert!(result.conflicts.is_empty());
        assert_eq!(result.merged[1], "b_local");
    }
    // â”€â”€ Sync-point tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn test_sync_points_no_points_behaves_like_normal_diff() {
        let a: Vec<String> = vec!["0".into(), "1".into(), "2".into()];
        let b: Vec<String> = vec!["0".into(), "x".into(), "2".into()];
        let normal = Differ::new(a.clone(), b.clone()).compare();
        let with_empty = Differ::new(a, b).with_sync_points(vec![]).compare();
        assert_eq!(normal.chunks.len(), with_empty.chunks.len());
        for (c1, c2) in normal.chunks.iter().zip(with_empty.chunks.iter()) {
            assert_eq!(c1.op, c2.op);
            assert_eq!(c1.start_a, c2.start_a);
            assert_eq!(c1.end_a, c2.end_a);
        }
    }

    #[test]
    fn test_sync_points_forces_split() {
        let a: Vec<String> = "012a3456c789".chars().map(|c| c.to_string()).collect();
        let b: Vec<String> = "0a3412b5678".chars().map(|c| c.to_string()).collect();
        let result = Differ::new(a, b).with_sync_points(vec![(3, 6)]).compare();
        assert!(!result.chunks.is_empty());
        let mut a_covered = 0usize;
        let mut b_covered = 0usize;
        for chunk in &result.chunks {
            assert_eq!(chunk.start_a, a_covered);
            assert_eq!(chunk.start_b, b_covered);
            a_covered = chunk.end_a;
            b_covered = chunk.end_b;
        }
        assert_eq!(a_covered, 12);
        assert_eq!(b_covered, 11);
    }

    #[test]
    fn test_sync_points_multiple_boundaries() {
        let a: Vec<String> = "012a3456c789".chars().map(|c| c.to_string()).collect();
        let b: Vec<String> = "02a341b5678".chars().map(|c| c.to_string()).collect();
        let result = Differ::new(a, b)
            .with_sync_points(vec![(3, 2), (8, 6)])
            .compare();
        assert!(!result.chunks.is_empty());
        let mut a_covered = 0usize;
        let mut b_covered = 0usize;
        for chunk in &result.chunks {
            assert_eq!(chunk.start_a, a_covered);
            assert_eq!(chunk.start_b, b_covered);
            a_covered = chunk.end_a;
            b_covered = chunk.end_b;
        }
        assert_eq!(a_covered, 12);
        assert_eq!(b_covered, 11);
    }

    #[test]
    fn test_sync_points_same_content_different_with_without_points() {
        let a: Vec<String> = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        let b: Vec<String> = vec!["A".into(), "C".into(), "D".into()];
        let with_sync = Differ::new(a.clone(), b.clone())
            .with_sync_points(vec![(0, 0)])
            .compare();
        let without = Differ::new(a, b).compare();
        assert_eq!(with_sync.chunks.len(), without.chunks.len());
    }

    #[test]
    fn test_sync_points_out_of_bounds_filtered() {
        let a: Vec<String> = vec!["x".into(), "y".into()];
        let b: Vec<String> = vec!["x".into(), "y".into()];
        let result = Differ::new(a, b).with_sync_points(vec![(99, 99)]).compare();
        assert!(!result.chunks.is_empty());
    }

    #[test]
    fn test_sync_points_duplicate_points_deduplicated() {
        let a: Vec<String> = "abc".chars().map(|c| c.to_string()).collect();
        let b: Vec<String> = "abc".chars().map(|c| c.to_string()).collect();
        let single = Differ::new(a.clone(), b.clone())
            .with_sync_points(vec![(1, 1)])
            .compare();
        let duped = Differ::new(a, b)
            .with_sync_points(vec![(1, 1), (1, 1)])
            .compare();
        assert_eq!(single.chunks.len(), duped.chunks.len());
    }
}
