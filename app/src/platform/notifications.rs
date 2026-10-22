//! The notification shown while a share is active, sent straight to
//! `org.freedesktop.Notifications`.
//!
//! `GNotification` cannot set the `resident` hint, and without it Phosh closes the notification as
//! soon as one of its actions is tapped. So the app talks to the notification server itself, and
//! re-sends with `replaces_id` to change the title. Phosh offers the actions in the notification
//! and, on the lock screen, only those named in the desktop file's `X-Phosh-Lockscreen-Actions`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use async_channel::Sender;
use gio::prelude::*;
use jastrzab_core::engine::Event;
use jastrzab_core::i18n::tr;
use jastrzab_core::paths::APP_ID;
use jastrzab_core::view::NotificationView;

pub const ACTION_COPY_LINK: &str = "copy-link";
pub const ACTION_STOP: &str = "stop-sharing";
const DEFAULT_ACTION: &str = "default";

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT_PATH: &str = "/org/freedesktop/Notifications";

#[derive(Clone)]
pub struct Notifier {
    inner: Rc<Inner>,
}

struct Inner {
    events: Sender<Event>,
    /// Called when the notification itself is tapped: bring the app to the front.
    activate: Box<dyn Fn()>,
    conn: RefCell<Option<gio::DBusConnection>>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
    id: Cell<u32>,
    /// Orders sends so a late reply cannot overwrite a newer notification's id.
    sending: Cell<bool>,
    wanted: RefCell<Option<NotificationView>>,
}

impl Notifier {
    pub fn new(events: Sender<Event>, activate: impl Fn() + 'static) -> Self {
        Self {
            inner: Rc::new(Inner {
                events,
                activate: Box::new(activate),
                conn: RefCell::new(None),
                subscriptions: RefCell::new(Vec::new()),
                id: Cell::new(0),
                sending: Cell::new(false),
                wanted: RefCell::new(None),
            }),
        }
    }

    /// Shows the notification, or replaces the one already shown; `None` withdraws it.
    pub fn set(&self, view: Option<NotificationView>) {
        *self.inner.wanted.borrow_mut() = view;
        self.sync();
    }

    fn sync(&self) {
        if self.inner.sending.get() {
            return; // the running send re-checks `wanted` when it finishes
        }
        self.inner.sending.set(true);
        let this = self.clone();
        glib::spawn_future_local(async move {
            loop {
                let wanted = this.inner.wanted.borrow().clone();
                match this.apply(wanted.as_ref()).await {
                    Ok(()) => {}
                    Err(err) => tracing::warn!(%err, "couldn't update the notification"),
                }
                if *this.inner.wanted.borrow() == wanted {
                    break;
                }
            }
            this.inner.sending.set(false);
        });
    }

    async fn apply(&self, view: Option<&NotificationView>) -> Result<(), glib::Error> {
        let conn = self.connection().await?;
        let id = self.inner.id.get();
        match view {
            None => {
                if id != 0 {
                    self.inner.id.set(0);
                    conn.call_future(
                        Some(BUS_NAME),
                        OBJECT_PATH,
                        BUS_NAME,
                        "CloseNotification",
                        Some(&(id,).to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        5_000,
                    )
                    .await?;
                }
            }
            Some(view) => {
                let hints = glib::VariantDict::new(None);
                hints.insert("resident", true);
                hints.insert("desktop-entry", APP_ID);
                let actions =
                    [DEFAULT_ACTION.to_string(), tr("Open"), ACTION_COPY_LINK.to_string(), tr("Copy Link"), ACTION_STOP.to_string(), tr("Stop Sharing")];
                let args = glib::Variant::tuple_from_iter([
                    tr("Jastrzab").to_variant(),
                    id.to_variant(),
                    APP_ID.to_variant(),
                    view.title.to_variant(),
                    view.body.to_variant(),
                    actions.to_variant(),
                    hints.end(),
                    (-1i32).to_variant(),
                ]);
                let reply = conn.call_future(Some(BUS_NAME), OBJECT_PATH, BUS_NAME, "Notify", Some(&args), None, gio::DBusCallFlags::NONE, 5_000).await?;
                if let Some((new_id,)) = reply.get::<(u32,)>() {
                    self.inner.id.set(new_id);
                }
            }
        }
        Ok(())
    }

    async fn connection(&self) -> Result<gio::DBusConnection, glib::Error> {
        if let Some(conn) = self.inner.conn.borrow().clone() {
            return Ok(conn);
        }
        let conn = gio::bus_get_future(gio::BusType::Session).await?;
        let this = self.clone();
        let invoked = conn.subscribe_to_signal(
            Some(BUS_NAME),
            Some(BUS_NAME),
            Some("ActionInvoked"),
            Some(OBJECT_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some((id, action)) = signal.parameters.get::<(u32, String)>() else { return };
                if id != this.inner.id.get() {
                    return;
                }
                match action.as_str() {
                    ACTION_COPY_LINK => {
                        let _ = this.inner.events.try_send(Event::CopyLink(None));
                    }
                    ACTION_STOP => {
                        let _ = this.inner.events.try_send(Event::Stop);
                    }
                    _ => (this.inner.activate)(),
                }
            },
        );
        let this = self.clone();
        let closed = conn.subscribe_to_signal(
            Some(BUS_NAME),
            Some(BUS_NAME),
            Some("NotificationClosed"),
            Some(OBJECT_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if let Some((id, _reason)) = signal.parameters.get::<(u32, u32)>()
                    && id == this.inner.id.get()
                {
                    // Dismissed by the user. The in-app rows stay the source of truth, so it is not
                    // brought back until its text changes.
                    this.inner.id.set(0);
                }
            },
        );
        self.inner.subscriptions.borrow_mut().extend([invoked, closed]);
        *self.inner.conn.borrow_mut() = Some(conn.clone());
        Ok(conn)
    }
}
