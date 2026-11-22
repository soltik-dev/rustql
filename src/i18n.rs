//! Translations of the interface.
//!
//! The texts live in `locales/<language>/rustql.ftl` (Fluent), embedded in the
//! binary at compile time. English is the reference: a text missing in another
//! language falls back to it. `t!("id")` and `t!("id", name = value, …)`
//! return a text in the current language, which the settings choose (or the
//! system's, by default). Worker threads use it too, for their error messages.
//!
//! Server errors are not translated: they arrive in the server's language.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{LanguageIdentifier, Loader, langid, static_loader};

static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "en",
        // Without the Unicode isolation marks around arguments: egui would
        // draw them as boxes
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

/// Languages of the interface: (code, name in that language).
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("pt-BR", "Português (Brasil)"),
];

/// Current language. English until `set_language` runs (and so in tests).
static CURRENT: LazyLock<RwLock<LanguageIdentifier>> = LazyLock::new(|| RwLock::new(langid!("en")));

/// Uses `code` (one of `LANGUAGES`) or, with `None`, the system language if
/// the interface is translated into it, and English otherwise.
pub fn set_language(code: Option<&str>) {
    let wanted = code.map(str::to_owned).or_else(sys_locale::get_locale);
    let lang = best_match(wanted.as_deref())
        .parse()
        .expect("LANGUAGES codes are valid");
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = lang;
}

/// The language of `LANGUAGES` for a locale such as "es-ES", "pt_PT.UTF-8"
/// or "fr": the same code, or else the same language (any Portuguese is
/// Brazilian Portuguese); English if there is none.
fn best_match(locale: Option<&str>) -> &'static str {
    let Some(locale) = locale else { return "en" };
    let locale = locale
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-");
    if let Some((code, _)) = LANGUAGES
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(&locale))
    {
        return code;
    }
    let language = locale.split('-').next().unwrap_or_default();
    LANGUAGES
        .iter()
        .find(|(c, _)| {
            c.split('-')
                .next()
                .is_some_and(|l| l.eq_ignore_ascii_case(language))
        })
        .map_or("en", |(c, _)| c)
}

fn current() -> LanguageIdentifier {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Text `id` in the current language (see `t!`).
pub fn tr(id: &str) -> String {
    LOCALES.lookup(&current(), id)
}

/// Text `id` with its arguments, in the current language (see `t!`).
pub fn tr_args(id: &str, args: &HashMap<Cow<'static, str>, FluentValue<'_>>) -> String {
    LOCALES.lookup_with_args(&current(), id, args)
}

/// What `t!` accepts as an argument: text or a number (numbers pick the
/// plural form in the translations).
pub trait Arg<'a> {
    fn into_arg(self) -> FluentValue<'a>;
}

impl<'a> Arg<'a> for &'a str {
    fn into_arg(self) -> FluentValue<'a> {
        self.into()
    }
}

impl<'a> Arg<'a> for &'a String {
    fn into_arg(self) -> FluentValue<'a> {
        self.as_str().into()
    }
}

impl<'a> Arg<'a> for String {
    fn into_arg(self) -> FluentValue<'a> {
        self.into()
    }
}

macro_rules! number_args {
    ($($t:ty),*) => {$(
        impl<'a> Arg<'a> for $t {
            fn into_arg(self) -> FluentValue<'a> {
                self.into()
            }
        }
    )*};
}
number_args!(i32, i64, u16, u32, u64, usize, f64);

