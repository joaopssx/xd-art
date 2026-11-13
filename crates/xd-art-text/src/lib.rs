//! Interface text in English and Portuguese, for the desktop app and the command line
//! tool. No translation library: the English text is the key, and [`tr`] looks it up
//! in one sorted table per language.
//!
//! ```
//! use xd_art_text::{Lang, fill, set, tr};
//!
//! set(Lang::Pt);
//! assert_eq!(tr("undo"), "desfazer");
//! assert_eq!(fill(tr("{0} colors"), &[&16]), "16 cores");
//! set(Lang::En);
//! assert_eq!(tr("undo"), "undo");
//! ```
//!
//! Text that is not in the table comes back unchanged, so a missing translation shows
//! English instead of breaking. A test in this crate reads the sources of the app and
//! the command line tool and fails when a string passed to `tr` has no translation.
//! `CONTRIBUTING.md` explains how to add a language.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

mod pt;

/// A language of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    /// English, the language of the sources.
    En,
    /// Brazilian Portuguese.
    Pt,
}

impl Lang {
    /// Every language, in menu order.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::Pt];

    /// The language's own name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Pt => "Português",
        }
    }

    /// Short code: `en` or `pt`.
    #[must_use]
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Pt => "pt",
        }
    }

    /// The language for a code or locale name like `pt`, `pt_BR.UTF-8` or `pt-BR`.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Lang> {
        let lower = code.trim().to_ascii_lowercase();
        let base = lower.split(['_', '-', '.', '@']).next().unwrap_or("");
        match base {
            "en" | "c" | "posix" => Some(Lang::En),
            "pt" => Some(Lang::Pt),
            _ => None,
        }
    }

    fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Lang::En => &[],
            Lang::Pt => pt::TABLE,
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

/// Sets the language for every later [`tr`] call.
pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

/// The language in use.
#[must_use]
pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Lang::Pt,
        _ => Lang::En,
    }
}

/// `text` in the current language, or `text` itself when there is no translation.
#[must_use]
pub fn tr(text: &str) -> &str {
    let table = current().table();
    match table.binary_search_by(|(k, _)| (*k).cmp(text)) {
        Ok(i) => table[i].1,
        Err(_) => text,
    }
}

/// Marks text that is translated later, where it is shown: a label kept in a table,
/// for example. It returns `text` unchanged; the completeness test looks for it.
#[must_use]
pub const fn mark(text: &'static str) -> &'static str {
    text
}

/// Replaces `{0}`, `{1}`, ... in `template` with `args`. Translations can put the
/// values in another order than English does.
#[must_use]
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after
            .find('}')
            .and_then(|close| after[..close].parse::<usize>().ok().map(|i| (i, close)))
        {
            Some((i, close)) => {
                if let Some(a) = args.get(i) {
                    out.push_str(&a.to_string());
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The language of the system: `XD_ART_LANG` first, then the usual locale variables,
/// then (on Windows) the user's locale setting. English when nothing matches.
#[must_use]
pub fn detect() -> Lang {
    for var in ["XD_ART_LANG", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(l) = std::env::var(var)
            .ok()
            .filter(|v| !v.is_empty())
            .and_then(|v| Lang::from_code(&v))
        {
            return l;
        }
    }
    system_locale()
        .and_then(|s| Lang::from_code(&s))
        .unwrap_or(Lang::En)
}

#[cfg(windows)]
fn system_locale() -> Option<String> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetUserDefaultLocaleName(name: *mut u16, len: i32) -> i32;
    }
    let mut buf = [0u16; 85];
    // SAFETY: the buffer is valid for writes of `buf.len()` UTF-16 units, which is the
    // length passed; the function writes at most that many and returns how many.
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    let n = usize::try_from(n).ok().filter(|&n| n > 1)?;
    String::from_utf16(&buf[..n - 1]).ok()
}

#[cfg(not(windows))]
fn system_locale() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_and_unique() {
        for lang in Lang::ALL {
            for w in lang.table().windows(2) {
                assert!(
                    w[0].0 < w[1].0,
                    "{:?}: {:?} before {:?}",
                    lang,
                    w[0].0,
                    w[1].0
                );
            }
        }
    }

    #[test]
    fn placeholders_match() {
        let count = |s: &str| (0..9).filter(|i| s.contains(&format!("{{{i}}}"))).count();
        for (k, v) in pt::TABLE {
            assert_eq!(count(k), count(v), "{k:?} -> {v:?}");
            assert!(!v.is_empty(), "{k:?}");
        }
    }

    #[test]
    fn fill_puts_values_in_place() {
        assert_eq!(fill("{1} of {0}", &[&"a", &2]), "2 of a");
        assert_eq!(fill("no braces", &[]), "no braces");
        assert_eq!(fill("{x} stays, {0}", &[&1]), "{x} stays, 1");
        assert_eq!(fill("{5} missing", &[&1]), " missing");
    }

    #[test]
    fn codes() {
        assert_eq!(Lang::from_code("pt_BR.UTF-8"), Some(Lang::Pt));
        assert_eq!(Lang::from_code("pt-PT"), Some(Lang::Pt));
        assert_eq!(Lang::from_code("en_US"), Some(Lang::En));
        assert_eq!(Lang::from_code("C"), Some(Lang::En));
        assert_eq!(Lang::from_code("de_DE"), None);
    }

    /// Every literal passed to `tr(` or `mark(` in the app and the command line tool
    /// has a Portuguese entry.
    #[test]
    fn every_used_string_is_translated() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut missing = Vec::new();
        for dir in ["xd-art-gui/src", "xd-art-cli/src"] {
            let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
                continue;
            };
            for e in entries.flatten() {
                let text = std::fs::read_to_string(e.path()).unwrap_or_default();
                for call in ["tr(\"", "mark(\""] {
                    let mut rest = text.as_str();
                    while let Some(i) = rest.find(call) {
                        // Skip names that only end the same way, like `attr(` or `.mark(`.
                        let before = rest[..i].chars().last();
                        rest = &rest[i + call.len()..];
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
                            continue;
                        }
                        let Some(end) = rest.find('"') else { break };
                        let key = rest[..end].to_string();
                        if pt::TABLE.binary_search_by(|(k, _)| (*k).cmp(&key)).is_err() {
                            missing.push(key);
                        }
                        rest = &rest[end..];
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "no Portuguese for: {missing:#?}");
    }
}
