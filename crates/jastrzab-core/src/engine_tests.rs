//! Scenario tests: drive the engine the way the app does and assert on the effects.

use hauk::request::{Provider, Request, ShareMode};
use hauk::response::Reply;
use hauk::version::Version;

use super::*;
use crate::signal::SignalButton;

struct Harness {
    engine: Engine,
    now: Now,
    next_random: u8,
}

fn settings() -> Settings {
    Settings { server_url: "https://hauk.example.org".into(), interval_secs: 5, ..Settings::default() }
}

fn gps_fix(lat: f64, accuracy_m: f64, unix: f64) -> Fix {
    Fix { latitude: lat, longitude: 13.0, accuracy_m, speed: Some(1.5), description: "GPS GGA".into(), unix_time: unix }
}

fn start_request() -> StartRequest {
    StartRequest {
        duration: 30,
        unit: DurationUnit::Minutes,
        mode: Mode::Alone,
        nickname: String::new(),
        group_pin: String::new(),
        adoptable: true,
        password: "server-secret".into(),
        e2e_password: String::new(),
    }
}

fn reply(version: &str, body: &str) -> Reply {
    Reply { status: 200, version: Version::parse(version), body: body.into() }
}

const CREATE_OK: &str = "OK\nSID-1\nhttps://hauk.example.org/?ABCD-1234\nABCD-1234\n";
const POST_OK: &str = "OK\nhttps://hauk.example.org/?%s\nABCD-1234\n";

impl Harness {
    fn new() -> Self {
        Self::with(settings())
    }

    fn with(settings: Settings) -> Self {
        // Deterministic "random" bytes: 1, 2, 3, ...
        let mut counter = 0u8;
        let random: Box<dyn FnMut(&mut [u8])> = Box::new(move |buf: &mut [u8]| {
            for b in buf {
                counter = counter.wrapping_add(1);
                *b = counter;
            }
        });
        Self { engine: Engine::new(settings, random), now: Now { mono_ms: 10_000, unix: 1_000_000.0 }, next_random: 0 }
    }

    fn send(&mut self, event: Event) -> Vec<Effect> {
        let _ = self.next_random;
        self.engine.handle(event, self.now)
    }

    fn advance(&mut self, secs: u64) {
        self.now.mono_ms += secs * 1000;
        self.now.unix += secs as f64;
    }

    /// The most recent arm generation for `kind` in `effects`.
    fn fire(&mut self, kind: TimerKind, effects: &[Effect]) -> Vec<Effect> {
        let generation = effects
            .iter()
            .rev()
            .find_map(|e| match e {
                Effect::Arm { kind: k, generation, .. } if *k == kind => Some(*generation),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no Arm for {kind:?} in {effects:?}"));
        self.send(Event::Timer { kind, generation })
    }

    /// Start a solo share and bring it to Active with the first reply. Returns the effects of the
    /// last step.
    fn activate(&mut self) -> Vec<Effect> {
        self.send(Event::Start(start_request()));
        let effects = self.send(Event::LocationStarted);
        let id = http_id(&effects);
        self.send(Event::HttpDone { id, result: Ok(reply("1.6.3", CREATE_OK)) })
    }
}

fn http_requests(effects: &[Effect]) -> Vec<(HttpId, &Request)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Http { id, request, .. } => Some((*id, request)),
            _ => None,
        })
        .collect()
}

fn http_id(effects: &[Effect]) -> HttpId {
    let all = http_requests(effects);
    assert_eq!(all.len(), 1, "expected one request in {effects:?}");
    all[0].0
}

fn has(effects: &[Effect], wanted: &Effect) -> bool {
    effects.contains(wanted)
}

fn dialog(effects: &[Effect]) -> &DialogSpec {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::Dialog(d) => Some(d),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no dialog in {effects:?}"))
}

fn toasts(effects: &[Effect]) -> Vec<&Toast> {
    effects.iter().filter_map(|e| if let Effect::Toast(t) = e { Some(t) } else { None }).collect()
}

fn form(request: &Request) -> Vec<(&'static str, String)> {
    request.form()
}

fn field<'a>(form: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
    form.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------------------------------------
// Starting

