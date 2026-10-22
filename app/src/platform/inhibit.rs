//! Keeping the phone from suspending while a share is active.
//!
//! Phosh takes logind "sleep" inhibitors itself and does not implement the session manager's
//! `Inhibit`, so this asks logind directly for a *block* lock. logind hands back a file
//! descriptor; the lock lasts exactly as long as that descriptor is open, so releasing is just
//! closing it, and a crash releases it too.

use std::cell::RefCell;
use std::os::fd::OwnedFd;
use std::rc::Rc;

use gio::prelude::*;

#[derive(Clone, Default)]
pub struct SleepInhibitor {
    lock: Rc<RefCell<Option<OwnedFd>>>,
    /// Bumped on each change so a slow reply cannot resurrect a lock that was released meanwhile.
    generation: Rc<std::cell::Cell<u64>>,
}

impl SleepInhibitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, on: bool) {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        if !on {
            self.lock.borrow_mut().take();
            return;
        }
        if self.lock.borrow().is_some() {
            return;
        }
        let this = self.clone();
        glib::spawn_future_local(async move {
            match acquire().await {
                Ok(fd) => {
                    if this.generation.get() == generation {
                        *this.lock.borrow_mut() = Some(fd);
                        tracing::debug!("holding a suspend inhibitor");
                    }
                }
                Err(err) => tracing::warn!(%err, "couldn't take a suspend inhibitor; the phone may sleep while sharing"),
            }
        });
    }
}

async fn acquire() -> Result<OwnedFd, glib::Error> {
    let conn = gio::bus_get_future(gio::BusType::System).await?;
    let args = ("sleep", "Jastrzab", "Sharing your location", "block").to_variant();
    let (reply, fds) = conn
        .call_with_unix_fd_list_future(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "Inhibit",
            Some(&args),
            None,
            gio::DBusCallFlags::NONE,
            10_000,
            None::<&gio::UnixFDList>,
        )
        .await?;
    let missing = || glib::Error::new(gio::IOErrorEnum::Failed, "logind returned no file descriptor");
    let (index,): (glib::variant::Handle,) = reply.get().ok_or_else(missing)?;
    fds.ok_or_else(missing)?.get(index.0)
}
