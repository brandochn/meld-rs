//! Minimal GNU gettext runtime for meld-rs.
//!
//! Mirrors the reference Meld's gettext setup (`bin/meld::setup_i18n()`), but
//! without depending on `libintl`: compiled `.mo` catalogs are parsed directly.
//! This keeps the lookup layer portable (including Windows/MSYS2), free of FFI,
//! and usable from `--no-default-features` builds and background diff threads.
//!
//! Catalogs are produced by `build.rs` from the vendored `po/*.po` files into
//! `<share>/locale/<lang>/LC_MESSAGES/meld-rs.mo`.  The active language is chosen
//! from the process locale (`LANGUAGE`, then `LC_ALL`, `LC_MESSAGES`, `LANG`),
//! matching gettext's precedence.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

/// Catalog domain, i.e. the `.mo` file name (`meld-rs.mo`).
pub const DOMAIN: &str = "meld-rs";

/// GNU MO file magic number.
const MO_MAGIC: u32 = 0x9504_12de;

// ────────────────────────────────────────────────────────────────────────────
// Plural-Forms expression evaluation
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Not,
    Question,
    Colon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Num(i64),
    N,
    LParen,
    RParen,
    Op(Op),
}

/// A parsed `Plural-Forms` expression plus the number of plural forms.
#[derive(Debug, Clone)]
struct PluralRule {
    nplurals: usize,
    tokens: Vec<Token>,
}

impl PluralRule {
    /// Parse the `Plural-Forms: nplurals=N; plural=EXPR;` header value.
    fn parse(header: &str) -> Option<Self> {
        let line = header
            .lines()
            .find(|l| l.trim_start().starts_with("Plural-Forms:"))?;

        let mut nplurals = None;
        let mut expr = None;
        for part in line["Plural-Forms:".len()..].split(';') {
            let part = part.trim();
            if let Some(v) = part.strip_prefix("nplurals=") {
                nplurals = v.trim().parse::<usize>().ok();
            } else if let Some(v) = part.strip_prefix("plural=") {
                expr = Some(v.trim().to_string());
            }
        }

        let tokens = lex(&expr?)?;
        Some(Self {
            nplurals: nplurals.unwrap_or(2).max(1),
            tokens,
        })
    }

    /// Select the plural form index for `n`.
    fn select(&self, n: i64) -> usize {
        let mut parser = ExprParser {
            tokens: &self.tokens,
            pos: 0,
        };
        let value = parser.ternary(n).unwrap_or(i64::from(n != 1));
        let index = if value < 0 { 0 } else { value as usize };
        index.min(self.nplurals - 1)
    }
}

/// Tokenise a `Plural-Forms` expression.  Only the constructs that occur in
/// real catalogs are supported; anything else makes parsing fail (and callers
/// fall back to the default Germanic rule).
fn lex(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token::Num(text.parse().ok()?));
            continue;
        }
        if c == 'n' {
            tokens.push(Token::N);
            i += 1;
            continue;
        }
        if c == '(' {
            tokens.push(Token::LParen);
            i += 1;
            continue;
        }
        if c == ')' {
            tokens.push(Token::RParen);
            i += 1;
            continue;
        }

        let (op, len) = match c {
            '|' if chars.get(i + 1) == Some(&'|') => (Op::Or, 2),
            '&' if chars.get(i + 1) == Some(&'&') => (Op::And, 2),
            '=' if chars.get(i + 1) == Some(&'=') => (Op::Eq, 2),
            '!' if chars.get(i + 1) == Some(&'=') => (Op::Ne, 2),
            '<' if chars.get(i + 1) == Some(&'=') => (Op::Le, 2),
            '>' if chars.get(i + 1) == Some(&'=') => (Op::Ge, 2),
            '<' => (Op::Lt, 1),
            '>' => (Op::Gt, 1),
            '+' => (Op::Add, 1),
            '-' => (Op::Sub, 1),
            '*' => (Op::Mul, 1),
            '/' => (Op::Div, 1),
            '%' => (Op::Mod, 1),
            '!' => (Op::Not, 1),
            '?' => (Op::Question, 1),
            ':' => (Op::Colon, 1),
            _ => return None,
        };
        tokens.push(Token::Op(op));
        i += len;
    }
    Some(tokens)
}

