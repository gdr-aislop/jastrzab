//! Writing small state files to flash sparingly.
//!
//! Flash wears with writes and every write wakes storage, so files that change in bursts (the
//! settings while a preference is being edited) are written once after the burst, not once per
//! change. Each write goes to a temporary file and is renamed over the real one, so a crash or a
//! power cut leaves either the old file or the new one, never half of one. Files are created
//! readable by the user only: they hold a server address and, for the resume file, session IDs.

use std::cell::RefCell;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub struct DebouncedFile {
    path: PathBuf,
    delay_secs: u32,
    pending: Rc<RefCell<Option<String>>>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl DebouncedFile {
    pub fn new(path: PathBuf, delay_secs: u32) -> Self {
        Self { path, delay_secs, pending: Rc::new(RefCell::new(None)), timer: RefCell::new(None) }
    }

    /// Schedules `text` to be written after the quiet period. A newer call replaces it and
    /// restarts the wait.
    pub fn write(&self, text: String) {
        *self.pending.borrow_mut() = Some(text);
        self.cancel_timer();
        let pending = self.pending.clone();
        let path = self.path.clone();
        let timer = glib::timeout_add_seconds_local_once(self.delay_secs, move || {
            if let Some(text) = pending.borrow_mut().take()
                && let Err(err) = write_atomic(&path, &text)
            {
                tracing::warn!(%err, path = %path.display(), "couldn't save a state file");
            }
        });
        *self.timer.borrow_mut() = Some(timer);
    }

    /// Writes anything still waiting, now. Called when the app quits.
    pub fn flush_now(&self) {
        self.cancel_timer();
        if let Some(text) = self.pending.borrow_mut().take()
            && let Err(err) = write_atomic(&self.path, &text)
        {
            tracing::warn!(%err, path = %self.path.display(), "couldn't save a state file");
        }
    }

    /// Drops anything waiting and deletes the file.
    pub fn remove(&self) {
        self.cancel_timer();
        self.pending.borrow_mut().take();
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(%err, path = %self.path.display(), "couldn't delete a state file"),
        }
    }

    fn cancel_timer(&self) {
        if let Some(id) = self.timer.borrow_mut().take()
            && let Some(source) = glib::MainContext::default().find_source_by_id(&id)
        {
            source.destroy();
        }
    }
}

/// Writes `text` to `path` through a temporary file in the same directory and an atomic rename.
pub fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_data()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn atomic_writes_replace_the_file_and_leave_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("settings.toml");
        write_atomic(&path, "a = 1\n").unwrap();
        write_atomic(&path, "a = 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 2\n");
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 1, "no .tmp left behind");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn a_burst_of_writes_costs_one_file_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.toml");
        let file = DebouncedFile::new(path.clone(), 2);
        for n in 0..10 {
            file.write(format!("n = {n}\n"));
        }
        assert!(!path.exists(), "nothing is written until the burst is over");
        file.flush_now();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "n = 9\n");
        file.flush_now();
    }

    #[test]
    fn removing_cancels_a_pending_write_and_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.toml");
        write_atomic(&path, "x = 1\n").unwrap();
        let file = DebouncedFile::new(path.clone(), 2);
        file.write("x = 2\n".into());
        file.remove();
        file.flush_now();
        assert!(!path.exists());
        file.remove(); // already gone: fine
    }
}
