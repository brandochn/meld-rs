# Manual GUI checklist: issue #45 (large-file performance)

CI only runs `cargo test --no-default-features` and builds the AppImage, so the
GTK behavior must be checked by hand. Use the reporter's generator scripts
(issue #45 comments) or the `Block55` shapes from
`tests/large_file_perf.rs` at 10 / 50 / 100 MB.

Run: `./scripts/run.ps1` (Windows) or `./scripts/run.sh` (Linux).

## 1. Inline coverage (Step 1)

- [ ] `same-10MB` vs `diff-10MB`: inline/token highlighting continues past
      line 40000 (previously stopped around 40005).
- [ ] `same-10MB` vs `inli-10MB`: same, no stop around line 40000.
- [ ] `same-50MB` vs `inli-50MB`: highlighting reaches the end; the UI stays
      responsive while it fills in.
- [ ] Small file (< 1 MB) looks identical to before.

## 2. Busy indicator (Step 2)

- [ ] `same-50MB` vs `same-50MB`: the "...large files..." banner / wait cursor
      stays until the end of the file is reachable.
- [ ] It always disappears eventually (hard timeout) and never leaves the wait
      cursor stuck.
- [ ] Closing the tab while busy does not panic or leave a timer running.

## 3. Scroll / overview (Step 3)

- [ ] `same-100MB` vs `inli-100MB`: click the right pane, press `Ctrl-End`
      repeatedly. It reaches the end without bouncing back to ~17000/30000/...
- [ ] Sync works normally once layout is complete (top/bottom, page scroll).
- [ ] Same with the overview map enabled and disabled.
- [ ] 3-pane comparison still scroll-syncs panes 1 and 2 as before.

## 4. Editing / regressions

- [ ] Edit a line in a large file; the diff re-computes and the edited line is
      highlighted correctly.
- [ ] Undo/redo still works.
- [ ] Ignore-blank-lines and text filters still change the diff as expected
      (`LineCache` is now built on the worker).
- [ ] Scroll/resize on `same-50MB` vs `diff-50MB` is responsive (insert overlay
      is viewport-clipped).
- [ ] No `RefCell already borrowed` panic, no scroll feedback loop.

## 5. Larger shapes

- [ ] `same-50MB` vs `diff-50MB` and `inli-50MB` complete and remain editable.
- [ ] `same-100MB` vs `inli-100MB` does not block the UI for minutes.

## 6. Overview progress + busy completion (follow-up)

- [ ] During processing the overview shows a muted full-length track and fills
      in as markup is applied.
- [ ] The "...large files..." banner / wait cursor stays until the overview is
      fully filled (not just until the diff finishes).
- [ ] It never gets stuck (stall timeout ~10 s without progress).
- [ ] Overview off: busy still behaves correctly; no CPU spike.
- [ ] Empty / identical files: no gray track, busy clears normally.
- [ ] Scrolling does not cause an overview-redraw CPU spike (redraws are
      throttled to ~100 ms / 0.5% progress).
- [ ] 3-pane comparison: overview progress is approximate but does not error.

## 7. GtkLabel warnings (follow-up)

Diagnosis so far (see the issue thread for the reporter's counts):

- The message comes from GTK itself when a widget is snapshotted while its
  allocation is still invalid; the tooltip window contains a `GtkLabel`, and
  the same warning is seen across GTK apps (e.g. gnome-text-editor), so it is
  an allocation-timing artefact rather than a meld-rs widget.
- Not reproducible on GTK 4.22.4 here: 0 warnings while loading a 23 MB
  synthetic file and while programmatically moving the cursor back and forth
  across a changed/unchanged chunk (the reporter's "click a blue line" path),
  on both the cairo and GL renderers over X11 and Wayland.
- The reporter relates the clicks/hover to the header action buttons, which are
  re-enabled/disabled as the current chunk changes. The refresh is now
  coalesced into one idle update per main-loop iteration and only applied when
  a state actually changes, so bound buttons (and their tooltips) are no longer
  invalidated mid-frame.

Manual re-check on an environment that reproduces it (reporter's GTK version):

- [ ] Count `Gtk-WARNING: ... without a current allocation` while loading real
      ~50 MB JSON files (reporter saw 557).
- [ ] Count warnings on repeated blue<->white clicks and on hovering between
      the tab and an enabled header button.
- [ ] If the count persists, it is expected to be the upstream tooltip /
      allocation-timing issue; capture the `0x...` widget pointer and GTK
      version for the upstream report.
