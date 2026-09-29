//! Automated verification of the i18n implementation.
//!
//! This is the regression suite for the vendored catalogs and the `tr!` marking
//! pass. It runs headlessly under `cargo test --no-default-features` (no GTK).
//!
//! Everything lives in one test binary so that mutating the process environment
//! and the global catalog state cannot race with other tests.
//!
//! Checks performed:
//! 1. every language in `po/LINGUAS` has a compiled, parsable catalog of a
//!    plausible size (guards the `.po` → `.mo` build step and the catastrophic
//!    translation-loss scenario that a `msgmerge` against these catalogs causes);
//! 2. the runtime loader selects each of those catalogs;
//! 3. real plural rules from a vendored catalog select the right forms;
//! 4. representative translations and both fallback paths work;
//! 5. `MELD_RS_LOCALEDIR` is honoured;
//! 6. no user-visible literal reached a text-setting API without going through
//!    `tr!` (the invariant that keeps future UI strings translatable).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// Tests in one binary run on parallel threads and share the process
/// environment and the global catalog, so every test that mutates them must
/// hold this lock for its whole body.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn env_guard() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Minimum translated entries we expect in any single catalog. The smallest
/// vendored catalogs currently hold ~28 entries; this floor catches wholesale
/// loss (the `msgmerge` hazard drops a catalog to single digits) without being
/// fragile when translations are updated.
const MIN_ENTRIES_PER_CATALOG: usize = 20;

/// Minimum total translated entries across all catalogs. Currently ~24,000.
const MIN_TOTAL_ENTRIES: usize = 15_000;

fn manifest_path(relative: &str) -> PathBuf {
    Path::new(MANIFEST_DIR).join(relative)
}

/// Locate the directory `build.rs` compiles catalogs into
/// (`target/<profile>/share/locale`).
fn built_locale_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("MELD_RS_LOCALEDIR") {
        let dir = PathBuf::from(dir);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    let exe = std::env::current_exe().ok()?;
    // .../target/<profile>/deps/<test-bin> -> .../target/<profile>
    let profile_dir = exe.parent()?.parent()?;
    let dir = profile_dir.join("share").join("locale");
    dir.is_dir().then_some(dir)
}

/// Set the translation-related environment to a single, deterministic value.
fn set_locale(locale_dir: &Path, language: &str) {
    std::env::set_var("MELD_RS_LOCALEDIR", locale_dir);
    std::env::set_var("LANGUAGE", language);
    std::env::remove_var("LC_ALL");
    std::env::remove_var("LC_MESSAGES");
    std::env::remove_var("LANG");
}