#[test]
fn starting_without_a_server_explains_and_offers_preferences() {
    let mut h = Harness::with(Settings::default());
    let effects = h.send(Event::Start(start_request()));
    assert!(dialog(&effects).open_preferences);
    assert!(!effects.iter().any(|e| matches!(e, Effect::Location(_) | Effect::Hold(_))));
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn invalid_forms_are_rejected_before_anything_starts() {
    let mut h = Harness::new();
    let mut bad = start_request();
    bad.duration = 0;
    assert_eq!(dialog(&h.send(Event::Start(bad))).heading, "Invalid Duration");

    let mut bad = start_request();
    bad.mode = Mode::CreateGroup;
    assert_eq!(dialog(&h.send(Event::Start(bad.clone()))).heading, "Nickname Required");
    bad.nickname = "Ann".into();
    bad.mode = Mode::JoinGroup;
    assert_eq!(dialog(&h.send(Event::Start(bad))).heading, "Group PIN Required");

    let mut bad = start_request();
    bad.unit = DurationUnit::Days;
    bad.duration = 32;
    assert_eq!(dialog(&h.send(Event::Start(bad))).heading, "Invalid Duration");
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn encryption_needs_a_password_and_a_solo_share() {
    let mut h = Harness::with(Settings { e2e_enabled: true, ..settings() });
    let effects = h.send(Event::Start(start_request()));
    assert_eq!(dialog(&effects).heading, "Encryption Password Needed");
    let mut group = start_request();
    group.mode = Mode::CreateGroup;
    group.nickname = "Ann".into();
    group.e2e_password = "pw".into();
    assert_eq!(dialog(&h.send(Event::Start(group))).heading, "Encryption Unavailable");
}

#[test]
fn a_solo_share_goes_from_connecting_to_active() {
    let mut h = Harness::new();

    let effects = h.send(Event::Start(start_request()));
    assert!(has(&effects, &Effect::Hold(true)));
    assert!(has(&effects, &Effect::Location(LocationCmd::Start { interval_secs: 5 })));
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSettings(s) if s.duration == 30)));
    assert!(http_requests(&effects).is_empty(), "no request before location is ready");
    let view = h.engine.view();
    assert_eq!(view.phase, PhaseView::Connecting);
    assert_eq!(view.primary, PrimaryButton::StartDisabled);
    assert_eq!(view.signal.level, Level::Searching, "the signal row is live from the start");
    assert_eq!(view.signal.button, SignalButton::Hidden);

    let effects = h.send(Event::LocationStarted);
    let (id, request) = http_requests(&effects)[0];
    let form = form(request);
    assert_eq!(request.endpoint(), "api/create.php");
    assert_eq!(field(&form, "pwd"), Some("server-secret"));
    assert_eq!(field(&form, "dur"), Some("1800"));
    assert_eq!(field(&form, "int"), Some("5"));
    assert_eq!(field(&form, "mod"), Some("0"));
    assert_eq!(field(&form, "ado"), Some("1"));
    assert_eq!(field(&form, "e2e"), Some("0"));

    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", CREATE_OK)) });
    assert!(has(&effects, &Effect::Inhibit(true)));
    assert!(effects.iter().any(|e| matches!(e, Effect::Arm { kind: TimerKind::Expiry, secs: 1800, .. })));
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSession(Some(s)) if s.sid == "SID-1")));
    assert_eq!(toasts(&effects)[0], &Toast { title: "Sharing started".into(), copy_link: true });
    assert!(effects.iter().any(|e| matches!(e, Effect::Notify(Some(n)) if n.title == "Sharing · Waiting for GPS" && n.body.contains("hauk.example.org"))));

    let view = h.engine.view();
    assert_eq!(view.phase, PhaseView::Active);
    assert_eq!(view.status.title, "Sharing location");
    assert_eq!(view.primary, PrimaryButton::Stop);
    assert!(!view.form_sensitive && view.can_add_link);
    assert_eq!(view.links.len(), 1);
    assert_eq!(view.links[0].id, "ABCD-1234");
    assert_eq!(view.links[0].subtitle, "Your location only");
    assert_eq!(view.signal.level, Level::Searching);
}

#[test]
fn optional_settings_reach_the_create_request() {
    let mut h = Harness::with(Settings { username: "u".into(), preferred_link_id: "mine".into(), ..settings() });
    h.send(Event::Start(start_request()));
    let effects = h.send(Event::LocationStarted);
    let form = form(http_requests(&effects)[0].1);
    assert_eq!(field(&form, "usr"), Some("u"));
    assert_eq!(field(&form, "lid"), Some("mine"));
}

