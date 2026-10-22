//! Resolves the on-disk locations this app uses, following the XDG Base Directory layout and the
//! same scheme as the author's other Phosh app (audiobooklet): configuration under
//! `$XDG_CONFIG_HOME`, and operational state (logs, crash dumps, the resume file — not user data,
//! not safe to evict like a cache) under `$XDG_STATE_HOME`. Nothing outside this module computes
//! one of these paths directly.

use std::path::{Path, PathBuf};

use directories::BaseDirs;

/// The app's reverse-DNS identity: also the GTK application id, the Flatpak id, and the directory
/// name below, used verbatim rather than through `directories::ProjectDirs`, which on Linux
/// ignores the qualifier and organization and lowercases the application name.
///
/// The GitHub owner segment uses an underscore rather than its real hyphen (`gdr-aislop`) because
/// `flatpak-builder` rejects a hyphen in any but the last dotted segment.
pub const APP_ID: &str = "io.github.gdr_aislop.jastrzab";

/// File-name prefix of the rolling log files in [`AppPaths::logs_dir`]. The `app` crate's logger
/// writes with it and [`AppPaths::latest_log_file`] looks for it, so the two can't drift.
pub const LOG_FILE_PREFIX: &str = "jastrzab";

#[derive(Debug, Clone)]
pub struct AppPaths {
    config_dir: PathBuf,
    state_dir: PathBuf,
}

impl AppPaths {
    /// Resolve paths from the real XDG environment. Returns `None` if no home directory can be
    /// determined at all.
    pub fn resolve() -> Option<Self> {
        let dirs = BaseDirs::new()?;
        Some(Self {
            config_dir: dirs.config_dir().join(APP_ID),
            // `state_dir()` is `None` on platforms with no `$XDG_STATE_HOME` equivalent: fall
            // back to the data directory (durable-adjacent) rather than a cache (evictable).
            state_dir: dirs.state_dir().unwrap_or_else(|| dirs.data_dir()).join(APP_ID),
        })
    }

    /// An instance rooted at arbitrary directories, for tests.
    pub fn rooted_at(config_dir: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self { config_dir: config_dir.into(), state_dir: state_dir.into() }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// The user's settings (no secrets: those live in the keyring).
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.toml")
    }

    /// The unfinished session, kept so a killed app can offer to resume it.
    pub fn session_file(&self) -> PathBuf {
        self.state_dir.join("session.toml")
    }

    /// Rotating application logs: see `crash_reporting::init_logging` in the `app` crate.
    pub fn logs_dir(&self) -> PathBuf {
        self.state_dir.join("logs")
    }

