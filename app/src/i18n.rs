//! gettext set-up. The core's text goes through the same translator (`jastrzab_core::i18n`), so
//! the whole app is translated by one catalogue. No translations ship yet: message IDs are the
//! English text, which is what gettext returns for a missing translation.

use gettext_rs::{LocaleCategory, bindtextdomain, gettext, setlocale, textdomain};

const DOMAIN: &str = "jastrzab";
const DEFAULT_LOCALE_DIR: &str = "/usr/share/locale";

/// Takes the locale from the environment. `setlocale` is not thread-safe, so this runs first thing
/// in `main`, before logging or GTK start any thread.
pub fn set_locale() {
    // SAFETY: called once at the start of `main`, while the process is single-threaded.
    unsafe { setlocale(LocaleCategory::LcAll, "") };
}

pub fn init() {
    let dir = std::env::var_os("JASTRZAB_LOCALEDIR").map_or_else(|| DEFAULT_LOCALE_DIR.into(), std::path::PathBuf::from);
    if let Err(err) = bindtextdomain(DOMAIN, dir) {
        tracing::warn!(%err, "couldn't bind the translation catalogue");
    }
    let _ = textdomain(DOMAIN);
    jastrzab_core::i18n::set_translator(|message| gettext(message));
}
