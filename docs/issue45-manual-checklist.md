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