    /// The newest log file in [`logs_dir`](Self::logs_dir), or `None` if there isn't one. Chosen by
    /// file name, not modification time: the rolling appender suffixes each file with its date
    /// (`jastrzab.2026-10-09`), which sorts chronologically and can't be skewed by a file being
    /// touched or restored from a backup.
    pub fn latest_log_file(&self) -> std::io::Result<Option<PathBuf>> {
        let entries = match std::fs::read_dir(self.logs_dir()) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let mut newest: Option<(std::ffi::OsString, PathBuf)> = None;
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let is_log = name
                .to_str()
                .is_some_and(|n| n == LOG_FILE_PREFIX || n.strip_prefix(LOG_FILE_PREFIX).is_some_and(|rest| rest.starts_with('.')));
            if is_log && newest.as_ref().is_none_or(|(best, _)| name > *best) {
                newest = Some((name, entry.path()));
            }
        }
        Ok(newest.map(|(_, path)| path))
    }

    /// Local Breakpad-format `.dmp` files written on a native (signal-level) crash: see
    /// `crash_reporting::attach_crash_handler` in the `app` crate.
    pub fn crash_dumps_dir(&self) -> PathBuf {
        self.state_dir.join("crashes")
    }

    /// Every directory this `AppPaths` might write into.
    fn all_dirs(&self) -> [PathBuf; 4] {
        [self.config_dir.clone(), self.state_dir.clone(), self.logs_dir(), self.crash_dumps_dir()]
    }

    /// Create every directory this `AppPaths` might write into. Idempotent.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in self.all_dirs() {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// The `logs_dir`/`crash_dumps_dir`-only subset of [`ensure_dirs`](Self::ensure_dirs), for
    /// `main` to call before anything else: logging and crash-dump capture are wired up first so
    /// they can catch failures during the rest of start-up.
    pub fn ensure_early_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.logs_dir())?;
        std::fs::create_dir_all(self.crash_dumps_dir())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> (tempfile::TempDir, AppPaths) {
        let tmp = tempfile::tempdir().expect("create temp dir");
        let paths = AppPaths::rooted_at(tmp.path().join("config"), tmp.path().join("state"));
        (tmp, paths)
    }

    #[test]
    fn files_live_where_documented() {
        let (_tmp, paths) = test_paths();
        assert_eq!(paths.settings_file(), paths.config_dir().join("settings.toml"));
        assert_eq!(paths.session_file(), paths.state_dir().join("session.toml"));
        assert_eq!(paths.logs_dir(), paths.state_dir().join("logs"));
        assert_eq!(paths.crash_dumps_dir(), paths.state_dir().join("crashes"));
        assert_ne!(paths.logs_dir(), paths.crash_dumps_dir());
    }

    #[test]
    fn latest_log_file_is_none_without_a_logs_dir_or_logs() {
        let (_tmp, paths) = test_paths();
        assert_eq!(paths.latest_log_file().unwrap(), None, "no directory yet");
        std::fs::create_dir_all(paths.logs_dir()).unwrap();
        assert_eq!(paths.latest_log_file().unwrap(), None, "empty directory");
    }

    #[test]
    fn latest_log_file_picks_the_newest_date_even_when_mtimes_disagree() {
        let (_tmp, paths) = test_paths();
        let dir = paths.logs_dir();
        std::fs::create_dir_all(&dir).unwrap();
        // Written newest-date-first, so the newest *date* has the oldest mtime.
        for day in ["2026-10-09", "2026-10-07", "2026-10-08"] {
            std::fs::write(dir.join(format!("jastrzab.{day}")), day).unwrap();
        }
        assert_eq!(paths.latest_log_file().unwrap(), Some(dir.join("jastrzab.2026-10-09")));
    }

    #[test]
    fn latest_log_file_ignores_other_files_and_directories() {
        let (_tmp, paths) = test_paths();
        let dir = paths.logs_dir();
        std::fs::create_dir_all(dir.join("jastrzab.2099-01-01")).unwrap(); // a directory
        std::fs::write(dir.join("other.log"), "").unwrap();
        std::fs::write(dir.join("jastrzab-old.txt"), "").unwrap();
        std::fs::create_dir_all(paths.crash_dumps_dir()).unwrap();
        std::fs::write(paths.crash_dumps_dir().join("jastrzab.2099-01-01"), "").unwrap();
        assert_eq!(paths.latest_log_file().unwrap(), None);

        std::fs::write(dir.join("jastrzab"), "undated").unwrap();
        assert_eq!(paths.latest_log_file().unwrap(), Some(dir.join("jastrzab")), "the undated name is still a log");
    }

    #[test]
    fn ensure_early_dirs_creates_only_logs_and_crash_dumps() {
        let (_tmp, paths) = test_paths();
        paths.ensure_early_dirs().unwrap();
        assert!(paths.logs_dir().is_dir() && paths.crash_dumps_dir().is_dir());
        assert!(!paths.config_dir().exists());
    }

    #[test]
    fn ensure_dirs_is_idempotent_and_creates_everything() {
        let (_tmp, paths) = test_paths();
        paths.ensure_dirs().unwrap();
        paths.ensure_dirs().unwrap();
        assert!(paths.config_dir().is_dir() && paths.state_dir().is_dir());
    }

    #[test]
    fn resolve_uses_the_literal_app_id_as_the_directory_name() {
        let Some(paths) = AppPaths::resolve() else { return };
        assert_eq!(paths.config_dir().file_name().and_then(|n| n.to_str()), Some(APP_ID));
        assert_eq!(paths.state_dir().file_name().and_then(|n| n.to_str()), Some(APP_ID));
        assert_eq!(paths.logs_dir().parent(), Some(paths.state_dir()));
    }
}