struct ExprParser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl ExprParser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat_op(&mut self, op: Op) -> bool {
        if self.peek() == Some(&Token::Op(op)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn ternary(&mut self, n: i64) -> Option<i64> {
        let cond = self.or(n)?;
        if self.eat_op(Op::Question) {
            let then = self.ternary(n)?;
            if !self.eat_op(Op::Colon) {
                return None;
            }
            let otherwise = self.ternary(n)?;
            Some(if cond != 0 { then } else { otherwise })
        } else {
            Some(cond)
        }
    }

    fn or(&mut self, n: i64) -> Option<i64> {
        let mut value = self.and(n)?;
        while self.eat_op(Op::Or) {
            let rhs = self.and(n)?;
            value = i64::from(value != 0 || rhs != 0);
        }
        Some(value)
    }

    fn and(&mut self, n: i64) -> Option<i64> {
        let mut value = self.equality(n)?;
        while self.eat_op(Op::And) {
            let rhs = self.equality(n)?;
            value = i64::from(value != 0 && rhs != 0);
        }
        Some(value)
    }

    fn equality(&mut self, n: i64) -> Option<i64> {
        let mut value = self.relational(n)?;
        loop {
            if self.eat_op(Op::Eq) {
                value = i64::from(value == self.relational(n)?);
            } else if self.eat_op(Op::Ne) {
                value = i64::from(value != self.relational(n)?);
            } else {
                return Some(value);
            }
        }
    }

    fn relational(&mut self, n: i64) -> Option<i64> {
        let mut value = self.additive(n)?;
        loop {
            if self.eat_op(Op::Lt) {
                value = i64::from(value < self.additive(n)?);
            } else if self.eat_op(Op::Gt) {
                value = i64::from(value > self.additive(n)?);
            } else if self.eat_op(Op::Le) {
                value = i64::from(value <= self.additive(n)?);
            } else if self.eat_op(Op::Ge) {
                value = i64::from(value >= self.additive(n)?);
            } else {
                return Some(value);
            }
        }
    }

    fn additive(&mut self, n: i64) -> Option<i64> {
        let mut value = self.multiplicative(n)?;
        loop {
            if self.eat_op(Op::Add) {
                value = value.wrapping_add(self.multiplicative(n)?);
            } else if self.eat_op(Op::Sub) {
                value = value.wrapping_sub(self.multiplicative(n)?);
            } else {
                return Some(value);
            }
        }
    }

    fn multiplicative(&mut self, n: i64) -> Option<i64> {
        let mut value = self.unary(n)?;
        loop {
            if self.eat_op(Op::Mul) {
                value = value.wrapping_mul(self.unary(n)?);
            } else if self.eat_op(Op::Div) {
                let rhs = self.unary(n)?;
                if rhs == 0 {
                    return None;
                }
                value /= rhs;
            } else if self.eat_op(Op::Mod) {
                let rhs = self.unary(n)?;
                if rhs == 0 {
                    return None;
                }
                value %= rhs;
            } else {
                return Some(value);
            }
        }
    }

    fn unary(&mut self, n: i64) -> Option<i64> {
        if self.eat_op(Op::Not) {
            Some(i64::from(self.unary(n)? == 0))
        } else if self.eat_op(Op::Sub) {
            Some(-self.unary(n)?)
        } else {
            self.primary(n)
        }
    }

    fn primary(&mut self, n: i64) -> Option<i64> {
        match self.peek()? {
            Token::Num(v) => {
                let v = *v;
                self.pos += 1;
                Some(v)
            }
            Token::N => {
                self.pos += 1;
                Some(n)
            }
            Token::LParen => {
                self.pos += 1;
                let value = self.ternary(n)?;
                if self.peek() != Some(&Token::RParen) {
                    return None;
                }
                self.pos += 1;
                Some(value)
            }
            _ => None,
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// MO catalog
// ────────────────────────────────────────────────────────────────────────────

/// A parsed GNU `.mo` catalog.
#[derive(Debug, Default)]
pub struct Catalog {
    /// `msgid` (plural entries join singular/plural with a NUL) → `msgstr`
    /// (plural forms separated by NUL).
    entries: HashMap<Vec<u8>, Vec<u8>>,
    plural: Option<PluralRule>,
}

impl Catalog {
    /// Look up a singular `msgid`.  Returns `None` when untranslated.
    pub fn gettext(&self, msgid: &str) -> Option<&str> {
        let raw = self.entries.get(msgid.as_bytes())?;
        let form = raw.split(|b| *b == 0).next().unwrap_or(&[]);
        str::from_utf8(form).ok().filter(|s| !s.is_empty())
    }

    /// Look up a plural `msgid` and select the form for `n`.
    pub fn ngettext(&self, singular: &str, plural: &str, n: i64) -> Option<&str> {
        let mut key = Vec::with_capacity(singular.len() + 1 + plural.len());
        key.extend_from_slice(singular.as_bytes());
        key.push(0);
        key.extend_from_slice(plural.as_bytes());

        let raw = self.entries.get(&key)?;
        let forms: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
        let index = match &self.plural {
            Some(rule) => rule.select(n),
            None => usize::from(n != 1),
        };
        let form = forms.get(index).or_else(|| forms.first())?;
        str::from_utf8(form).ok().filter(|s| !s.is_empty())
    }

    /// Number of translated entries in the catalog.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog holds no translated entries at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn read_u32(bytes: &[u8], offset: usize, little_endian: bool) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    let array: [u8; 4] = slice.try_into().ok()?;
    Some(if little_endian {
        u32::from_le_bytes(array)
    } else {
        u32::from_be_bytes(array)
    })
}

fn trim_nul(bytes: &[u8]) -> &[u8] {
    match bytes.split_last() {
        Some((0, rest)) => rest,
        _ => bytes,
    }
}

/// Parse a GNU `.mo` catalog.  Returns `None` if the magic or structure is
/// invalid; individual malformed entries are skipped.
pub fn parse_mo(bytes: &[u8]) -> Option<Catalog> {
    if bytes.len() < 28 {
        return None;
    }
    let as_le = u32::from_le_bytes(bytes[0..4].try_into().ok()?);
    let little_endian = if as_le == MO_MAGIC {
        true
    } else {
        let as_be = u32::from_be_bytes(bytes[0..4].try_into().ok()?);
        if as_be != MO_MAGIC {
            return None;
        }
        false
    };

    let count = read_u32(bytes, 8, little_endian)? as usize;
    let orig_table = read_u32(bytes, 12, little_endian)? as usize;
    let trans_table = read_u32(bytes, 16, little_endian)? as usize;

    let mut catalog = Catalog::default();
    for index in 0..count {
        let orig_len = read_u32(bytes, orig_table + index * 8, little_endian);
        let orig_off = read_u32(bytes, orig_table + index * 8 + 4, little_endian);
        let trans_len = read_u32(bytes, trans_table + index * 8, little_endian);
        let trans_off = read_u32(bytes, trans_table + index * 8 + 4, little_endian);
        let (Some(orig_len), Some(orig_off), Some(trans_len), Some(trans_off)) =
            (orig_len, orig_off, trans_len, trans_off)
        else {
            continue;
        };

        let Some(orig) =
            bytes.get(orig_off as usize..(orig_off as usize).saturating_add(orig_len as usize))
        else {
            continue;
        };
        let Some(trans) =
            bytes.get(trans_off as usize..(trans_off as usize).saturating_add(trans_len as usize))
        else {
            continue;
        };
        let orig = trim_nul(orig);
        let trans = trim_nul(trans);

        if orig.is_empty() {
            // Header entry: metadata, including the `Plural-Forms` rule.
            if let Ok(header) = str::from_utf8(trans) {
                catalog.plural = PluralRule::parse(header);
            }
            continue;
        }
        catalog.entries.insert(orig.to_vec(), trans.to_vec());
    }

    Some(catalog)
}

// ────────────────────────────────────────────────────────────────────────────
// Locale resolution
// ────────────────────────────────────────────────────────────────────────────

/// Expand a raw locale name into catalog-name candidates, most specific first.
///
/// `de_DE.UTF-8` → `["de_DE", "de"]`, `ca_ES@valencia` →
/// `["ca_ES@valencia", "ca_ES", "ca@valencia", "ca"]`.  `C`/`POSIX` yield no
/// candidates (no translation).
pub fn normalize_locale(raw: &str) -> Vec<String> {
    let (base, modifier) = match raw.split_once('@') {
        Some((base, modifier)) => (base, Some(modifier)),
        None => (raw, None),
    };
    let base = base.split('.').next().unwrap_or(base);
    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }

    let (language, territory) = match base.split_once('_') {
        Some((language, territory)) => (language.to_string(), Some(territory.to_string())),
        None => (base.to_string(), None),
    };

    let mut candidates: Vec<String> = Vec::new();
    let mut push = |value: String| {
        if !value.is_empty() && !candidates.contains(&value) {
            candidates.push(value);
        }
    };

    match (territory, modifier) {
        (Some(territory), Some(modifier)) => {
            push(format!("{language}_{territory}@{modifier}"));
            push(format!("{language}_{territory}"));
            push(format!("{language}@{modifier}"));
            push(language);
        }
        (Some(territory), None) => {
            push(format!("{language}_{territory}"));
            push(language);
        }
        (None, Some(modifier)) => {
            push(format!("{language}@{modifier}"));
            push(language);
        }
        (None, None) => push(language),
    }

    candidates
}

/// Catalog names to try for the current process locale, in priority order.
///
/// Environment variables are consulted first (`LANGUAGE`, then `LC_ALL`,
/// `LC_MESSAGES`, `LANG` — gettext precedence). When none of them is set, GLib's
/// language list is used: on Windows the environment is typically bare, but GLib
/// derives the language from the OS user locale, so translations still activate
/// (this mirrors what Meld does via `g_get_language_names`). GLib must be queried
/// after `setlocale`, which the caller does.
pub fn locale_candidates() -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    if let Ok(language) = std::env::var("LANGUAGE") {
        raw.extend(
            language
                .split(':')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        );
    }
    if raw.is_empty() {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(value) = std::env::var(var) {
                if !value.is_empty() {
                    raw.push(value);
                    break;
                }
            }
        }
    }
    if raw.is_empty() {
        raw = glib_language_names();
    }

    let mut candidates = Vec::new();
    for value in raw {
        for candidate in normalize_locale(&value) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

/// GLib's language list.
///
/// On Windows this is derived from the OS user locale (where `LANG`/`LC_*` are
/// usually unset). Only available with the `gui` feature, which pulls in `glib`;
/// without it the environment is the only source and this returns nothing.
#[cfg(feature = "gui")]
fn glib_language_names() -> Vec<String> {
    glib::language_names()
        .iter()
        .map(|name| name.to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[cfg(not(feature = "gui"))]
fn glib_language_names() -> Vec<String> {
    Vec::new()
}

/// Directories to search for catalogs, highest priority first.
pub fn locale_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(dir) = std::env::var("MELD_RS_LOCALEDIR") {
        if !dir.is_empty() {
            dirs.push(PathBuf::from(dir));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.join("share").join("locale"));
        }
    }
    #[cfg(unix)]
    dirs.push(PathBuf::from("/usr/share/locale"));
    dirs
}

// ────────────────────────────────────────────────────────────────────────────
// Global state + public API
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct State {
    catalog: Option<Catalog>,
    language: Option<String>,
}

static STATE: OnceLock<RwLock<State>> = OnceLock::new();

fn state() -> &'static RwLock<State> {
    STATE.get_or_init(|| RwLock::new(State::default()))
}

fn read_state() -> std::sync::RwLockReadGuard<'static, State> {
    state()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Load the best-matching catalog for the current locale.  Safe to call more
/// than once; missing catalogs leave the app in English fallback mode.
pub fn init() {
    let (catalog, language) = load_catalog();
    let mut state = state()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.language = language;
    state.catalog = catalog;
}

fn load_catalog() -> (Option<Catalog>, Option<String>) {
    let dirs = locale_dirs();
    for language in locale_candidates() {
        for dir in &dirs {
            let path = dir
                .join(&language)
                .join("LC_MESSAGES")
                .join(format!("{DOMAIN}.mo"));
            if let Ok(bytes) = std::fs::read(&path) {
                if let Some(catalog) = parse_mo(&bytes) {
                    return (Some(catalog), Some(language));
                }
            }
        }
    }
    (None, None)
}

/// The catalog name that was loaded, if any (e.g. `de`, `pt_BR`).
pub fn current_language() -> Option<String> {
    read_state().language.clone()
}

/// Translate a singular `msgid`, falling back to the `msgid` itself.
pub fn gettext<'a>(msgid: &'a str) -> Cow<'a, str> {
    let state = read_state();
    if let Some(translated) = state.catalog.as_ref().and_then(|c| c.gettext(msgid)) {
        return Cow::Owned(translated.to_string());
    }
    drop(state);
    Cow::Borrowed(msgid)
}