#[test]
fn a_server_error_on_create_shows_the_servers_message_and_returns_to_idle() {
    let mut h = Harness::new();
    h.send(Event::Start(start_request()));
    let effects = h.send(Event::LocationStarted);
    let id = http_id(&effects);
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "Incorrect password!\n")) });
    let d = dialog(&effects);
    assert_eq!((d.heading.as_str(), d.body.as_str()), ("Server Error", "Incorrect password!"));
    assert!(has(&effects, &Effect::Location(LocationCmd::Stop)));
    assert!(has(&effects, &Effect::Hold(false)));
    let view = h.engine.view();
    assert_eq!(view.phase, PhaseView::Idle);
    assert_eq!(view.signal.level, Level::Off);
}

#[test]
fn a_network_failure_on_create_is_a_connection_error() {
    let mut h = Harness::new();
    h.send(Event::Start(start_request()));
    let id = http_id(&h.send(Event::LocationStarted));
    let effects = h.send(Event::HttpDone { id, result: Err("Could not connect".into()) });
    assert_eq!(dialog(&effects).heading, "Connection Error");
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn location_denied_while_connecting_aborts_before_any_request() {
    let mut h = Harness::new();
    h.send(Event::Start(start_request()));
    let effects = h.send(Event::LocationFailed(LocationFailure::Denied));
    assert_eq!(dialog(&effects).heading, "Location Access Denied");
    assert!(http_requests(&effects).is_empty());
    assert!(has(&effects, &Effect::Hold(false)));
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn an_old_server_downgrades_a_group_share_and_says_so() {
    let mut h = Harness::new();
    let mut req = start_request();
    req.mode = Mode::CreateGroup;
    req.nickname = "Ann".into();
    h.send(Event::Start(req));
    let effects = h.send(Event::LocationStarted);
    let (id, request) = http_requests(&effects)[0];
    assert_eq!(field(&form(request), "mod"), Some("1"));
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.0.2", "OK\nSID\nhttps://h/?X\n")) });
    assert_eq!(dialog(&effects).heading, "Server Is Out of Date");
    let view = h.engine.view();
    assert_eq!(view.phase, PhaseView::Active);
    assert_eq!(view.mode, Mode::Alone);
    assert_eq!(view.group_pin, None);
}

#[test]
fn a_group_share_shows_its_pin_and_host_label() {
    let mut h = Harness::new();
    let mut req = start_request();
    req.mode = Mode::CreateGroup;
    req.nickname = "Ann".into();
    h.send(Event::Start(req));
    let id = http_id(&h.send(Event::LocationStarted));
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\nSID\nhttps://h/?G\n123456\nG-ID\n")) });
    let view = h.engine.view();
    assert_eq!(view.group_pin.as_deref(), Some("123456"));
    assert_eq!(view.links[0].subtitle, "Group share (host)");
    let effects = h.send(Event::CopyPin);
    assert!(has(&effects, &Effect::Clipboard("123456".into())));
}

// ---------------------------------------------------------------------------------------------
// End-to-end encryption

#[test]
fn an_encrypted_share_sends_a_salt_and_encrypts_every_point() {
    let mut h = Harness::with(Settings { e2e_enabled: true, ..settings() });
    let mut req = start_request();
    req.e2e_password = "correct horse".into();
    h.send(Event::Start(req));
    let effects = h.send(Event::LocationStarted);
    let (id, request) = http_requests(&effects)[0];
    let create = form(request);
    assert_eq!(field(&create, "e2e"), Some("1"));
    assert!(field(&create, "salt").is_some());
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", CREATE_OK)) });

    let effects = h.send(Event::Fix(gps_fix(52.5, 3.0, 1_000_000.0)));
    let (_, request) = http_requests(&effects)[0];
    let post = form(request);
    assert!(field(&post, "iv").is_some());
    assert_ne!(field(&post, "lat"), Some("52.5"), "the latitude is ciphertext");
    assert!(field(&post, "lat").unwrap().len() >= 24);
}

#[test]
fn an_old_server_cannot_do_encryption_so_nothing_is_shared() {
    let mut h = Harness::with(Settings { e2e_enabled: true, ..settings() });
    let mut req = start_request();
    req.e2e_password = "pw".into();
    h.send(Event::Start(req));
    let id = http_id(&h.send(Event::LocationStarted));
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.4.0", CREATE_OK)) });
    assert_eq!(dialog(&effects).heading, "Server Is Out of Date");
    let (stop_id, request) = http_requests(&effects)[0];
    assert_eq!(request.endpoint(), "api/stop.php", "the just-created session is ended");
    assert_eq!(h.engine.view().phase, PhaseView::Stopping);
    let effects = h.send(Event::HttpDone { id: stop_id, result: Ok(reply("1.4.0", "OK\n")) });
    assert!(has(&effects, &Effect::Hold(false)));
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

// ---------------------------------------------------------------------------------------------
// Posting

#[test]
fn the_first_fix_is_posted_at_once_with_the_viewer_fields() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::Fix(gps_fix(52.5, 3.0, 1_000_001.25)));
    let (_, request) = http_requests(&effects)[0];
    let Request::Post(post) = request else { panic!("expected a post") };
    assert_eq!((post.sid.as_str(), post.latitude, post.provider), ("SID-1", 52.5, Provider::Fine));
    assert_eq!(post.time, 1_000_001.25, "the fix's own time, not the send time");
    assert_eq!(post.accuracy, Some(3.0));
    assert_eq!(h.engine.view().signal.level, Level::Good);
}

#[test]
fn a_network_fix_is_posted_as_coarse() {
    let mut h = Harness::new();
    h.activate();
    let mut fix = gps_fix(52.5, 80.0, 1.0);
    fix.description = "WiFi".into();
    let effects = h.send(Event::Fix(fix));
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!() };
    assert_eq!(post.provider, Provider::Coarse);
}