/// Text in the current language: `t!("id")`, or with arguments,
/// `t!("rows-copied", count = n, table = name)`.
macro_rules! t {
    ($id:literal) => {
        $crate::i18n::tr($id)
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = ::std::collections::HashMap::new();
        $(args.insert(
            ::std::borrow::Cow::Borrowed(stringify!($name)),
            $crate::i18n::Arg::into_arg($value),
        );)+
        $crate::i18n::tr_args($id, &args)
    }};
}
pub(crate) use t;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    #[test]
    fn system_locales_map_to_a_translation() {
        assert_eq!(best_match(Some("es-ES")), "es");
        assert_eq!(best_match(Some("es_MX.UTF-8")), "es");
        assert_eq!(best_match(Some("pt-PT")), "pt-BR");
        assert_eq!(best_match(Some("pt_BR")), "pt-BR");
        assert_eq!(best_match(Some("fr-CA")), "fr");
        assert_eq!(best_match(Some("it")), "it");
        assert_eq!(best_match(Some("de-DE")), "en");
        assert_eq!(best_match(Some("C")), "en");
        assert_eq!(best_match(None), "en");
    }

    /// Ids defined in a Fluent file (messages, not their attributes).
    fn ftl_ids(path: &Path) -> BTreeSet<String> {
        std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .lines()
            .filter_map(|l| l.split_once(" =").map(|(id, _)| id))
            .filter(|id| !id.is_empty() && !id.starts_with([' ', '#', '.', '*', '[']))
            .map(str::to_owned)
            .collect()
    }

    /// Ids used with `t!` in the sources.
    fn used_ids() -> BTreeSet<String> {
        fn walk(dir: &Path, ids: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, ids);
                } else if path.extension().is_some_and(|e| e == "rs")
                    // This file only mentions t! in its docs and tests
                    && !path.ends_with("src/i18n.rs")
                {
                    let src = std::fs::read_to_string(&path).unwrap();
                    for (at, _) in src.match_indices("t!(") {
                        // `t!`, not the end of `format!` or `assert!`
                        let before = src[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        // The id may be on the next line
                        let Some(rest) = src[at + 3..].trim_start().strip_prefix('"') else {
                            continue;
                        };
                        if let Some((id, _)) = rest.split_once('"') {
                            ids.insert(id.to_owned());
                        }
                    }
                }
            }
        }
        let mut ids = BTreeSet::new();
        walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut ids);
        ids
    }

    /// Every text the code uses exists in every language, and no language
    /// has texts the code no longer uses.
    #[test]
    fn every_language_has_every_text() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let used = used_ids();
        assert!(!used.is_empty());
        for (code, _) in LANGUAGES {
            let defined = ftl_ids(&root.join(code).join("rustql.ftl"));
            let missing: Vec<_> = used.difference(&defined).collect();
            let unused: Vec<_> = defined.difference(&used).collect();
            assert!(missing.is_empty(), "{code}: missing {missing:?}");
            assert!(unused.is_empty(), "{code}: unused {unused:?}");
        }
    }

    /// Arguments (`$name`) of each message of a Fluent file.
    fn ftl_args(path: &Path) -> std::collections::BTreeMap<String, BTreeSet<String>> {
        let mut args = std::collections::BTreeMap::new();
        let mut current: Option<String> = None;
        for line in std::fs::read_to_string(path).unwrap().lines() {
            if let Some((id, _)) = line.split_once(" =")
                && !id.is_empty()
                && !id.starts_with([' ', '#'])
            {
                current = Some(id.to_owned());
            }
            let Some(id) = &current else { continue };
            let entry: &mut BTreeSet<String> = args.entry(id.clone()).or_default();
            for part in line.split('$').skip(1) {
                let name: String = part
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if !name.is_empty() {
                    entry.insert(name);
                }
            }
        }
        args
    }

    /// A translation uses the same arguments as English: a misspelt one
    /// shows as `{$name}` on screen and Fluent does not complain.
    #[test]
    fn every_translation_uses_the_same_arguments() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        let english = ftl_args(&root.join("en").join("rustql.ftl"));
        for (code, _) in &LANGUAGES[1..] {
            let translated = ftl_args(&root.join(code).join("rustql.ftl"));
            for (id, args) in &english {
                assert_eq!(translated.get(id), Some(args), "{code}: {id}");
            }
        }
    }

    /// The files parse cleanly: Fluent drops a message with a syntax error,
    /// and the English text would quietly show instead.
    #[test]
    fn every_file_parses() {
        use fluent_templates::fluent_bundle::FluentResource;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        for (code, _) in LANGUAGES {
            let src = std::fs::read_to_string(root.join(code).join("rustql.ftl")).unwrap();
            if let Err((_, errors)) = FluentResource::try_new(src) {
                panic!("{code}: {errors:?}");
            }
        }
    }
}