/// The languages listed in `po/LINGUAS` (blank lines and `#` comments ignored).
fn vendored_languages() -> Vec<String> {
    let text = std::fs::read_to_string(manifest_path("po/LINGUAS")).expect("read po/LINGUAS");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// A locally-relevant, translated msgid used as a canary across checks.
const CANARY_MSGID: &str = "Start a new comparison";
const CANARY_GERMAN: &str = "Neuen Vergleich starten";

#[test]
fn every_vendored_language_has_a_loadable_catalog() {
    let Some(locale_dir) = built_locale_dir() else {
        eprintln!("skipping: no compiled locale dir (msgfmt unavailable at build time)");
        return;
    };

    let languages = vendored_languages();
    assert_eq!(
        languages.len(),
        50,
        "expected 50 vendored languages, found {}",
        languages.len()
    );

    let _guard = env_guard();
    let mut missing = Vec::new();
    let mut too_small = Vec::new();
    let mut total = 0usize;

    for language in &languages {
        let path = locale_dir
            .join(language)
            .join("LC_MESSAGES")
            .join("meld-rs.mo");
        let Ok(bytes) = std::fs::read(&path) else {
            missing.push(language.clone());
            continue;
        };
        let Some(catalog) = meld_rs::i18n::parse_mo(&bytes) else {
            missing.push(format!("{language} (unparsable)"));
            continue;
        };
        total += catalog.len();
        if catalog.len() < MIN_ENTRIES_PER_CATALOG {
            too_small.push(format!("{language}={}", catalog.len()));
        }
    }

    assert!(
        missing.is_empty(),
        "catalogs missing/unparsable: {missing:?}"
    );
    assert!(
        too_small.is_empty(),
        "catalogs with fewer than {MIN_ENTRIES_PER_CATALOG} translated entries: {too_small:?}"
    );
    assert!(
        total >= MIN_TOTAL_ENTRIES,
        "total translated entries dropped to {total} (expected >= {MIN_TOTAL_ENTRIES})"
    );
}

#[test]
fn every_vendored_language_is_selected_by_the_loader() {
    let Some(locale_dir) = built_locale_dir() else {
        eprintln!("skipping: no compiled locale dir");
        return;
    };

    for language in vendored_languages() {
        let _guard = env_guard();
        set_locale(&locale_dir, &language);
        meld_rs::i18n::init();
        assert_eq!(
            meld_rs::i18n::current_language().as_deref(),
            Some(language.as_str()),
            "locale {language} resolved to the wrong catalog"
        );
    }
}

#[test]
fn real_catalog_plural_rules_select_expected_forms() {
    let Some(locale_dir) = built_locale_dir() else {
        eprintln!("skipping: no compiled locale dir");
        return;
    };

    // Polish has nplurals=3 with three *distinct* forms, so the selected index
    // is observable from the returned text.
    let _guard = env_guard();
    set_locale(&locale_dir, "pl");
    meld_rs::i18n::init();
    assert_eq!(meld_rs::i18n::current_language().as_deref(), Some("pl"));

    let singular = "%d unpushed commit";
    let plural = "%d unpushed commits";
    let expectations = [
        (1i64, "%d niewysłane zatwierdzenie"),
        (2, "%d niewysłane zatwierdzenia"),
        (5, "%d niewysłanych zatwierdzeń"),
        (12, "%d niewysłanych zatwierdzeń"),
        (22, "%d niewysłane zatwierdzenia"),
    ];
    for (count, expected) in expectations {
        assert_eq!(
            meld_rs::i18n::ngettext(singular, plural, count).as_ref(),
            expected,
            "plural form for n={count}"
        );
    }
}

#[test]
fn translations_and_both_fallbacks_behave() {
    let Some(locale_dir) = built_locale_dir() else {
        eprintln!("skipping: no compiled locale dir");
        return;
    };

    // Exact German strings, including the gear-menu labels that were previously
    // unmarked.
    let _guard = env_guard();
    set_locale(&locale_dir, "de");
    meld_rs::i18n::init();
    assert_eq!(meld_rs::i18n::current_language().as_deref(), Some("de"));
    assert_eq!(meld_rs::i18n::gettext(CANARY_MSGID).as_ref(), CANARY_GERMAN);
    assert_eq!(
        meld_rs::i18n::gettext("Text Filters").as_ref(),
        "Textfilter"
    );
    assert_eq!(
        meld_rs::i18n::gettext("Save As…").as_ref(),
        "Speichern unter …"
    );
    assert_eq!(
        meld_rs::i18n::gettext("Save A_ll").as_ref(),
        "A_lles speichern"
    );
    assert_eq!(
        meld_rs::i18n::gettext("Version Control Console").as_ref(),
        "Versionsvergleichskonsole"
    );
    // Unknown msgid → English (msgid) fallback.
    assert_eq!(
        meld_rs::i18n::gettext("Not a real msgid").as_ref(),
        "Not a real msgid"
    );

    // Other locales translate a shared menu msgid.
    for language in ["fr", "zh_CN", "pt_BR", "ru"] {
        set_locale(&locale_dir, language);
        meld_rs::i18n::init();
        assert_eq!(meld_rs::i18n::current_language().as_deref(), Some(language));
        assert_ne!(
            meld_rs::i18n::gettext("Save A_ll").as_ref(),
            "Save A_ll",
            "{language} should translate menu labels"
        );
    }

    // Region-qualified locale falls back to its base language.
    set_locale(&locale_dir, "de_DE.UTF-8");
    meld_rs::i18n::init();
    assert_eq!(meld_rs::i18n::current_language().as_deref(), Some("de"));

    // Unknown locale → no catalog → English fallback.
    set_locale(&locale_dir, "xx_YY");
    meld_rs::i18n::init();
    assert_eq!(meld_rs::i18n::current_language(), None);
    assert_eq!(meld_rs::i18n::gettext(CANARY_MSGID).as_ref(), CANARY_MSGID);
    assert_eq!(meld_rs::i18n::ngettext("one", "many", 1).as_ref(), "one");
    assert_eq!(meld_rs::i18n::ngettext("one", "many", 2).as_ref(), "many");
}

#[test]
fn locale_dir_override_is_honoured() {
    let Some(real_dir) = built_locale_dir() else {
        eprintln!("skipping: no compiled locale dir");
        return;
    };
    let Ok(source) = std::fs::read(real_dir.join("de").join("LC_MESSAGES").join("meld-rs.mo"))
    else {
        eprintln!("skipping: German catalog unavailable");
        return;
    };

    // Stage the German catalog under a language name that does not exist in the
    // real locale dir, so only the override can supply it.
    let _guard = env_guard();
    let scratch = std::env::temp_dir().join(format!("meld-rs-i18n-{}", std::process::id()));
    let target_dir = scratch.join("zz").join("LC_MESSAGES");
    std::fs::create_dir_all(&target_dir).expect("create scratch locale dir");
    std::fs::write(target_dir.join("meld-rs.mo"), source).expect("write scratch catalog");

    set_locale(&scratch, "zz");
    meld_rs::i18n::init();
    assert_eq!(meld_rs::i18n::current_language().as_deref(), Some("zz"));
    assert_eq!(meld_rs::i18n::gettext(CANARY_MSGID).as_ref(), CANARY_GERMAN);

    let _ = std::fs::remove_dir_all(&scratch);
}

/// Text-setting APIs whose string argument must be translation-wrapped.
///
/// Each entry is `(regex, sample)`: the regex captures the literal passed
/// *directly* (i.e. without `tr!`/`format!`/a variable in between), and `sample`
/// is a synthetic bare call. `scanner_patterns_are_effective` uses the samples to
/// prove the regexes still match — a scanner that silently matched nothing would
/// otherwise pass forever.
///
/// Coverage boundary: the positional text argument of `MessageDialog::new` is
/// not matched here; every current call site passes either a variable or a `tr!`
/// expression.
const UNTRANSLATED_LITERAL_PATTERNS: &[(&str, &str)] = &[
    (r#"\.set_label\(\s*"([^"]*)""#, r#"x.set_label("Diff");"#),
    (
        r#"\.set_tooltip_text\(Some\(\s*"([^"]*)""#,
        r#"b.set_tooltip_text(Some("Tip"));"#,
    ),
    (
        r#"::with_label\(\s*"([^"]*)""#,
        r#"gtk::Button::with_label("Compare");"#,
    ),
    (
        r#"Label::new\(Some\(\s*"([^"]*)""#,
        r#"let l = gtk::Label::new(Some("Path"));"#,
    ),
    (
        r#"\.set_title\(Some\(\s*"([^"]*)""#,
        r#"d.set_title(Some("Push"));"#,
    ),
    (
        r#"\.set_placeholder_text\(Some\(\s*"([^"]*)""#,
        r#"e.set_placeholder_text(Some("Find"));"#,
    ),
    (
        r#"\.set_subtitle\(\s*"([^"]*)""#,
        r#"r.set_subtitle("Font");"#,
    ),
    (r#"\.set_text\(\s*"([^"]*)""#, r#"l.set_text("Hi");"#),
    (
        r#"\.set_markup\(\s*"([^"]*)""#,
        r#"l.set_markup("<b>x</b>");"#,
    ),
    (
        r#"\.set_secondary_text\(Some\(\s*"([^"]*)""#,
        r#"d.set_secondary_text(Some("More"));"#,
    ),
    (
        r#"\.set_button_label\(Some\(\s*"([^"]*)""#,
        r#"b.set_button_label(Some("Hide"));"#,
    ),
    (
        r#"\.append\(Some\(\s*"([^"]*)""#,
        r#"m.append(Some("Save"), Some("a"));"#,
    ),
    (
        r#"add_button\(\s*"([^"]*)""#,
        r#"d.add_button("_Cancel", t);"#,
    ),
    (
        r#"add_response\(\s*"[^"]*",\s*"([^"]*)""#,
        r#"d.add_response("cancel", "_Cancel");"#,
    ),
    (r#"\.title\(\s*"([^"]*)""#, r#"b.title("Choose")"#),
    (
        r#"\.accept_label\(\s*"([^"]*)""#,
        r#"b.accept_label("_Save")"#,
    ),
    (
        r#"banner\.set_title\(\s*"([^"]*)""#,
        r#"banner.set_title("Msg");"#,
    ),
    // MsgArea public API: callers own the text and must pass it already
    // translated (see `MsgArea::show_msg`).
    (r#"show_info\(\s*"([^"]*)""#, r#"a.show_info("Hi");"#),
    (r#"show_error\(\s*"([^"]*)""#, r#"a.show_error("Boom");"#),
    (
        r#"show_warning\(\s*"([^"]*)""#,
        r#"a.show_warning("Careful");"#,
    ),
    (
        r#"show_info_dismissable\(\s*"([^"]*)""#,
        r#"a.show_info_dismissable("Same");"#,
    ),
    (
        r#"show_warning_action\(\s*"([^"]*)""#,
        r#"a.show_warning_action("Changed", l, f);"#,
    ),
];

/// Comparisons of a *translated* value against a string literal are almost always
/// a bug: the literal stays English while the value is localized, so the branch
/// silently stops matching in every non-English locale. This exact pattern was a
/// real regression (the "New comparison" placeholder tab was never removed).
const TRANSLATED_COMPARISON_PATTERNS: &[(&str, &str)] = &[(
    r#"tr!\([^)]*\)\s*(?:==|!=)\s*""#,
    r#"if tr!("X") == "X" {}"#,
)];

/// Literals that are intentionally not translated (brand name / application
/// name, encodings, symbols, and empty strings).
const NON_TRANSLATABLE_LITERALS: &[&str] =
    &["", "Meld-rs", "_About Meld-rs", "UTF-8", "Aa", "W", ".*"];

/// Correctly wrapped calls: none of these may match any pattern above.
const WRAPPED_SAMPLES: &[&str] = &[
    r#"x.set_label(&tr!("Diff"));"#,
    r#"b.set_tooltip_text(Some(&tr!("Tip")));"#,
    r#"gtk::Button::with_label(&tr!("Compare"));"#,
    r#"let l = gtk::Label::new(Some(&tr!("Path")));"#,
    r#"d.set_title(Some(&tr!("Push")));"#,
    r#"e.set_placeholder_text(Some(&tr!("Find")));"#,
    r#"r.set_subtitle(&tr!("Font"));"#,
    r#"l.set_text(&tr!("Hi"));"#,
    r#"l.set_markup(&format!("<b>{}</b>", tr!("x")));"#,
    r#"d.set_secondary_text(Some(&tr!("More")));"#,
    r#"b.set_button_label(Some(&tr!("Hide")));"#,
    r#"m.append(Some(&tr!("Save")), Some("a"));"#,
    r#"d.add_button(&tr!("_Cancel"), t);"#,
    r#"d.add_response("cancel", &tr!("_Cancel"));"#,
    r#"b.title(title.as_ref())"#,
    r#"b.accept_label(tr!("_Save").as_ref())"#,
    r#"banner.set_title(&tr!("Msg"));"#,
    // Negative for the translated-comparison guard: this is the correct way to
    // answer "is this the placeholder?" and must not be flagged.
    r#"if page.is_new_comparison_placeholder() {}"#,
];

/// All `.rs` files under `src/`, recursively.
fn rust_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    let mut files = Vec::new();
    walk(&manifest_path("src"), &mut files);
    files.sort();
    assert!(!files.is_empty(), "no Rust sources found under src/");
    files
}

#[test]
fn scanner_patterns_are_effective() {
    for (pattern, sample) in UNTRANSLATED_LITERAL_PATTERNS {
        let regex = regex::Regex::new(pattern).expect("valid regex");
        let captures = regex
            .captures(sample)
            .unwrap_or_else(|| panic!("pattern {pattern} no longer matches {sample}"));
        let literal = captures.get(1).expect("capture group").as_str();
        assert!(
            !NON_TRANSLATABLE_LITERALS.contains(&literal),
            "sample {sample} must contain a bare, non-allowlisted literal"
        );
    }

    for sample in WRAPPED_SAMPLES {
        for (pattern, _) in UNTRANSLATED_LITERAL_PATTERNS {
            let regex = regex::Regex::new(pattern).expect("valid regex");
            assert!(
                regex.captures(sample).is_none(),
                "wrapped sample {sample} must not match pattern {pattern}"
            );
        }
    }
}

#[test]
fn no_unmarked_user_visible_literals() {
    let regexes: Vec<regex::Regex> = UNTRANSLATED_LITERAL_PATTERNS
        .iter()
        .map(|(pattern, _)| regex::Regex::new(pattern).expect("valid regex"))
        .collect();

    let mut offenders = Vec::new();
    for path in rust_sources() {
        let text = std::fs::read_to_string(&path).expect("read source file");
        for (index, line) in text.lines().enumerate() {
            for regex in &regexes {
                let Some(captures) = regex.captures(line) else {
                    continue;
                };
                let literal = captures.get(1).expect("capture group").as_str();
                if !NON_TRANSLATABLE_LITERALS.contains(&literal) {
                    offenders.push(format!(
                        "{}:{}: {:?}",
                        path.strip_prefix(MANIFEST_DIR).unwrap_or(&path).display(),
                        index + 1,
                        literal
                    ));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "user-visible literals are not wrapped in tr!:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_comparisons_on_translated_strings() {
    let regexes: Vec<regex::Regex> = TRANSLATED_COMPARISON_PATTERNS
        .iter()
        .map(|(pattern, sample)| {
            let regex = regex::Regex::new(pattern).expect("valid regex");
            assert!(
                regex.is_match(sample),
                "pattern {pattern} no longer matches its sample {sample}"
            );
            regex
        })
        .collect();

    let mut offenders = Vec::new();
    for path in rust_sources() {
        let text = std::fs::read_to_string(&path).expect("read source file");
        for (index, line) in text.lines().enumerate() {
            if regexes.iter().any(|regex| regex.is_match(line)) {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.strip_prefix(MANIFEST_DIR).unwrap_or(&path).display(),
                    index + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a translated value is compared against a literal, which stops matching \
         in non-English locales:\n{}",
        offenders.join("\n")
    );
}
