//! The controller owns the sharing engine and everything that touches the outside world.
//!
//! UI callbacks, platform results and timer firings all become [`Event`]s on one queue. A single
//! task takes them one at a time, hands each to the engine with the current time, executes the
//! effects it returns through the platform adapters, and re-renders the window. Nothing handles an
//! event while another is being handled, so no GTK signal handler can re-enter the engine.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use async_channel::Sender;
use jastrzab_core::engine::{Effect, Engine, Event, LocationCmd, Now, StartRequest, TimerKind};
use jastrzab_core::session::Snapshot;
use jastrzab_core::settings::{DurationUnit, Mode, ProxyKind, Settings};
use jastrzab_core::view::{DialogSpec, PhaseView, Toast, View};
use jastrzab_core::AppPaths;

use crate::platform::http::Http;
use crate::platform::inhibit::SleepInhibitor;
use crate::platform::location::Location;
use crate::platform::notifications::Notifier;
use crate::platform::secrets::{self, Kind};
use crate::platform::store::DebouncedFile;
use crate::platform::{network, signals};
use crate::ui::main_window::MainWindow;

/// Settings are written once the user has stopped changing them for this long.
const SETTINGS_QUIET_SECS: u32 = 2;
/// The resume file changes rarely (start, link changes, stop): a short wait is enough.
const SESSION_QUIET_SECS: u32 = 1;
/// How long a termination request waits for the server to be told the share has ended.
const QUIT_GRACE_SECS: u32 = 4;

/// What the user typed into the "New Share" form.
#[derive(Debug, Clone, PartialEq)]
pub struct FormValues {
    pub duration: u32,
    pub unit: DurationUnit,
    pub mode: Mode,
    pub nickname: String,
    pub group_pin: String,
    pub adoptable: bool,
}

/// A cheap handle the UI uses to talk to the controller.
#[derive(Clone)]
pub struct Controller {
    inner: Rc<Inner>,
}

struct Inner {
    app: adw::Application,
    engine: RefCell<Engine>,
    tx: Sender<Event>,
    settings: Rc<RefCell<Settings>>,
    http: Http,
    location: Location,
    notifier: Notifier,
    inhibitor: SleepInhibitor,
    settings_file: DebouncedFile,
    session_file: DebouncedFile,
    session_path: std::path::PathBuf,
    timers: RefCell<HashMap<TimerKind, glib::SourceId>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
    window: RefCell<Option<MainWindow>>,
    /// Dialogs raised while no window was open, shown when one next is.
    pending_dialogs: RefCell<Vec<DialogSpec>>,
    quit_when_idle: Cell<bool>,
}

