//! Launching programs on the host from inside an AppImage.
//!
//! The AppImage's `AppRun` hook points `LD_LIBRARY_PATH`, `GIO_MODULE_DIR`, `XDG_DATA_DIRS` and
//! friends at the bundle. A program the app starts (the file manager behind *Open Log Folder*) would
//! inherit them, load the bundle's libraries instead of its own and die with symbol-lookup errors.
//! The hook therefore records the host's values first (`JASTRZAB_HOST_<VAR>`, absent when the variable
//! was unset) and lists the variables it changed in `JASTRZAB_BUNDLE_ENV`; this puts them back for
//! launched programs. Outside an AppImage `JASTRZAB_BUNDLE_ENV` is unset and nothing changes.

use gio::prelude::*;

const BUNDLE_ENV: &str = "JASTRZAB_BUNDLE_ENV";
const HOST_PREFIX: &str = "JASTRZAB_HOST_";

/// What to do to the environment of a launched program: `Some(value)` sets a variable to what the
/// host had, `None` removes it because the host had none.
pub fn host_env_changes(env: impl Fn(&str) -> Option<String>) -> Vec<(String, Option<String>)> {
    let Some(list) = env(BUNDLE_ENV) else { return Vec::new() };
    list.split_whitespace().map(|var| (var.to_string(), env(&format!("{HOST_PREFIX}{var}")))).collect()
}

/// A launch context that restores the host's environment (an ordinary one outside an AppImage).
pub fn launch_context() -> gio::AppLaunchContext {
    let context = gio::AppLaunchContext::new();
    for (var, value) in host_env_changes(|name| std::env::var(name).ok()) {
        match value {
            Some(value) => context.setenv(&var, &value),
            None => context.unsetenv(&var),
        }
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn nothing_changes_outside_an_appimage() {
        assert!(host_env_changes(lookup(&[("LD_LIBRARY_PATH", "/x")])).is_empty());
    }

    #[test]
    fn the_hosts_values_come_back_and_variables_the_host_lacked_are_removed() {
        let env = lookup(&[
            ("JASTRZAB_BUNDLE_ENV", "LD_LIBRARY_PATH GIO_MODULE_DIR XDG_DATA_DIRS"),
            ("JASTRZAB_HOST_LD_LIBRARY_PATH", "/host/lib"),
            ("JASTRZAB_HOST_XDG_DATA_DIRS", "/usr/share"),
            ("LD_LIBRARY_PATH", "/appdir/usr/lib:/host/lib"),
        ]);
        assert_eq!(
            host_env_changes(env),
            [
                ("LD_LIBRARY_PATH".to_string(), Some("/host/lib".to_string())),
                ("GIO_MODULE_DIR".to_string(), None),
                ("XDG_DATA_DIRS".to_string(), Some("/usr/share".to_string())),
            ]
        );
    }

    #[test]
    fn an_empty_host_value_is_kept_as_empty_not_removed() {
        let env = lookup(&[("JASTRZAB_BUNDLE_ENV", "GTK_PATH"), ("JASTRZAB_HOST_GTK_PATH", "")]);
        assert_eq!(host_env_changes(env), [("GTK_PATH".to_string(), Some(String::new()))]);
    }
}
