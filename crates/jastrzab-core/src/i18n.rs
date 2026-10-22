//! Translation hook. The core produces user-visible text (the GPS signal wording, status lines)
//! so that it can be tested without a UI; it stays free of any toolkit by calling [`tr`], which
//! the app points at gettext with [`set_translator`]. Tests and a fresh process get the English
//! text unchanged.

use std::sync::OnceLock;

static TRANSLATOR: OnceLock<fn(&str) -> String> = OnceLock::new();

/// Installs the translator. Only the first call has an effect.
pub fn set_translator(translator: fn(&str) -> String) {
    let _ = TRANSLATOR.set(translator);
}

/// Translates a message. The strings passed here are the gettext message IDs.
pub fn tr(message: &str) -> String {
    match TRANSLATOR.get() {
        Some(translator) => translator(message),
        None => message.to_string(),
    }
}

/// Translates `message` and replaces each `{name}` placeholder with its value.
pub fn tr_args(message: &str, args: &[(&str, &str)]) -> String {
    let mut text = tr(message);
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untranslated_text_passes_through_with_placeholders_filled() {
        assert_eq!(tr("Good"), "Good");
        assert_eq!(tr_args("About {m} m", &[("m", "50")]), "About 50 m");
    }
}
