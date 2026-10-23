//! The sharing engine: a sans-IO state machine.
//!
//! [`Engine::handle`] takes an [`Event`] and the current time and returns [`Effect`]s. It never
//! reads a clock, draws a random number from the OS, opens a socket or touches a widget; the app
//! executes the effects with real adapters and feeds their results back as events. That makes
//! every rule about sessions, batching and the GPS signal testable without a phone.
//!
//! One event is handled at a time, to completion, so nothing re-enters the engine.

use std::collections::{HashMap, HashSet};

use hauk::crypto::{E2eKey, IV_LEN, SALT_LEN};
use hauk::request::{Adopt, Create, E2ePost, NewLink, Post, Request, Stop};
use hauk::response::{self, Reply};
use hauk::version;

use crate::geo::Fix;
use crate::i18n::{tr, tr_args};
use crate::session::{Link, LinkKind, Session, Snapshot};
use crate::settings::{DurationUnit, MAX_DURATION_SECS, Mode, Settings};
use crate::signal::{Level, LostCheck, SignalMonitor};
use crate::uplink::{HttpId, Kind, Poll, Uplink};
use crate::view::*;

/// How long a pre-share signal check runs before it stops by itself.
pub const CHECK_LIMIT_SECS: u32 = 300;
/// While only checking the signal, ask for a fix every second so the display feels live.
const CHECK_INTERVAL_SECS: u32 = 1;
/// How long the network must stay gone before location is switched off. A Wi-Fi hand-over or a
/// tunnel of a few seconds is not worth stopping and restarting the GPS for.
const OFFLINE_PAUSE_SECS: u32 = 20;

/// The current time: a monotonic reading for scheduling and the wall clock for expiry and
/// timestamps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Now {
    pub mono_ms: u64,
    pub unix: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerKind {
    /// The share's time is up.
    Expiry,
    /// A waiting post may go out.
    Send,
    /// Check whether the GPS signal has gone quiet.
    SignalLost,
    /// The pre-share signal check has run long enough.
    CheckLimit,
    /// The network has been gone long enough that location is switched off until it returns.
    OfflinePause,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocationFailure {
    /// The user, or the system policy, refused location access.
    Denied,
    Unavailable(String),
}

/// What the user entered in the "New Share" form, plus the secrets from the keyring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest {
    pub duration: u32,
    pub unit: DurationUnit,
    pub mode: Mode,
    pub nickname: String,
    pub group_pin: String,
    pub adoptable: bool,
    pub password: String,
    pub e2e_password: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Start(StartRequest),
    Stop,
    NewLink {
        adoptable: bool,
    },
    StopLink(String),
    Adopt {
        target: String,
        nickname: String,
    },
    /// Copy a link (`None`: the first one) to the clipboard.
    CopyLink(Option<String>),
    CopyPin,
    /// At start-up: an unfinished session was found on disk.
    ResumeAvailable(Snapshot),
    Resume,
    DismissResume,
    CheckSignal(bool),
    WindowVisible(bool),
    LocationStarted,
    LocationFailed(LocationFailure),
    Fix(Fix),
    /// A request finished. `Err` is a transport failure (no connection, TLS, timeout).
    HttpDone {
        id: HttpId,
        result: Result<Reply, String>,
    },
    Network(bool),
    Timer {
        kind: TimerKind,
        generation: u64,
    },
    SettingsChanged(Settings),
}