impl Controller {
    pub fn new(app: &adw::Application, paths: &AppPaths, settings: Settings) -> Self {
        let (tx, rx) = async_channel::unbounded::<Event>();
        let shared = Rc::new(RefCell::new(settings.clone()));

        let activate_app = app.downgrade();
        let notifier = Notifier::new(tx.clone(), move || {
            if let Some(app) = activate_app.upgrade() {
                app.activate();
            }
        });

        let inner = Rc::new(Inner {
            app: app.clone(),
            engine: RefCell::new(Engine::new(settings, Box::new(|buf| getrandom::fill(buf).expect("the operating system's random source")))),
            tx: tx.clone(),
            settings: shared.clone(),
            http: Http::new(shared, tx.clone()),
            location: Location::new(tx.clone()),
            notifier,
            inhibitor: SleepInhibitor::new(),
            settings_file: DebouncedFile::new(paths.settings_file(), SETTINGS_QUIET_SECS),
            session_file: DebouncedFile::new(paths.session_file(), SESSION_QUIET_SECS),
            session_path: paths.session_file(),
            timers: RefCell::new(HashMap::new()),
            hold: RefCell::new(None),
            window: RefCell::new(None),
            pending_dialogs: RefCell::new(Vec::new()),
            quit_when_idle: Cell::new(false),
        });
        let controller = Controller { inner };
        controller.register_log_terms(&controller.settings());

        // The event loop. Holding only a weak reference lets the controller be dropped at exit.
        let weak = Rc::downgrade(&controller.inner);
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                let Some(inner) = weak.upgrade() else { break };
                Controller { inner }.dispatch(event);
            }
        });

        network::watch(tx);
        controller.watch_termination();
        controller.load_resume_offer();
        controller
    }

    // --- what the UI calls -------------------------------------------------------------------

    pub fn send(&self, event: Event) {
        let _ = self.inner.tx.try_send(event);
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.borrow().clone()
    }

    pub fn view(&self) -> View {
        self.inner.engine.borrow().view()
    }

    /// Changes the settings: applied to the engine and the network layer at once, saved after a
    /// quiet period.
    pub fn update_settings(&self, change: impl FnOnce(&mut Settings)) {
        let (before, after) = {
            let mut settings = self.inner.settings.borrow_mut();
            let before = settings.clone();
            change(&mut settings);
            *settings = settings.clone().sanitized();
            (before, settings.clone())
        };
        if before == after {
            return;
        }
        if (before.proxy, &before.proxy_host, before.proxy_port, before.timeout_secs, before.tls)
            != (after.proxy, &after.proxy_host, after.proxy_port, after.timeout_secs, after.tls)
        {
            self.inner.http.reconfigure();
        }
        self.register_log_terms(&after);
        self.inner.settings_file.write(after.to_toml());
        self.send(Event::SettingsChanged(after));
    }

    /// Reads the secrets for the current server and starts a share.
    pub fn start(&self, form: FormValues) {
        let settings = self.settings();
        let server = settings.server_base().unwrap_or_default();
        let tx = self.inner.tx.clone();
        glib::spawn_future_local(async move {
            let password = secrets::lookup(Kind::ServerPassword, &server).await;
            let e2e_password = if settings.e2e_enabled { secrets::lookup(Kind::EncryptionPassword, &server).await } else { String::new() };
            let _ = tx.try_send(Event::Start(StartRequest {
                duration: form.duration,
                unit: form.unit,
                mode: form.mode,
                nickname: form.nickname,
                group_pin: form.group_pin,
                adoptable: form.adoptable,
                password,
                e2e_password,
            }));
        });
    }

    /// Reads a stored secret for the server currently configured.
    pub fn load_secret(&self, kind: Kind, done: impl FnOnce(String) + 'static) {
        let server = self.settings().server_base().unwrap_or_default();
        glib::spawn_future_local(async move { done(secrets::lookup(kind, &server).await) });
    }

    pub fn store_secret(&self, kind: Kind, secret: String) {
        let server = self.settings().server_base().unwrap_or_default();
        glib::spawn_future_local(async move { secrets::store(kind, &server, &secret).await });
    }

    pub fn app(&self) -> adw::Application {
        self.inner.app.clone()
    }

    // --- the window --------------------------------------------------------------------------

    /// Shows the window, creating it if the app is running without one (a share kept going after
    /// the window was closed).
    pub fn present_window(&self) {
        if let Some(window) = self.inner.window.borrow().as_ref() {
            window.present();
            return;
        }
        let window = MainWindow::new(self);
        *self.inner.window.borrow_mut() = Some(window.clone());
        window.render(&self.view());
        window.present();
        let pending: Vec<DialogSpec> = self.inner.pending_dialogs.borrow_mut().drain(..).collect();
        for spec in pending {
            window.dialog(&spec);
        }
    }

    pub fn window_closed(&self) {
        self.inner.window.borrow_mut().take();
        self.send(Event::WindowVisible(false));
        // Idle and windowless means nothing is left to do: GApplication exits by itself once the
        // last window is gone and nothing holds it.
    }

    pub fn open_preferences(&self) {
        if let Some(window) = self.inner.window.borrow().as_ref() {
            window.open_preferences();
        }
    }

    // --- the event loop ----------------------------------------------------------------------

    fn now(&self) -> Now {
        Now {
            mono_ms: (glib::monotonic_time() / 1000) as u64,
            unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64()),
        }
    }

    fn dispatch(&self, event: Event) {
        let effects = self.inner.engine.borrow_mut().handle(event, self.now());
        for effect in effects {
            self.execute(effect);
        }
        let view = self.view();
        if let Some(window) = self.inner.window.borrow().as_ref() {
            window.render(&view);
        }
        if self.inner.quit_when_idle.get() && view.phase == PhaseView::Idle {
            self.inner.app.quit();
        }
    }

    fn execute(&self, effect: Effect) {
        let inner = &self.inner;
        match effect {
            Effect::Http { id, base, request } => inner.http.send(id, base, request),
            Effect::Location(LocationCmd::Start { interval_secs }) => inner.location.start(interval_secs),
            Effect::Location(LocationCmd::Stop) => inner.location.stop(),
            Effect::Arm { kind, secs, generation } => self.arm(kind, secs, generation),
            Effect::Disarm(kind) => self.disarm(kind),
            Effect::PersistSession(Some(snapshot)) => {
                self.register_snapshot_terms(&snapshot);
                inner.session_file.write(snapshot.to_toml());
            }
            Effect::PersistSession(None) => {
                inner.session_file.remove();
                crate::log_privacy::global().clear_ids();
            }
            Effect::PersistSettings(settings) => {
                *inner.settings.borrow_mut() = settings.clone();
                inner.settings_file.write(settings.to_toml());
            }
            Effect::Inhibit(on) => inner.inhibitor.set(on),
            Effect::Hold(on) => {
                let mut hold = inner.hold.borrow_mut();
                if on && hold.is_none() {
                    *hold = Some(inner.app.hold());
                } else if !on {
                    hold.take();
                }
            }
            Effect::Notify(view) => inner.notifier.set(view),
            Effect::Toast(toast) => self.toast(&toast),
            Effect::Dialog(spec) => self.dialog(spec),
            Effect::Clipboard(text) => {
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
            }
        }
    }

    fn toast(&self, toast: &Toast) {
        if let Some(window) = self.inner.window.borrow().as_ref() {
            window.toast(toast);
        }
    }

    fn dialog(&self, spec: DialogSpec) {
        match self.inner.window.borrow().as_ref() {
            Some(window) => window.dialog(&spec),
            None => self.inner.pending_dialogs.borrow_mut().push(spec),
        }
    }

    fn arm(&self, kind: TimerKind, secs: u32, generation: u64) {
        self.disarm(kind);
        let weak: Weak<Inner> = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_seconds_local_once(secs, move || {
            if let Some(inner) = weak.upgrade() {
                inner.timers.borrow_mut().remove(&kind);
                let _ = inner.tx.try_send(Event::Timer { kind, generation });
            }
        });
        self.inner.timers.borrow_mut().insert(kind, id);
    }

    fn disarm(&self, kind: TimerKind) {
        if let Some(id) = self.inner.timers.borrow_mut().remove(&kind)
            && let Some(source) = glib::MainContext::default().find_source_by_id(&id)
        {
            source.destroy();
        }
    }

    // --- start-up and shutdown ---------------------------------------------------------------

    fn load_resume_offer(&self) {
        let Ok(text) = std::fs::read_to_string(&self.inner.session_path) else { return };
        let unix = self.now().unix;
        match Snapshot::from_toml(&text, unix) {
            Some(snapshot) => {
                self.register_snapshot_terms(&snapshot);
                self.send(Event::ResumeAvailable(snapshot));
            }
            None => {
                tracing::info!("discarding an unusable or expired resume file");
                self.inner.session_file.remove();
            }
        }
    }

    /// SIGINT/SIGTERM: end a running share properly, then quit.
    fn watch_termination(&self) {
        let Some(rx) = signals::watch() else { return };
        let weak = Rc::downgrade(&self.inner);
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                let Some(inner) = weak.upgrade() else { break };
                let controller = Controller { inner };
                tracing::info!("asked to terminate");
                controller.quit_gracefully();
            }
        });
    }

    pub fn quit_gracefully(&self) {
        let sharing = self.view().phase != PhaseView::Idle;
        if !sharing {
            self.inner.app.quit();
            return;
        }
        self.inner.quit_when_idle.set(true);
        self.send(Event::Stop);
        let app = self.inner.app.clone();
        glib::timeout_add_seconds_local_once(QUIT_GRACE_SECS, move || app.quit());
    }

    /// Writes what is still waiting to be written. Called as the app shuts down.
    pub fn shutdown(&self) {
        self.inner.settings_file.flush_now();
        self.inner.session_file.flush_now();
        self.inner.location.stop();
    }

    // --- log scrubbing -----------------------------------------------------------------------

    fn register_log_terms(&self, settings: &Settings) {
        let redactor = crate::log_privacy::global();
        redactor.set_enabled(settings.anonymize_logs);
        redactor.clear_registered_terms();
        if let Some(server) = settings.server_base() {
            redactor.register_server_url(&server);
        }
        redactor.register_username(&settings.username);
        if settings.proxy != ProxyKind::System && settings.proxy != ProxyKind::None {
            redactor.register_address(&settings.proxy_host);
        }
    }

    fn register_snapshot_terms(&self, snapshot: &Snapshot) {
        let redactor = crate::log_privacy::global();
        redactor.register_id(&snapshot.sid);
        for link in &snapshot.links {
            redactor.register_id(&link.id);
        }
        if let Some(pin) = &snapshot.group_pin {
            redactor.register_id(pin);
        }
    }
}