#[test]
fn unknown_accuracy_is_not_posted_as_a_number() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::Fix(gps_fix(52.5, -1.0, 1.0)));
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!() };
    assert_eq!(post.accuracy, None);
}

#[test]
fn fixes_between_posts_are_batched_into_the_latest() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    let first = http_id(&effects);
    h.advance(1);
    let a = h.send(Event::Fix(gps_fix(2.0, 3.0, 2.0)));
    assert!(http_requests(&a).is_empty(), "one request in flight at a time");
    h.advance(1);
    h.send(Event::Fix(gps_fix(3.0, 3.0, 3.0)));
    let effects = h.send(Event::HttpDone { id: first, result: Ok(reply("1.6.3", POST_OK)) });
    assert!(http_requests(&effects).is_empty(), "the interval has not passed");
    let wait = effects.iter().find_map(|e| match e {
        Effect::Arm { kind: TimerKind::Send, secs, .. } => Some(*secs),
        _ => None,
    });
    assert_eq!(wait, Some(3), "5 s interval, 2 s used");
    h.advance(3);
    let effects = h.fire(TimerKind::Send, &effects);
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!() };
    assert_eq!(post.latitude, 3.0, "the 2.0 point was superseded and never sent");
}

#[test]
fn a_fix_that_arrives_while_connecting_is_posted_when_the_share_starts() {
    let mut h = Harness::new();
    h.send(Event::Start(start_request()));
    let id = http_id(&h.send(Event::LocationStarted));
    let during = h.send(Event::Fix(gps_fix(9.0, 3.0, 5.0)));
    assert!(http_requests(&during).is_empty());
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", CREATE_OK)) });
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!("the waiting fix goes out at once") };
    assert_eq!(post.latitude, 9.0);
}

#[test]
fn the_minimum_distance_filter_applies_to_posts_not_to_the_signal() {
    let mut h = Harness::with(Settings { min_distance_m: 100.0, ..settings() });
    h.activate();
    let first = http_id(&h.send(Event::Fix(gps_fix(52.0, 3.0, 1.0))));
    h.send(Event::HttpDone { id: first, result: Ok(reply("1.6.3", POST_OK)) });
    h.advance(10);
    let effects = h.send(Event::Fix(gps_fix(52.0001, 0.0, 2.0)));
    assert!(http_requests(&effects).is_empty(), "11 m is under the 100 m minimum");
    assert_eq!(h.engine.view().signal.level, Level::Good, "but the signal row still sees the fix");
}

