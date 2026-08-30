//! User-visible strings, keyed by dot notation and loaded from `locales/`.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::settings::Language;

static EN: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
static ES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct LocaleFile {
    strings: HashMap<String, String>,
}

fn load_embedded(json: &str) -> HashMap<&'static str, &'static str> {
    let file: LocaleFile = serde_json::from_str(json).expect("locale file must parse");
    file.strings
        .into_iter()
        .map(|(key, value)| {
            let key: &'static str = Box::leak(key.into_boxed_str());
            let value: &'static str = Box::leak(value.into_boxed_str());
            (key, value)
        })
        .collect()
}

fn english() -> &'static HashMap<&'static str, &'static str> {
    EN.get_or_init(|| load_embedded(include_str!("../locales/en.json")))
}

fn spanish() -> &'static HashMap<&'static str, &'static str> {
    ES.get_or_init(|| load_embedded(include_str!("../locales/es-ES.json")))
}

/// Resolved catalogue for one language choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Catalog {
    language: Language,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            language: Language::resolve(Language::System),
        }
    }
}

impl Catalog {
    pub fn new(language: Language) -> Self {
        Self {
            language: Language::resolve(language),
        }
    }

    pub fn language(self) -> Language {
        self.language
    }

    /// Look up a string. Missing keys fall back to English, then to the key.
    pub fn get(self, key: &'static str) -> &'static str {
        self.lookup(key).unwrap_or(key)
    }

    fn lookup(self, key: &str) -> Option<&'static str> {
        let table = match self.language {
            Language::SpanishSpain => spanish(),
            Language::English | Language::System => english(),
        };
        table.get(key).or_else(|| english().get(key)).copied()
    }

    /// Replace `{name}` placeholders in a translated string.
    pub fn format(self, key: &'static str, args: &[(&str, &str)]) -> String {
        let mut text = self.get(key).to_string();
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    /// Like [`Self::format`] but accepts a runtime key from the backend.
    pub fn format_dynamic(self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.lookup(key).unwrap_or(key).to_string();
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    /// The modifier key name for shortcut hints on this platform.
    pub fn mod_key(self) -> &'static str {
        if cfg!(target_os = "macos") {
            self.get("platform.mod_key.mac")
        } else {
            self.get("platform.mod_key.other")
        }
    }
}

/// Separator between a message key and `name=value` placeholder pairs.
const ARGS: char = '\x1e';

/// A user-visible message identified by an i18n key, optionally with placeholders.
pub fn keyed(key: &'static str) -> String {
    key.to_string()
}

/// A user-visible message key with `{name}` placeholders filled later in the UI.
pub fn keyed_args(key: &'static str, args: &[(&str, &str)]) -> String {
    let mut out = String::from(key);
    for (name, value) in args {
        out.push(ARGS);
        out.push_str(name);
        out.push('=');
        out.push_str(value);
    }
    out
}

/// Resolve a backend or engine message: plain keys, keyed placeholders, or passthrough.
pub fn translate(catalog: Catalog, message: &str) -> String {
    let (key, rest) = match message.split_once(ARGS) {
        Some(parts) => parts,
        None => {
            if looks_like_key(message) {
                if let Some(translated) = catalog.lookup(message) {
                    return translated.to_string();
                }
            }
            return message.to_string();
        }
    };
    let mut args = Vec::new();
    if !rest.is_empty() {
        for pair in rest.split(ARGS) {
            if let Some((name, value)) = pair.split_once('=') {
                args.push((name, value));
            }
        }
    }
    catalog.format_dynamic(key, &args)
}

fn looks_like_key(message: &str) -> bool {
    message.contains('.') && !message.contains(' ')
}

/// Tray menu labels in the user's language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayLabels {
    pub show_hide: String,
    pub play: String,
    pub pause: String,
    pub next: String,
    pub previous: String,
    pub quit: String,
    pub tooltip: String,
}

impl TrayLabels {
    pub fn from_catalog(catalog: Catalog) -> Self {
        Self {
            show_hide: catalog.get("tray.show_hide").to_string(),
            play: catalog.get("common.play").to_string(),
            pause: catalog.get("common.pause").to_string(),
            next: catalog.get("common.next").to_string(),
            previous: catalog.get("common.previous").to_string(),
            quit: catalog.get("common.quit").to_string(),
            tooltip: catalog.get("tray.tooltip").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_has_every_key_spanish_has() {
        let en_keys: std::collections::HashSet<_> = english().keys().copied().collect();
        for key in spanish().keys() {
            assert!(
                en_keys.contains(key),
                "es-ES is missing the English key {key}"
            );
        }
    }

    #[test]
    fn spanish_has_every_english_key() {
        for key in english().keys() {
            assert!(spanish().contains_key(key), "es-ES is missing {key}");
        }
    }

    #[test]
    fn placeholders_are_replaced() {
        let catalog = Catalog::new(Language::English);
        assert_eq!(
            catalog.format("settings.connected_as", &[("username", "alex")]),
            "Connected as alex"
        );
    }
}
