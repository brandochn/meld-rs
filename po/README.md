# Translation catalogs

These gettext `.po` catalogs are taken from the reference **GNOME Meld** project
and vendored here so that meld-rs can ship the same set of UI translations.

- Upstream: <https://gitlab.gnome.org/GNOME/meld> (`po/` directory)
- License: GPL-2.0-or-later (same as meld-rs) — see the repository `LICENSE`.
- `LINGUAS` lists the 50 supported languages; `*.po` are the per-language
  catalogs (translator credits are preserved in each file header).

Catalog keys are the English `msgid` strings, so translations apply to any
meld-rs user-visible string that matches Meld's wording exactly. Strings unique
to meld-rs fall back to English until a translation exists.

Compiled `.mo` files are build artifacts and are not committed.

## Shipping the catalogs

`build.rs` compiles these catalogs with `msgfmt` into
`target/<profile>/share/locale/<lang>/LC_MESSAGES/meld-rs.mo`. At runtime the
application looks for them at `<executable dir>/share/locale` (overridable with
the `MELD_RS_LOCALEDIR` environment variable), then at `/usr/share/locale` on
Unix.

When packaging a build, the `share/locale` tree must ship next to the binary;
if it is missing, the UI silently falls back to English. `scripts/copy-dlls.ps1`
warns if the catalogs are absent from the distribution directory.

If `msgfmt` is not installed at build time, the build still succeeds with a
warning and the resulting binary is English-only.

## Maintenance: do NOT run `msgmerge` on these catalogs

These catalogs are treated as **read-only, vendored upstream data**. They must
not be regenerated or merged against a meld-rs-generated template:

- meld-rs contributes ~150 msgids, while Meld's catalogs contain ~547. Running
  `msgmerge --update po/<lang>.po meld-rs.pot` therefore marks the bulk of the
  catalog obsolete (measured on `de.po`: translated entries dropped 547 → 7,
  with 660 entries obsoleted). Obsolete entries are omitted from the compiled
  `.mo`, so this silently removes most translations.
- GNU gettext has no Rust support (`xgettext` reports `language 'Rust' unknown`),
  so a meld-rs template cannot be produced with `xgettext` anyway.

If meld-rs-only strings ever need translating, the researched path is:

1. extract with a Rust-aware tool (`xtr`, or a small custom `syn`/
   `proc_macro2` extractor) into `po/meld-rs.pot`; then
2. merge **non-destructively** with `msgcat --use-first` (a union that keeps
   every existing translation and appends new msgids untranslated), never with
   `msgmerge --update`;

or, alternatively, keep a separate catalog domain for meld-rs-only strings and
add it to the runtime lookup as a fallback behind the Meld domain.