#[test]
fn a_failed_post_marks_the_server_unreachable_and_retries_with_backoff() {
    let mut h = Harness::new();
    h.activate();
    let first = http_id(&h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0))));
    h.advance(1);
    let effects = h.send(Event::HttpDone { id: first, result: Err("timed out".into()) });
    assert!(effects.iter().any(|e| matches!(e, Effect::Arm { kind: TimerKind::Send, .. })));
    assert_eq!(h.engine.view().status.title, "Server unreachable");
    assert!(effects.iter().any(|e| matches!(e, Effect::Notify(Some(n)) if n.title == "Sharing · Server unreachable")));

    h.advance(10);
    let effects = h.fire(TimerKind::Send, &effects);
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!() };
    assert_eq!(post.latitude, 1.0, "the same fix is retried");
    let id = http_id(&effects);
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", POST_OK)) });
    assert_eq!(h.engine.view().status.title, "Sharing location");
}

#[test]
fn a_server_message_on_post_is_shown_in_the_status_row() {
    let mut h = Harness::new();
    h.activate();
    let id = http_id(&h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0))));
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "Session expired!\n")) });
    let status = h.engine.view().status;
    assert_eq!(status.title, "Server reported a problem");
    assert_eq!(status.detail, StatusDetail::Message("Session expired!".into()));
}

#[test]
fn going_offline_pauses_posting_and_coming_back_retries_at_once() {
    let mut h = Harness::new();
    h.activate();
    h.send(Event::Network(false));
    assert_eq!(h.engine.view().status.title, "No network connection");
    let effects = h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    assert!(http_requests(&effects).is_empty(), "nothing is sent offline");
    let effects = h.send(Event::Network(true));
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!("retry on reconnect") };
    assert_eq!(post.latitude, 1.0);
}

#[test]
fn the_link_list_follows_the_server() {
    let mut h = Harness::new();
    h.activate();
    let id = http_id(&h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0))));
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\nhttps://hauk.example.org/?%s\nABCD-1234,GRP-1\n")) });
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistSession(Some(s)) if s.links.len() == 2)));
    let view = h.engine.view();
    assert_eq!(view.links.len(), 2);
    assert_eq!(view.links[1].subtitle, "Group share (member)");
}

// ---------------------------------------------------------------------------------------------
// Links

#[test]
fn a_new_link_is_requested_in_order_and_added() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::NewLink { adoptable: false });
    let (id, request) = http_requests(&effects)[0];
    assert_eq!(request.endpoint(), "api/new-link.php");
    assert_eq!(field(&form(request), "ado"), Some("0"));
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\nhttps://hauk.example.org/?N-2\nN-2\n")) });
    assert_eq!(toasts(&effects)[0].title, "Link created");
    assert_eq!(h.engine.view().links.len(), 2);
}

#[test]
fn control_requests_wait_for_a_post_in_flight() {
    let mut h = Harness::new();
    h.activate();
    let post = http_id(&h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0))));
    let effects = h.send(Event::NewLink { adoptable: true });
    assert!(http_requests(&effects).is_empty());
    let effects = h.send(Event::HttpDone { id: post, result: Ok(reply("1.6.3", POST_OK)) });
    assert_eq!(http_requests(&effects)[0].1.endpoint(), "api/new-link.php");
}

#[test]
fn stopping_one_of_several_links_sends_its_id() {
    let mut h = Harness::new();
    h.activate();
    let id = http_id(&h.send(Event::NewLink { adoptable: true }));
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\nhttps://h/?N-2\nN-2\n")) });
    let effects = h.send(Event::StopLink("N-2".into()));
    let (id, request) = http_requests(&effects)[0];
    assert_eq!(field(&form(request), "lid"), Some("N-2"));
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\n")) });
    assert_eq!(h.engine.view().links.len(), 1);
}

#[test]
fn stopping_the_last_link_stops_the_share() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::StopLink("ABCD-1234".into()));
    let request = http_requests(&effects)[0].1;
    assert_eq!(request.endpoint(), "api/stop.php");
    assert_eq!(field(&form(request), "lid"), None, "the whole session");
}