impl Event {
    /// A short name for logs. The events themselves are not logged: they carry coordinates,
    /// secrets and share IDs.
    pub fn name(&self) -> &'static str {
        match self {
            Event::Start(_) => "start",
            Event::Stop => "stop",
            Event::NewLink { .. } => "new-link",
            Event::StopLink(_) => "stop-link",
            Event::Adopt { .. } => "adopt",
            Event::CopyLink(_) => "copy-link",
            Event::CopyPin => "copy-pin",
            Event::ResumeAvailable(_) => "resume-available",
            Event::Resume => "resume",
            Event::DismissResume => "dismiss-resume",
            Event::CheckSignal(_) => "check-signal",
            Event::WindowVisible(_) => "window-visible",
            Event::LocationStarted => "location-started",
            Event::LocationFailed(_) => "location-failed",
            Event::Fix(_) => "fix",
            Event::HttpDone { .. } => "http-done",
            Event::Network(_) => "network",
            Event::Timer { .. } => "timer",
            Event::SettingsChanged(_) => "settings-changed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationCmd {
    /// Start (or retune) location updates at this interval. Always answered with
    /// `LocationStarted` or `LocationFailed`, even when already running.
    Start {
        interval_secs: u32,
    },
    Stop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Http {
        id: HttpId,
        base: String,
        request: Request,
    },
    Location(LocationCmd),
    Arm {
        kind: TimerKind,
        secs: u32,
        generation: u64,
    },
    Disarm(TimerKind),
    /// `None` deletes the resume file.
    PersistSession(Option<Snapshot>),
    PersistSettings(Settings),
    /// Hold or release the suspend inhibitor.
    Inhibit(bool),
    /// Keep the process alive without a window, or stop doing so.
    Hold(bool),
    /// `None` withdraws the notification.
    Notify(Option<NotificationView>),
    Toast(Toast),
    Dialog(DialogSpec),
    Clipboard(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Connecting,
    Active,
    Stopping,
}

#[derive(Debug, Clone, PartialEq)]
enum Conn {
    Ok,
    /// The system reports no network.
    Offline,
    /// The server could not be reached, or answered with an HTTP error.
    Unreachable,
    /// The server answered, but not with success.
    Problem(String),
}

/// Requests other than posts, which go through the uplink in order.
#[derive(Debug, Clone, PartialEq)]
enum Control {
    NewLink { adoptable: bool },
    Adopt { target: String, nickname: String },
    StopLink(String),
}

#[derive(Debug)]
enum Stage {
    AwaitLocation,
    AwaitCreate,
}

struct Starting {
    request: StartRequest,
    base: String,
    stage: Stage,
    /// The password and salt of an end-to-end encrypted session, until the key is derived.
    e2e: Option<(Vec<u8>, [u8; SALT_LEN])>,
    /// Set when resuming: the session to continue instead of creating one.
    resuming: Option<Session>,
}

/// Fills a buffer with cryptographically secure random bytes.
pub type RandomFill = Box<dyn FnMut(&mut [u8])>;

pub struct Engine {
    settings: Settings,
    random: RandomFill,
    phase: Phase,
    starting: Option<Starting>,
    session: Option<Session>,
    uplink: Uplink<Control>,
    controls: HashMap<HttpId, Control>,
    signal: SignalMonitor,
    checking: bool,
    window_visible: bool,
    /// What the system says about the network.
    net_up: bool,
    /// Whether the app acts as if there is a network: `net_up`, or always when the user turned
    /// `pause_when_offline` off.
    online: bool,
    conn: Conn,
    next_id: HttpId,
    create_id: Option<HttpId>,
    farewell_id: Option<HttpId>,
    generations: HashMap<TimerKind, u64>,
    armed: HashSet<TimerKind>,
    send_due_ms: Option<u64>,
    lost_toast_shown: bool,
    /// Whether location updates have been requested (and not yet stopped).
    location_on: bool,
    /// Location was switched off by [`TimerKind::OfflinePause`] and comes back with the network.
    paused: bool,
    /// The latest fix while a check or a connecting share is running, so a share started after
    /// a check posts its first point at once.
    latest_fix: Option<Fix>,
    resume_offer: Option<Snapshot>,
    hold: bool,
    inhibit: bool,
    last_notification: Option<NotificationView>,
    out: Vec<Effect>,
}

impl Engine {
    /// `random` fills a buffer with cryptographically secure random bytes (salts and IVs).
    pub fn new(settings: Settings, random: RandomFill) -> Self {
        Self {
            settings: settings.sanitized(),
            random,
            phase: Phase::Idle,
            starting: None,
            session: None,
            uplink: Uplink::new(),
            controls: HashMap::new(),
            signal: SignalMonitor::new(),
            checking: false,
            window_visible: true,
            net_up: true,
            online: true,
            conn: Conn::Ok,
            next_id: 1,
            create_id: None,
            farewell_id: None,
            generations: HashMap::new(),
            armed: HashSet::new(),
            send_due_ms: None,
            lost_toast_shown: false,
            location_on: false,
            paused: false,
            latest_fix: None,
            resume_offer: None,
            hold: false,
            inhibit: false,
            last_notification: None,
            out: Vec::new(),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn is_sharing(&self) -> bool {
        self.phase == Phase::Active
    }

    // ----------------------------------------------------------------------------------------
    // Event handling

    pub fn handle(&mut self, event: Event, now: Now) -> Vec<Effect> {
        match event {
            Event::Start(request) => self.start(request),
            Event::Stop => self.stop_by_user(),
            Event::NewLink { adoptable } => self.queue_control(Control::NewLink { adoptable }, now),
            Event::StopLink(id) => self.stop_link(id, now),
            Event::Adopt { target, nickname } => self.queue_control(Control::Adopt { target, nickname }, now),
            Event::CopyLink(id) => self.copy_link(id),
            Event::CopyPin => {
                if let Some(pin) = self.session.as_ref().and_then(|s| s.group_pin.clone()) {
                    self.out.push(Effect::Clipboard(pin));
                    self.out.push(Effect::Toast(Toast::text(tr("PIN copied"))));
                }
            }
            Event::ResumeAvailable(snapshot) => {
                if self.phase == Phase::Idle && snapshot.expires_at_unix > now.unix && snapshot.server_base_ok() {
                    self.resume_offer = Some(snapshot);
                } else {
                    self.out.push(Effect::PersistSession(None));
                }
            }
            Event::Resume => self.resume(),
            Event::DismissResume => {
                if self.resume_offer.take().is_some() {
                    self.out.push(Effect::PersistSession(None));
                }
            }
            Event::CheckSignal(on) => self.check_signal(on),
            Event::WindowVisible(visible) => {
                self.window_visible = visible;
                if !visible && self.checking {
                    self.stop_check();
                }
            }
            Event::LocationStarted => self.location_started(now),
            Event::LocationFailed(failure) => self.location_failed(failure),
            Event::Fix(fix) => self.fix(fix, now),
            Event::HttpDone { id, result } => self.http_done(id, result, now),
            Event::Network(up) => {
                self.net_up = up;
                self.sync_online(now);
            }
            Event::Timer { kind, generation } => self.timer(kind, generation, now),
            Event::SettingsChanged(settings) => {
                self.settings = settings.sanitized();
                self.sync_online(now);
            }
        }
        self.sync_notification();
        std::mem::take(&mut self.out)
    }

    // ----------------------------------------------------------------------------------------
    // Starting

    fn start(&mut self, request: StartRequest) {
        if self.phase != Phase::Idle {
            return;
        }
        let Some(base) = self.settings.server_base() else {
            self.out.push(Effect::Dialog(DialogSpec {
                heading: tr("No Server Set"),
                body: tr("Enter the address of your Hauk server in Preferences, then try again."),
                open_preferences: true,
            }));
            return;
        };
        let duration_secs = u64::from(request.duration) * request.unit.seconds();
        if request.duration == 0 || duration_secs > MAX_DURATION_SECS {
            self.out.push(Effect::Dialog(DialogSpec::error(tr("Invalid Duration"), tr("Choose a duration between one minute and 31 days."))));
            return;
        }
        if request.mode.is_group() && request.nickname.trim().is_empty() {
            self.out
                .push(Effect::Dialog(DialogSpec::error(tr("Nickname Required"), tr("Group shares need a nickname so people can tell you apart on the map."))));
            return;
        }
        if request.mode == Mode::JoinGroup && request.group_pin.trim().is_empty() {
            self.out.push(Effect::Dialog(DialogSpec::error(tr("Group PIN Required"), tr("Enter the PIN of the group you want to join."))));
            return;
        }
        let e2e = self.settings.e2e_enabled;
        if e2e && request.mode.is_group() {
            self.out.push(Effect::Dialog(DialogSpec {
                heading: tr("Encryption Unavailable"),
                body: tr("End-to-end encryption is not supported for group shares. Turn it off in Preferences, or share just your own location."),
                open_preferences: true,
            }));
            return;
        }
        if e2e && request.e2e_password.is_empty() {
            self.out.push(Effect::Dialog(DialogSpec {
                heading: tr("Encryption Password Needed"),
                body: tr("End-to-end encryption is on, but no encryption password is set. Set one in Preferences."),
                open_preferences: true,
            }));
            return;
        }

        // Remember the form for next time.
        self.settings.duration = request.duration;
        self.settings.unit = request.unit;
        self.settings.nickname = request.nickname.trim().to_string();
        self.settings.adoptable = request.adoptable;
        self.out.push(Effect::PersistSettings(self.settings.clone()));

        let e2e = if e2e {
            let mut salt = [0u8; SALT_LEN];
            (self.random)(&mut salt);
            Some((request.e2e_password.clone().into_bytes(), salt))
        } else {
            None
        };
        self.checking = false;
        self.disarm(TimerKind::CheckLimit);
        self.begin_connecting(Starting { request, base, stage: Stage::AwaitLocation, e2e, resuming: None });
    }

    fn begin_connecting(&mut self, starting: Starting) {
        self.phase = Phase::Connecting;
        let interval = self.settings.interval_secs;
        self.starting = Some(starting);
        self.signal.start(interval);
        if let Some(fix) = self.latest_fix.take() {
            self.uplink.offer_fix(fix, 0.0);
        }
        self.set_hold(true);
        self.location_start(interval);
    }

    fn resume(&mut self) {
        let Some(snapshot) = self.resume_offer.take() else { return };
        let Some(session) = snapshot.into_session() else {
            self.out.push(Effect::PersistSession(None));
            self.out.push(Effect::Dialog(DialogSpec::error(tr("Couldn't Resume"), tr("The saved share could not be read."))));
            return;
        };
        let request = StartRequest {
            duration: 0,
            unit: DurationUnit::Minutes,
            mode: session.mode,
            nickname: String::new(),
            group_pin: String::new(),
            adoptable: false,
            password: String::new(),
            e2e_password: String::new(),
        };
        let interval = session.interval_secs;
        self.phase = Phase::Connecting;
        self.starting = Some(Starting { request, base: session.server.clone(), stage: Stage::AwaitLocation, e2e: None, resuming: Some(session) });
        self.signal.start(interval);
        self.set_hold(true);
        self.location_start(interval);
    }

    fn location_started(&mut self, now: Now) {
        if self.phase != Phase::Connecting {
            return;
        }
        let Some(starting) = self.starting.as_mut() else { return };
        if !matches!(starting.stage, Stage::AwaitLocation) {
            return;
        }
        if let Some(session) = starting.resuming.take() {
            self.starting = None;
            self.activate(session, now, false);
            return;
        }
        starting.stage = Stage::AwaitCreate;
        let create = self.build_create();
        let id = self.alloc_id();
        self.create_id = Some(id);
        let base = self.starting.as_ref().map(|s| s.base.clone()).unwrap_or_default();
        self.out.push(Effect::Http { id, base, request: Request::Create(create) });
    }

    fn build_create(&self) -> Create {
        let starting = self.starting.as_ref().expect("connecting");
        let r = &starting.request;
        let s = &self.settings;
        let non_empty = |text: &str| (!text.trim().is_empty()).then(|| text.trim().to_string());
        Create {
            username: non_empty(&s.username),
            password: r.password.clone(),
            link_id: non_empty(&s.preferred_link_id),
            duration_secs: u64::from(r.duration) * r.unit.seconds(),
            interval_secs: s.interval_secs,
            mode: r.mode.share_mode(),
            adoptable: r.adoptable,
            nickname: r.mode.is_group().then(|| r.nickname.trim().to_string()),
            group_pin: (r.mode == Mode::JoinGroup).then(|| r.group_pin.trim().to_string()),
            e2e_salt: starting.e2e.as_ref().map(|(_, salt)| *salt),
        }
    }

    fn create_done(&mut self, result: Result<Reply, String>, now: Now) {
        self.create_id = None;
        let Some(starting) = self.starting.take() else { return };
        let request_mode = starting.request.mode;
        let reply = match result {
            Ok(reply) => reply,
            Err(message) => return self.abort_start(Failure::Transport(message)),
        };
        // A server too old for groups ignores the mode and creates a solo share.
        let server_version = reply.version();
        let mode = if request_mode.is_group() && !server_version.is_at_least(version::GROUP_SHARES) { Mode::Alone } else { request_mode };
        let created = match response::parse_create(&reply, mode.share_mode()) {
            Ok(created) => created,
            Err(err) => return self.abort_start(Failure::Hauk(err)),
        };

        // Encryption must not be silently dropped: if the server cannot do it, nothing is shared.
        if starting.e2e.is_some() && !server_version.is_at_least(version::E2E) {
            self.send_farewell(created.sid, None, &starting.base);
            self.out.push(Effect::Dialog(DialogSpec::error(
                tr("Server Is Out of Date"),
                tr_args(
                    "This server (version {version}) does not support end-to-end encryption, so nothing was shared.",
                    &[("version", &server_version.to_string())],
                ),
            )));
            return;
        }

        let e2e = starting.e2e.as_ref().map(|(password, salt)| E2eKey::derive(password, salt));
        let kind = match mode {
            Mode::Alone => LinkKind::Own,
            Mode::CreateGroup => LinkKind::GroupHost,
            Mode::JoinGroup => LinkKind::GroupMember,
        };
        let link_id = created.link_id.clone().unwrap_or_else(|| created.view_url.clone());
        let duration_secs = u64::from(starting.request.duration) * starting.request.unit.seconds();
        let session = Session {
            sid: created.sid,
            server: starting.base.clone(),
            mode,
            expires_at_unix: now.unix + duration_secs as f64,
            interval_secs: self.settings.interval_secs,
            group_pin: created.group_pin,
            e2e,
            links: vec![Link { id: link_id, url: created.view_url, kind }],
            link_format: None,
        };
        if request_mode.is_group() && mode == Mode::Alone {
            self.out.push(Effect::Dialog(DialogSpec::error(
                tr("Server Is Out of Date"),
                tr_args(
                    "This server (version {version}) does not support group shares. Your location is shared on its own.",
                    &[("version", &server_version.to_string())],
                ),
            )));
        }
        self.activate(session, now, true);
    }

    fn abort_start(&mut self, failure: Failure) {
        self.out.push(Effect::Dialog(failure.dialog()));
        self.after_session_ended();
    }

    fn activate(&mut self, session: Session, now: Now, announce: bool) {
        let remaining = (session.expires_at_unix - now.unix).max(1.0).ceil() as u32;
        self.phase = Phase::Active;
        self.conn = if self.online { Conn::Ok } else { Conn::Offline };
        self.lost_toast_shown = false;
        self.out.push(Effect::PersistSession(Some(session.snapshot())));
        self.session = Some(session);
        self.set_inhibit(true);
        self.arm(TimerKind::Expiry, remaining);
        if self.signal.has_fix() && !self.armed.contains(&TimerKind::SignalLost) {
            self.arm(TimerKind::SignalLost, self.signal.lost_after_secs());
        }
        if announce {
            self.out.push(Effect::Toast(Toast { title: tr("Sharing started"), copy_link: true }));
        }
        if !self.online {
            self.arm(TimerKind::OfflinePause, OFFLINE_PAUSE_SECS); // resumed with no network
        }
        self.pump(now);
    }

    // ----------------------------------------------------------------------------------------
    // Stopping

    fn stop_by_user(&mut self) {
        if self.phase != Phase::Active {
            return;
        }
        let Some(session) = self.session.clone() else { return };
        let send = self.online;
        self.end_session();
        if send {
            self.phase = Phase::Stopping;
            self.send_farewell(session.sid, None, &session.server);
        } else {
            self.out.push(Effect::Toast(Toast::text(tr("Offline. The share ends on the server when its time is up."))));
            self.after_session_ended();
        }
    }

    /// The share's time ran out. The server expires it by itself, so nothing is sent.
    fn expired(&mut self) {
        if self.phase != Phase::Active {
            return;
        }
        self.end_session();
        self.out.push(Effect::Toast(Toast::text(tr("Sharing has ended"))));
        self.after_session_ended();
    }

    /// Tears down everything a running share keeps going. Does not change the phase.
    fn end_session(&mut self) {
        self.uplink.clear();
        self.controls.clear();
        self.session = None;
        for kind in [TimerKind::Expiry, TimerKind::Send, TimerKind::SignalLost, TimerKind::OfflinePause] {
            self.disarm(kind);
        }
        self.paused = false;
        self.send_due_ms = None;
        self.set_inhibit(false);
        self.out.push(Effect::PersistSession(None));
    }

    /// Back to idle: stop location, drop the monitor, release the process.
    fn after_session_ended(&mut self) {
        self.phase = Phase::Idle;
        self.starting = None;
        self.session = None;
        self.checking = false;
        self.conn = Conn::Ok;
        self.signal.stop();
        self.latest_fix = None;
        self.disarm(TimerKind::SignalLost);
        self.disarm(TimerKind::CheckLimit);
        self.location_stop();
        self.set_hold(false);
    }

    fn send_farewell(&mut self, sid: String, link_id: Option<String>, base: &str) {
        let id = self.alloc_id();
        self.farewell_id = Some(id);
        self.out.push(Effect::Http { id, base: base.to_string(), request: Request::Stop(Stop { sid, link_id }) });
        if self.phase != Phase::Stopping {
            self.phase = Phase::Stopping;
        }
    }

    fn farewell_done(&mut self, result: Result<Reply, String>) {
        self.farewell_id = None;
        let outcome = match result {
            Ok(reply) => response::parse_ok(&reply).map_err(Failure::Hauk),
            Err(message) => Err(Failure::Transport(message)),
        };
        if let Err(failure) = outcome {
            tracing::warn!(error = %failure.text(), "couldn't tell the server the share ended");
            self.out.push(Effect::Toast(Toast::text(tr("Couldn't tell the server. The share ends when its time is up."))));
        }
        self.after_session_ended();
    }

    fn stop_link(&mut self, id: String, now: Now) {
        let Some(session) = &self.session else { return };
        if session.link(&id).is_none() {
            return;
        }
        if session.links.len() <= 1 {
            return self.stop_by_user();
        }
        self.queue_control(Control::StopLink(id), now);
    }

    // ----------------------------------------------------------------------------------------
    // The signal check before sharing

    fn check_signal(&mut self, on: bool) {
        if !on {
            if self.checking {
                self.stop_check();
            }
            return;
        }
        if self.phase != Phase::Idle || self.checking {
            return;
        }
        self.checking = true;
        self.signal.start(CHECK_INTERVAL_SECS);
        self.location_start(CHECK_INTERVAL_SECS);
        self.arm(TimerKind::CheckLimit, CHECK_LIMIT_SECS);
    }

    fn stop_check(&mut self) {
        self.checking = false;
        self.disarm(TimerKind::CheckLimit);
        self.disarm(TimerKind::SignalLost);
        if self.phase == Phase::Idle {
            self.signal.stop();
            self.latest_fix = None;
            self.location_stop();
        }
    }

    // ----------------------------------------------------------------------------------------
    // Location

    fn location_failed(&mut self, failure: LocationFailure) {
        let (heading, body) = match &failure {
            LocationFailure::Denied => {
                (tr("Location Access Denied"), tr("Jastrzab isn't allowed to use your location. Turn on location services in Settings, then try again."))
            }
            LocationFailure::Unavailable(reason) => {
                (tr("Location Unavailable"), tr_args("The location service is not available: {reason}", &[("reason", reason)]))
            }
        };
        self.location_on = false;
        match self.phase {
            Phase::Connecting => {
                if let Some(create) = self.create_id.take() {
                    tracing::warn!(create, "location failed while a session was being created");
                }
                self.out.push(Effect::Dialog(DialogSpec::error(heading, body)));
                self.after_session_ended();
            }
            Phase::Idle if self.checking => {
                self.out.push(Effect::Dialog(DialogSpec::error(heading, body)));
                self.stop_check();
            }
            Phase::Active => {
                // The share carries on (the server keeps the last point); say so once.
                self.out.push(Effect::Dialog(DialogSpec::error(heading, body)));
            }
            _ => {}
        }
    }

    fn fix(&mut self, fix: Fix, now: Now) {
        if !self.signal.is_active() || self.paused {
            return;
        }
        self.signal.on_fix(&fix, now.mono_ms);
        self.lost_toast_shown = false;
        if self.phase != Phase::Active {
            self.latest_fix = Some(fix.clone());
        }
        if !self.armed.contains(&TimerKind::SignalLost) {
            self.arm(TimerKind::SignalLost, self.signal.lost_after_secs());
        }
        if matches!(self.phase, Phase::Connecting | Phase::Active) {
            self.uplink.offer_fix(fix, self.settings.min_distance_m);
            self.pump(now);
        }
    }

    // ----------------------------------------------------------------------------------------
    // Requests

    fn queue_control(&mut self, control: Control, now: Now) {
        if self.phase != Phase::Active {
            return;
        }
        if matches!(control, Control::Adopt { .. }) && self.session.as_ref().and_then(|s| s.group_pin.as_ref()).is_none() {
            return;
        }
        self.uplink.push_control(control);
        self.pump(now);
    }

    fn alloc_id(&mut self) -> HttpId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn interval_ms(&self) -> u64 {
        self.session.as_ref().map_or(self.settings.interval_secs, |s| s.interval_secs).max(1) as u64 * 1000
    }

    /// Sends whatever the uplink says is next, or arms the timer for when it may.
    fn pump(&mut self, now: Now) {
        let Some(session) = &self.session else { return };
        let (sid, base, e2e) = (session.sid.clone(), session.server.clone(), session.e2e.clone());
        let group_pin = session.group_pin.clone();
        match self.uplink.poll(now.mono_ms, self.interval_ms(), self.online) {
            Poll::Control(control) => {
                let request = match &control {
                    Control::NewLink { adoptable } => Request::NewLink(NewLink { sid, adoptable: *adoptable }),
                    Control::Adopt { target, nickname } => {
                        Request::Adopt(Adopt { sid, nickname: nickname.clone(), target_id: target.clone(), group_pin: group_pin.unwrap_or_default() })
                    }
                    Control::StopLink(id) => Request::Stop(Stop { sid, link_id: Some(id.clone()) }),
                };
                let id = self.alloc_id();
                self.uplink.begin(id, Kind::Control, now.mono_ms);
                self.controls.insert(id, control);
                self.out.push(Effect::Http { id, base, request });
            }
            Poll::Post(fix) => {
                let e2e = e2e.map(|key| {
                    let mut iv = [0u8; IV_LEN];
                    (self.random)(&mut iv);
                    E2ePost { key, iv }
                });
                let request = Request::Post(Post {
                    sid,
                    latitude: fix.latitude,
                    longitude: fix.longitude,
                    time: fix.unix_time,
                    provider: fix.provider(),
                    speed: fix.speed,
                    accuracy: (fix.accuracy_m >= 0.0).then_some(fix.accuracy_m as f32),
                    e2e,
                });
                let id = self.alloc_id();
                self.uplink.begin(id, Kind::Post, now.mono_ms);
                self.out.push(Effect::Http { id, base, request });
            }
            Poll::Wait(secs) => {
                let due = now.mono_ms + u64::from(secs) * 1000;
                if !self.armed.contains(&TimerKind::Send) || self.send_due_ms.is_none_or(|current| current > due) {
                    self.send_due_ms = Some(due);
                    self.arm(TimerKind::Send, secs);
                }
            }
            Poll::Busy | Poll::Idle => {}
        }
    }

    fn http_done(&mut self, id: HttpId, result: Result<Reply, String>, now: Now) {
        if self.create_id == Some(id) {
            return self.create_done(result, now);
        }
        if self.farewell_id == Some(id) {
            return self.farewell_done(result);
        }
        let Some((current, kind)) = self.uplink.in_flight() else { return };
        if current != id {
            return;
        }
        match kind {
            Kind::Post => self.post_done(id, result, now),
            Kind::Control => self.control_done(id, result, now),
        }
    }

    fn post_done(&mut self, id: HttpId, result: Result<Reply, String>, now: Now) {
        let outcome = match result {
            Ok(reply) => response::parse_post(&reply).map_err(Failure::Hauk),
            Err(message) => Err(Failure::Transport(message)),
        };
        match outcome {
            Ok(ok) => {
                self.uplink.finish_ok(id);
                self.conn = if self.online { Conn::Ok } else { Conn::Offline };
                if let (Some(links), Some(session)) = (ok.links, self.session.as_mut())
                    && session.reconcile(&links)
                {
                    self.out.push(Effect::PersistSession(Some(session.snapshot())));
                }
            }
            Err(failure) => {
                self.uplink.finish_err(id, now.mono_ms);
                tracing::warn!(error = %failure.text(), "a location post failed");
                self.conn = match failure {
                    Failure::Hauk(hauk::Error::Server(message)) => Conn::Problem(message),
                    _ if !self.online => Conn::Offline,
                    _ => Conn::Unreachable,
                };
            }
        }
        self.pump(now);
    }

    fn control_done(&mut self, id: HttpId, result: Result<Reply, String>, now: Now) {
        let control = self.controls.remove(&id);
        let outcome: Result<ControlOk, Failure> = match (&control, result) {
            (_, Err(message)) => Err(Failure::Transport(message)),
            (Some(Control::NewLink { .. }), Ok(reply)) => response::parse_new_link(&reply).map(ControlOk::NewLink).map_err(Failure::Hauk),
            (_, Ok(reply)) => response::parse_ok(&reply).map(|()| ControlOk::Plain).map_err(Failure::Hauk),
        };
        match outcome {
            Ok(ok) => {
                self.uplink.finish_ok(id);
                if let Some(session) = self.session.as_mut() {
                    match (control, ok) {
                        (Some(Control::NewLink { .. }), ControlOk::NewLink(link)) => {
                            session.add_link(Link { id: link.link_id, url: link.view_url, kind: LinkKind::Own });
                            self.out.push(Effect::Toast(Toast::text(tr("Link created"))));
                        }
                        (Some(Control::Adopt { nickname, .. }), _) => {
                            self.out.push(Effect::Toast(Toast::text(tr_args("{nickname} has been added to your share!", &[("nickname", &nickname)]))));
                        }
                        (Some(Control::StopLink(link_id)), _) => {
                            session.remove_link(&link_id);
                        }
                        _ => {}
                    }
                    self.out.push(Effect::PersistSession(Some(session.snapshot())));
                }
            }
            Err(failure) => {
                self.uplink.finish_err(id, now.mono_ms);
                self.out.push(Effect::Dialog(failure.dialog()));
            }
        }
        self.pump(now);
    }

    fn sync_online(&mut self, now: Now) {
        self.network(self.net_up || !self.settings.pause_when_offline, now);
    }

    fn network(&mut self, up: bool, now: Now) {
        let changed = self.online != up;
        self.online = up;
        if !changed {
            return;
        }
        if self.phase != Phase::Active {
            return;
        }
        if up {
            // A phone that slept through the outage fires its timers late.
            if self.session.as_ref().is_some_and(|s| s.expires_at_unix <= now.unix) {
                self.expired();
                return;
            }
            self.uplink.network_restored();
            if self.conn == Conn::Offline {
                self.conn = Conn::Unreachable; // until a post gets through
            }
            self.disarm(TimerKind::OfflinePause);
            if self.paused {
                tracing::info!("network back: location on again");
                self.paused = false;
                self.signal.resume();
                self.location_start((self.interval_ms() / 1000) as u32);
            }
        } else {
            self.conn = Conn::Offline;
            self.arm(TimerKind::OfflinePause, OFFLINE_PAUSE_SECS);
        }
        self.pump(now);
    }

    /// The network has stayed gone: nothing can be sent, so the GPS (the biggest drain) and the
    /// signal watch go off until it returns. The share, its timer and the wake lock stay.
    fn pause_for_offline(&mut self) {
        if self.phase != Phase::Active || self.online || self.paused {
            return;
        }
        tracing::info!("no network for a while: location off until it returns");
        self.paused = true;
        self.disarm(TimerKind::SignalLost);
        self.signal.pause();
        self.location_stop();
    }

    fn timer(&mut self, kind: TimerKind, generation: u64, now: Now) {
        if self.generations.get(&kind) != Some(&generation) || !self.armed.remove(&kind) {
            return;
        }
        match kind {
            TimerKind::Expiry => self.expired(),
            TimerKind::OfflinePause => self.pause_for_offline(),
            TimerKind::Send => {
                self.send_due_ms = None;
                self.pump(now);
            }
            TimerKind::SignalLost => match self.signal.check_lost(now.mono_ms) {
                LostCheck::Recheck(secs) => self.arm(TimerKind::SignalLost, secs),
                LostCheck::BecameLost => {
                    if self.phase == Phase::Active && !self.lost_toast_shown {
                        self.lost_toast_shown = true;
                        self.out.push(Effect::Toast(Toast::text(tr("GPS signal lost"))));
                    }
                }
                LostCheck::Nothing => {}
            },
            TimerKind::CheckLimit => {
                if self.checking {
                    self.stop_check();
                }
            }
        }
    }

    // ----------------------------------------------------------------------------------------
    // Helpers

    fn copy_link(&mut self, id: Option<String>) {
        let Some(session) = &self.session else { return };
        let link = match &id {
            Some(id) => session.link(id),
            None => session.primary_link(),
        };
        if let Some(link) = link {
            self.out.push(Effect::Clipboard(link.url.clone()));
            self.out.push(Effect::Toast(Toast::text(tr("Link copied"))));
        }
    }

    fn arm(&mut self, kind: TimerKind, secs: u32) {
        let generation = self.generations.entry(kind).or_insert(0);
        *generation += 1;
        self.armed.insert(kind);
        self.out.push(Effect::Arm { kind, secs: secs.max(1), generation: *generation });
    }

    fn disarm(&mut self, kind: TimerKind) {
        if self.armed.remove(&kind) {
            *self.generations.entry(kind).or_insert(0) += 1;
            self.out.push(Effect::Disarm(kind));
        }
    }

    fn location_start(&mut self, interval_secs: u32) {
        self.location_on = true;
        self.out.push(Effect::Location(LocationCmd::Start { interval_secs }));
    }

    fn location_stop(&mut self) {
        if self.location_on {
            self.location_on = false;
            self.out.push(Effect::Location(LocationCmd::Stop));
        }
    }

    fn set_hold(&mut self, on: bool) {
        if self.hold != on {
            self.hold = on;
            self.out.push(Effect::Hold(on));
        }
    }

    fn set_inhibit(&mut self, on: bool) {
        if self.inhibit != on {
            self.inhibit = on;
            self.out.push(Effect::Inhibit(on));
        }
    }

    fn sync_notification(&mut self) {
        let wanted = self.notification();
        if wanted != self.last_notification {
            self.last_notification = wanted.clone();
            self.out.push(Effect::Notify(wanted));
        }
    }

    fn notification(&self) -> Option<NotificationView> {
        let session = self.session.as_ref().filter(|_| self.phase == Phase::Active)?;
        let title = match (&self.conn, self.signal.level()) {
            (Conn::Offline, _) => tr("Sharing · No network connection"),
            (Conn::Unreachable | Conn::Problem(_), _) => tr("Sharing · Server unreachable"),
            (Conn::Ok, Level::Lost) => tr("Sharing · GPS signal lost"),
            (Conn::Ok, Level::Searching) => tr("Sharing · Waiting for GPS"),
            (Conn::Ok, _) => tr("Sharing location"),
        };
        Some(NotificationView { title, body: tr_args("Jastrzab is sharing your location to {server}", &[("server", &host_of(&session.server))]) })
    }

    // ----------------------------------------------------------------------------------------
    // The view

    pub fn view(&self) -> View {
        let in_session = matches!(self.phase, Phase::Connecting | Phase::Active);
        let signal = self.signal.view(in_session);
        let session = self.session.as_ref();
        let host = session.map(|s| host_of(&s.server)).unwrap_or_default();
        let until = |s: &Session| StatusDetail::Until { expires_unix: s.expires_at_unix, host: host.clone() };

        let status = match self.phase {
            Phase::Idle => {
                StatusView { icon: "location-services-disabled-symbolic", class: "", title: tr("Not sharing"), detail: StatusDetail::ChooseDuration }
            }
            Phase::Connecting => StatusView { icon: "network-transmit-receive-symbolic", class: "", title: tr("Connecting…"), detail: StatusDetail::None },
            Phase::Stopping => StatusView { icon: "network-transmit-receive-symbolic", class: "", title: tr("Stopping…"), detail: StatusDetail::None },
            Phase::Active => {
                let detail = session.map_or(StatusDetail::None, until);
                match &self.conn {
                    Conn::Ok => StatusView { icon: "location-services-active-symbolic", class: "success", title: tr("Sharing location"), detail },
                    Conn::Offline => StatusView { icon: "network-offline-symbolic", class: "error", title: tr("No network connection"), detail },
                    Conn::Unreachable => StatusView { icon: "network-offline-symbolic", class: "error", title: tr("Server unreachable"), detail },
                    Conn::Problem(message) => StatusView {
                        icon: "network-offline-symbolic",
                        class: "error",
                        title: tr("Server reported a problem"),
                        detail: StatusDetail::Message(shorten(message, MAX_STATUS_MESSAGE_CHARS)),
                    },
                }
            }
        };

        let links = session
            .map(|s| {
                s.links
                    .iter()
                    .map(|l| LinkView {
                        id: l.id.clone(),
                        url: l.url.clone(),
                        kind: l.kind,
                        subtitle: match l.kind {
                            LinkKind::Own => tr("Your location only"),
                            LinkKind::GroupHost => tr("Group share (host)"),
                            LinkKind::GroupMember => tr("Group share (member)"),
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();

        View {
            phase: match self.phase {
                Phase::Idle => PhaseView::Idle,
                Phase::Connecting => PhaseView::Connecting,
                Phase::Active => PhaseView::Active,
                Phase::Stopping => PhaseView::Stopping,
            },
            status,
            signal,
            primary: match self.phase {
                Phase::Idle => PrimaryButton::Start,
                Phase::Connecting => PrimaryButton::StartDisabled,
                Phase::Active => PrimaryButton::Stop,
                Phase::Stopping => PrimaryButton::StopDisabled,
            },
            form_sensitive: self.phase == Phase::Idle,
            mode: session.map_or(Mode::Alone, |s| s.mode),
            group_pin: session.and_then(|s| s.group_pin.clone()),
            links,
            can_add_link: self.phase == Phase::Active,
            expires_unix: session.map(|s| s.expires_at_unix),
            resume_offer: self
                .resume_offer
                .as_ref()
                .filter(|_| self.phase == Phase::Idle)
                .map(|s| ResumeOffer { links: s.links.len(), expires_unix: s.expires_at_unix }),
            notification: self.last_notification.clone(),
        }
    }
}

enum ControlOk {
    NewLink(response::NewLinkOk),
    Plain,
}

/// Why a request failed, in the words shown to the user.
#[derive(Debug)]
enum Failure {
    Transport(String),
    Hauk(hauk::Error),
}

impl Failure {
    fn text(&self) -> String {
        match self {
            Failure::Transport(message) => message.clone(),
            Failure::Hauk(err) => err.to_string(),
        }
    }

    fn dialog(&self) -> DialogSpec {
        match self {
            Failure::Transport(message) => DialogSpec::error(tr("Connection Error"), message.clone()),
            Failure::Hauk(hauk::Error::Server(message)) => DialogSpec::error(tr("Server Error"), message.clone()),
            Failure::Hauk(hauk::Error::Http(code)) => {
                DialogSpec::error(tr("Connection Error"), tr_args("Received HTTP {code} from the server.", &[("code", &code.to_string())]))
            }
            Failure::Hauk(hauk::Error::Empty) => DialogSpec::error(tr("Server Error"), tr("The server sent an empty response.")),
            Failure::Hauk(hauk::Error::Malformed(what)) => {
                DialogSpec::error(tr("Server Error"), tr_args("The server's response was incomplete: {what}.", &[("what", what)]))
            }
        }
    }
}

/// A server's message can be any length; the status row shows a bounded part of it.
const MAX_STATUS_MESSAGE_CHARS: usize = 160;

/// At most `max` characters of `text`, ending in `…` when cut. Cuts on a character boundary.
fn shorten(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() { format!("{}…", head.trim_end()) } else { head }
}

/// `https://user@host:8443/path/` → `host:8443`.
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit_once('@').map_or(authority, |(_, host)| host).to_string()
}

impl Snapshot {
    /// A snapshot is only worth offering if it has a plausible server and key.
    fn server_base_ok(&self) -> bool {
        !self.server.is_empty() && !self.sid.is_empty() && !self.links.is_empty()
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;
