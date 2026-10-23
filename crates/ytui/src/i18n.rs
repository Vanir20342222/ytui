//! Translates interface labels before layout; user/provider content is untouched.
use std::{collections::BTreeMap, sync::OnceLock};

fn german() -> &'static BTreeMap<String, String> {
    static GERMAN: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    GERMAN.get_or_init(|| {
        toml::from_str(include_str!("locales/de.toml"))
            .expect("bundled German catalog must be valid")
    })
}
pub fn locale_is_german(locale: &str) -> bool {
    locale
        .split(['_', '-', '.', '@'])
        .next()
        .is_some_and(|s| s.eq_ignore_ascii_case("de"))
}
pub fn is_german(language: &str) -> bool {
    if language != "system" {
        return language == "de";
    }
    static SYSTEM_GERMAN: OnceLock<bool> = OnceLock::new();
    *SYSTEM_GERMAN.get_or_init(|| {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|s| !s.is_empty())
            .is_some_and(|s| locale_is_german(&s))
    })
}
pub fn text<'a>(language: &str, english: &'a str) -> &'a str {
    if is_german(language) {
        german().get(english).map_or(english, String::as_str)
    } else {
        english
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_and_locale_fallback_preserve_unknown_content() {
        assert!(german().len() > 100);
        assert!(locale_is_german("de_DE.UTF-8"));
        assert!(locale_is_german("de-AT"));
        assert!(!locale_is_german("default"));
        assert_eq!(text("de", "Settings"), "Einstellungen");
        assert_eq!(text("en", "Settings"), "Settings");
        assert_eq!(text("de", "My own song title"), "My own song title");
    }
    #[test]
    fn all_settings_action_titles_and_shortcut_descriptions_have_catalog_entries() {
        for field in crate::forms::Form::settings(ytui_core::Config::default()).fields {
            assert!(
                german().contains_key(field.label),
                "Missing setting: {}",
                field.label
            );
        }
        for entry in crate::actions::ENTRIES {
            assert!(
                german().contains_key(entry.title),
                "Missing action: {}",
                entry.title
            );
            assert!(
                german().contains_key(entry.group),
                "Missing group: {}",
                entry.group
            );
        }
        for (_, _, _, description) in ytui_core::shortcuts::REGISTRY {
            assert!(
                german().contains_key(*description),
                "Missing shortcut: {description}"
            );
        }
    }
}