#[test]
fn adoption_needs_a_group_and_reports_the_nickname() {
    let mut h = Harness::new();
    h.activate();
    let none = h.send(Event::Adopt { target: "T".into(), nickname: "Bo".into() });
    assert!(http_requests(&none).is_empty(), "a solo share has no group to adopt into");

    let mut h = Harness::new();
    let mut req = start_request();
    req.mode = Mode::CreateGroup;
    req.nickname = "Ann".into();
    h.send(Event::Start(req));
    let id = http_id(&h.send(Event::LocationStarted));
    h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\nSID\nhttps://h/?G\n123456\nG-ID\n")) });
    let effects = h.send(Event::Adopt { target: "T-9".into(), nickname: "Bo".into() });
    let (id, request) = http_requests(&effects)[0];
    let adopt = form(request);
    assert_eq!((field(&adopt, "aid"), field(&adopt, "pin"), field(&adopt, "nic")), (Some("T-9"), Some("123456"), Some("Bo")));
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\n")) });
    assert_eq!(toasts(&effects)[0].title, "Bo has been added to your share!");
}

#[test]
fn a_failed_control_request_shows_a_dialog_and_does_not_back_off_posts() {
    let mut h = Harness::new();
    h.activate();
    let id = http_id(&h.send(Event::NewLink { adoptable: true }));
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "Too many links!\n")) });
    assert_eq!(dialog(&effects).body, "Too many links!");
    let effects = h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    assert!(!http_requests(&effects).is_empty(), "posting is unaffected");
}

#[test]
fn copying_a_link_uses_the_clipboard() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::CopyLink(None));
    assert!(has(&effects, &Effect::Clipboard("https://hauk.example.org/?ABCD-1234".into())));
    assert_eq!(toasts(&effects)[0].title, "Link copied");
}

// ---------------------------------------------------------------------------------------------
// Stopping

#[test]
fn stopping_ends_the_session_on_the_server_and_releases_everything() {
    let mut h = Harness::new();
    h.activate();
    h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    let effects = h.send(Event::Stop);
    assert_eq!(http_requests(&effects)[0].1.endpoint(), "api/stop.php");
    assert!(has(&effects, &Effect::Inhibit(false)));
    assert!(has(&effects, &Effect::PersistSession(None)));
    assert!(has(&effects, &Effect::Notify(None)));
    assert!(!has(&effects, &Effect::Hold(false)), "the process stays until the server has been told");
    assert_eq!(h.engine.view().phase, PhaseView::Stopping);
    assert_eq!(h.engine.view().primary, PrimaryButton::StopDisabled);

    let id = http_id(&effects);
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", "OK\n")) });
    assert!(has(&effects, &Effect::Location(LocationCmd::Stop)));
    assert!(has(&effects, &Effect::Hold(false)));
    let view = h.engine.view();
    assert_eq!((view.phase, view.signal.level), (PhaseView::Idle, Level::Off));
    assert!(view.form_sensitive);
}