/// Translate a plural `msgid` for count `n`, falling back to the English
/// singular/plural selection.
///
/// Note: the application currently has no plural strings, so this is only
/// exercised by the unit tests. It is kept because the vendored catalogs do
/// contain plural entries (Meld uses them for VC counts such as
/// `"%d unpushed commit"`) and the `Plural-Forms` evaluator is subtle enough to
/// be worth keeping covered.
pub fn ngettext<'a>(singular: &'a str, plural: &'a str, n: i64) -> Cow<'a, str> {
    let state = read_state();
    if let Some(translated) = state
        .catalog
        .as_ref()
        .and_then(|c| c.ngettext(singular, plural, n))
    {
        return Cow::Owned(translated.to_string());
    }
    drop(state);
    Cow::Borrowed(if n == 1 { singular } else { plural })
}

/// Translate a singular `msgid` at the call site: `tr!("Diff Viewer")`.
#[macro_export]
macro_rules! tr {
    ($msgid:expr $(,)?) => {
        $crate::i18n::gettext($msgid)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an in-memory little-endian MO catalog for tests.
    fn build_mo(header: &str, entries: &[(&str, &str)]) -> Vec<u8> {
        let mut originals: Vec<Vec<u8>> = vec![Vec::new()];
        let mut translations: Vec<Vec<u8>> = vec![header.as_bytes().to_vec()];
        for (msgid, msgstr) in entries {
            originals.push(msgid.as_bytes().to_vec());
            translations.push(msgstr.as_bytes().to_vec());
        }

        let count = originals.len();
        let header_len = 28usize;
        let table_len = count * 8;
        let pool_start = header_len + table_len * 2;

        let mut pool = Vec::new();
        let mut orig_index = Vec::new();
        let mut trans_index = Vec::new();
        for value in &originals {
            orig_index.push((value.len(), pool_start + pool.len()));
            pool.extend_from_slice(value);
            pool.push(0);
        }
        for value in &translations {
            trans_index.push((value.len(), pool_start + pool.len()));
            pool.extend_from_slice(value);
            pool.push(0);
        }

        let mut out = Vec::new();
        out.extend_from_slice(&MO_MAGIC.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&(header_len as u32).to_le_bytes());
        out.extend_from_slice(&((header_len + table_len) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for (len, off) in &orig_index {
            out.extend_from_slice(&(*len as u32).to_le_bytes());
            out.extend_from_slice(&(*off as u32).to_le_bytes());
        }
        for (len, off) in &trans_index {
            out.extend_from_slice(&(*len as u32).to_le_bytes());
            out.extend_from_slice(&(*off as u32).to_le_bytes());
        }
        out.extend_from_slice(&pool);
        out
    }

    #[test]
    fn parses_and_translates_singular_and_plural() {
        let header = "Project-Id-Version: test\nPlural-Forms: nplurals=2; plural=(n != 1);\n";
        let bytes = build_mo(
            header,
            &[
                ("Diff Viewer", "Diff-Anzeige"),
                ("one file\0%d files", "%d Datei\0%d Dateien"),
                ("Untranslated", ""),
            ],
        );
        let catalog = parse_mo(&bytes).expect("valid MO");

        assert_eq!(catalog.gettext("Diff Viewer"), Some("Diff-Anzeige"));
        assert_eq!(catalog.gettext("Missing"), None);
        // Empty msgstr is treated as untranslated.
        assert_eq!(catalog.gettext("Untranslated"), None);
        assert_eq!(
            catalog.ngettext("one file", "%d files", 1),
            Some("%d Datei")
        );
        assert_eq!(
            catalog.ngettext("one file", "%d files", 5),
            Some("%d Dateien")
        );
        assert_eq!(
            catalog.ngettext("one file", "%d files", 0),
            Some("%d Dateien")
        );
    }

    #[test]
    fn rejects_invalid_magic() {
        assert!(parse_mo(&[0u8; 32]).is_none());
        assert!(parse_mo(&[]).is_none());
    }

    #[test]
    fn normalizes_locale_names() {
        assert_eq!(normalize_locale("de_DE.UTF-8"), vec!["de_DE", "de"]);
        assert_eq!(normalize_locale("de"), vec!["de"]);
        assert_eq!(
            normalize_locale("ca_ES@valencia"),
            vec!["ca_ES@valencia", "ca_ES", "ca@valencia", "ca"]
        );
        assert_eq!(
            normalize_locale("sr_RS@latin"),
            vec!["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
        assert!(normalize_locale("C").is_empty());
        assert!(normalize_locale("POSIX").is_empty());
        assert!(normalize_locale("").is_empty());
    }

    #[test]
    fn selects_plural_forms() {
        let german = PluralRule::parse("Plural-Forms: nplurals=2; plural=(n != 1);").unwrap();
        assert_eq!(german.select(1), 0);
        assert_eq!(german.select(2), 1);

        let russian = PluralRule::parse(
            "Plural-Forms: nplurals=3; \
             plural=(n%10==1 && n%100!=11 ? 0 : \
             n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
        )
        .unwrap();
        assert_eq!(russian.select(1), 0);
        assert_eq!(russian.select(2), 1);
        assert_eq!(russian.select(5), 2);
        assert_eq!(russian.select(11), 2);
        assert_eq!(russian.select(21), 0);

        let japanese = PluralRule::parse("Plural-Forms: nplurals=1; plural=0;").unwrap();
        assert_eq!(japanese.select(0), 0);
        assert_eq!(japanese.select(9), 0);
    }

    #[test]
    fn falls_back_to_msgid_without_catalog() {
        // No `init()` in tests, so the global state holds no catalog.
        assert_eq!(gettext("Hello"), "Hello");
        assert_eq!(ngettext("one", "many", 1), "one");
        assert_eq!(ngettext("one", "many", 2), "many");
        assert!(current_language().is_none());
    }
}