#[test]
fn a_failed_stop_still_ends_locally_and_says_so() {
    let mut h = Harness::new();
    h.activate();
    let id = http_id(&h.send(Event::Stop));
    let effects = h.send(Event::HttpDone { id, result: Err("unreachable".into()) });
    assert!(toasts(&effects)[0].title.starts_with("Couldn't tell the server"));
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn stopping_offline_does_not_wait_for_a_request() {
    let mut h = Harness::new();
    h.activate();
    h.send(Event::Network(false));
    let effects = h.send(Event::Stop);
    assert!(http_requests(&effects).is_empty());
    assert!(has(&effects, &Effect::Hold(false)));
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn the_share_ends_by_itself_without_a_request_when_its_time_is_up() {
    let mut h = Harness::new();
    let effects = h.activate();
    h.advance(1800);
    let effects = h.fire(TimerKind::Expiry, &effects);
    assert!(http_requests(&effects).is_empty(), "the server expires it itself");
    assert_eq!(toasts(&effects)[0].title, "Sharing has ended");
    assert!(has(&effects, &Effect::PersistSession(None)));
    assert!(has(&effects, &Effect::Hold(false)));
    assert_eq!(h.engine.view().phase, PhaseView::Idle);
}

#[test]
fn stale_timers_are_ignored() {
    let mut h = Harness::new();
    let effects = h.activate();
    let stale = effects.iter().find_map(|e| if let Effect::Arm { kind: TimerKind::Expiry, generation, .. } = e { Some(*generation) } else { None }).unwrap();
    let out = h.send(Event::Timer { kind: TimerKind::Expiry, generation: stale + 5 });
    assert!(out.is_empty());
    assert_eq!(h.engine.view().phase, PhaseView::Active);
}

#[test]
fn stop_and_new_link_do_nothing_when_idle() {
    let mut h = Harness::new();
    assert!(h.send(Event::Stop).is_empty());
    assert!(h.send(Event::NewLink { adoptable: true }).is_empty());
}

// ---------------------------------------------------------------------------------------------
// Resuming

fn snapshot(expires: f64) -> Snapshot {
    Snapshot {
        format: Snapshot::FORMAT,
        sid: "OLD".into(),
        server: "https://hauk.example.org/".into(),
        mode: Mode::Alone,
        expires_at_unix: expires,
        interval_secs: 5,
        group_pin: None,
        e2e_key: None,
        link_format: None,
        links: vec![Link { id: "OLD-1".into(), url: "https://hauk.example.org/?OLD-1".into(), kind: LinkKind::Own }],
    }
}

#[test]
fn an_unfinished_session_is_offered_and_can_be_resumed_without_creating_a_new_one() {
    let mut h = Harness::new();
    h.send(Event::ResumeAvailable(snapshot(1_000_600.0)));
    assert_eq!(h.engine.view().resume_offer.as_ref().map(|o| o.links), Some(1));

    let effects = h.send(Event::Resume);
    assert!(has(&effects, &Effect::Location(LocationCmd::Start { interval_secs: 5 })));
    let effects = h.send(Event::LocationStarted);
    assert!(http_requests(&effects).is_empty(), "no create.php on resume");
    assert!(effects.iter().any(|e| matches!(e, Effect::Arm { kind: TimerKind::Expiry, secs: 600, .. })));
    assert!(toasts(&effects).is_empty(), "silent, like the original");
    let view = h.engine.view();
    assert_eq!(view.phase, PhaseView::Active);
    assert_eq!(view.resume_offer, None);

    let effects = h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    let Request::Post(post) = http_requests(&effects)[0].1 else { panic!() };
    assert_eq!(post.sid, "OLD");
}

#[test]
fn an_expired_session_is_discarded_and_dismissing_deletes_the_file() {
    let mut h = Harness::new();
    let effects = h.send(Event::ResumeAvailable(snapshot(999_000.0)));
    assert!(has(&effects, &Effect::PersistSession(None)));
    assert_eq!(h.engine.view().resume_offer, None);

    h.send(Event::ResumeAvailable(snapshot(1_000_600.0)));
    let effects = h.send(Event::DismissResume);
    assert!(has(&effects, &Effect::PersistSession(None)));
    assert_eq!(h.engine.view().resume_offer, None);
}

// ---------------------------------------------------------------------------------------------
// The GPS signal

#[test]
fn checking_the_signal_before_sharing_starts_and_stops_location() {
    let mut h = Harness::new();
    assert_eq!(h.engine.view().signal.button, SignalButton::Check);
    let effects = h.send(Event::CheckSignal(true));
    assert!(has(&effects, &Effect::Location(LocationCmd::Start { interval_secs: 1 })));
    assert!(effects.iter().any(|e| matches!(e, Effect::Arm { kind: TimerKind::CheckLimit, secs: 300, .. })));
    assert_eq!(h.engine.view().signal.level, Level::Searching);
    assert_eq!(h.engine.view().signal.button, SignalButton::Stop);

    let effects = h.send(Event::Fix(gps_fix(1.0, 0.0, 1.0)));
    assert!(http_requests(&effects).is_empty(), "a check never posts anything");
    assert_eq!(h.engine.view().signal.level, Level::Excellent);

    let effects = h.send(Event::CheckSignal(false));
    assert!(has(&effects, &Effect::Location(LocationCmd::Stop)));
    assert_eq!(h.engine.view().signal.level, Level::Off);
}

#[test]
fn hiding_the_window_or_waiting_five_minutes_ends_the_check() {
    let mut h = Harness::new();
    h.send(Event::CheckSignal(true));
    let effects = h.send(Event::WindowVisible(false));
    assert!(has(&effects, &Effect::Location(LocationCmd::Stop)));
    assert_eq!(h.engine.view().signal.level, Level::Off);

    let effects = h.send(Event::CheckSignal(true));
    let effects = h.fire(TimerKind::CheckLimit, &effects);
    assert!(has(&effects, &Effect::Location(LocationCmd::Stop)));
}

#[test]
fn starting_a_share_after_a_check_keeps_the_fix_the_user_already_has() {
    let mut h = Harness::new();
    h.send(Event::CheckSignal(true));
    h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    let effects = h.send(Event::Start(start_request()));
    assert!(has(&effects, &Effect::Location(LocationCmd::Start { interval_secs: 5 })), "retuned to the share interval");
    assert_eq!(h.engine.view().signal.level, Level::Good);
    let effects = h.send(Event::LocationStarted);
    let id = http_id(&effects);
    let effects = h.send(Event::HttpDone { id, result: Ok(reply("1.6.3", CREATE_OK)) });
    assert!(!http_requests(&effects).is_empty(), "the fix from the check is posted straight away");
}

#[test]
fn the_signal_row_is_never_off_while_connecting_or_sharing() {
    let mut h = Harness::new();
    h.send(Event::Start(start_request()));
    assert_ne!(h.engine.view().signal.level, Level::Off);
    h.send(Event::LocationStarted);
    assert_ne!(h.engine.view().signal.summary, "Not checking");
    h.activate_from_connecting();
    assert_ne!(h.engine.view().signal.level, Level::Off);
}

impl Harness {
    fn activate_from_connecting(&mut self) {
        let id = self.engine_create_id();
        self.send(Event::HttpDone { id, result: Ok(reply("1.6.3", CREATE_OK)) });
    }

    fn engine_create_id(&self) -> HttpId {
        self.engine.create_id.expect("create in flight")
    }
}

#[test]
fn losing_the_signal_is_shown_once_and_sharing_goes_on() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    let lost_timer = effects.iter().find_map(|e| if let Effect::Arm { kind: TimerKind::SignalLost, secs, .. } = e { Some(*secs) } else { None });
    assert_eq!(lost_timer, Some(15), "3 x the 5 s interval");

    h.advance(15);
    let effects = h.fire(TimerKind::SignalLost, &effects);
    assert_eq!(toasts(&effects)[0].title, "GPS signal lost");
    assert!(effects.iter().any(|e| matches!(e, Effect::Notify(Some(n)) if n.title == "Sharing · GPS signal lost")));
    let view = h.engine.view();
    assert_eq!(view.signal.level, Level::Lost);
    assert_eq!(view.phase, PhaseView::Active);

    // A new fix brings it back, and the notification follows.
    let effects = h.send(Event::Fix(gps_fix(1.0, 3.0, 20.0)));
    assert_eq!(h.engine.view().signal.level, Level::Good);
    assert!(effects.iter().any(|e| matches!(e, Effect::Notify(Some(n)) if n.title == "Sharing location")));
}

#[test]
fn the_notification_is_only_sent_when_it_changes() {
    let mut h = Harness::new();
    h.activate();
    let first = h.send(Event::Fix(gps_fix(1.0, 3.0, 1.0)));
    h.advance(5);
    let second = h.send(Event::Fix(gps_fix(1.0001, 3.0, 2.0)));
    let notifies = |e: &[Effect]| e.iter().filter(|e| matches!(e, Effect::Notify(_))).count();
    assert_eq!(notifies(&first), 1, "Waiting for GPS becomes Sharing location");
    assert_eq!(notifies(&second), 0);
}

#[test]
fn losing_location_mid_share_tells_the_user_but_keeps_the_share() {
    let mut h = Harness::new();
    h.activate();
    let effects = h.send(Event::LocationFailed(LocationFailure::Unavailable("GeoClue stopped".into())));
    assert!(dialog(&effects).body.contains("GeoClue stopped"));
    assert_eq!(h.engine.view().phase, PhaseView::Active);
}

#[test]
fn a_denied_check_stops_cleanly() {
    let mut h = Harness::new();
    h.send(Event::CheckSignal(true));
    let effects = h.send(Event::LocationFailed(LocationFailure::Denied));
    assert_eq!(dialog(&effects).heading, "Location Access Denied");
    assert_eq!(h.engine.view().signal.level, Level::Off);
}

#[test]
fn only_one_share_at_a_time() {
    let mut h = Harness::new();
    h.activate();
    assert!(h.send(Event::Start(start_request())).is_empty());
}

#[test]
fn share_mode_maps_to_the_protocol() {
    assert_eq!(Mode::CreateGroup.share_mode(), ShareMode::CreateGroup);
}
