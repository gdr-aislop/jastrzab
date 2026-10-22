//! The GPS signal indicator (design §4.4): whether the phone has a fix, how good it is, and what
//! that means for the people watching.
//!
//! GeoClue reports only an accuracy in metres, a free-text description of the source and a
//! timestamp: no fix type and no satellite count. For satellite fixes the accuracy is not a
//! measurement but one of GeoClue's own buckets derived from the NMEA HDOP (0, 1, 3, 50, 100 or
//! 300 m), so the cut-offs sit on those buckets and cannot be finer. The thresholds and times here
//! are proposals that need calibrating on a device.

use std::collections::VecDeque;

use crate::geo::Fix;
use crate::i18n::{tr, tr_args};

/// How many recent accuracies the median is taken over.
const MEDIAN_WINDOW: usize = 5;
/// A better level is shown after this many consecutive better fixes.
const IMPROVE_AFTER_FIXES: u32 = 2;
/// A worse level is shown after it has persisted this long.
const DEGRADE_AFTER_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Level {
    /// Not monitoring (before sharing, and the user has not asked for a check).
    Off,
    /// Monitoring, no fix yet.
    Searching,
    Poor,
    Fair,
    Good,
    Excellent,
    /// Had a fix, but no update for a while.
    Lost,
    /// Location is switched off on purpose while the network is gone, to save battery.
    Paused,
}

impl Level {
    /// Filled segments of the four-segment bar.
    pub fn bars(self) -> u8 {
        match self {
            Level::Poor => 1,
            Level::Fair => 2,
            Level::Good => 3,
            Level::Excellent => 4,
            Level::Off | Level::Searching | Level::Lost | Level::Paused => 0,
        }
    }

    /// The CSS class that colours the bar.
    pub fn css_class(self) -> &'static str {
        match self {
            Level::Poor => "q-poor",
            Level::Fair => "q-fair",
            Level::Good => "q-good",
            Level::Excellent => "q-excellent",
            Level::Off | Level::Searching | Level::Lost | Level::Paused => "q-none",
        }
    }

    /// Whether this level comes from a measured fix.
    pub fn is_measured(self) -> bool {
        matches!(self, Level::Poor | Level::Fair | Level::Good | Level::Excellent)
    }

    fn rank(self) -> i8 {
        match self {
            Level::Poor => 1,
            Level::Fair => 2,
            Level::Good => 3,
            Level::Excellent => 4,
            _ => 0,
        }
    }
}

/// Maps a GeoClue accuracy (metres) to a level. GeoClue's buckets are 0, 1, 3, 50, 100 and 300 m
/// for HDOP ≤ 1, 2, 5, 10, 20 and above, so ≤ 1 m is HDOP ≤ 2 (excellent), ≤ 5 m is HDOP ≤ 5
/// (good), and ≤ 100 m is HDOP ≤ 20 (fair). Unknown (negative) is poor.
pub fn level_for(accuracy_m: f64) -> Level {
    if accuracy_m < 0.0 {
        Level::Poor
    } else if accuracy_m <= 1.0 {
        Level::Excellent
    } else if accuracy_m <= 5.0 {
        Level::Good
    } else if accuracy_m <= 100.0 {
        Level::Fair
    } else {
        Level::Poor
    }
}

/// "Precise" below 10 m (GeoClue's number is a coarse bucket there, not a measurement), otherwise
/// "About N m" rounded to 5 m; "Unknown" when there is no value.
pub fn accuracy_text(accuracy_m: f64) -> String {
    if accuracy_m < 0.0 {
        tr("Unknown")
    } else if accuracy_m < 10.0 {
        tr("Precise")
    } else {
        tr_args("About {m} m", &[("m", &(((accuracy_m / 5.0).round() as i64) * 5).to_string())])
    }
}

fn median(values: &VecDeque<f64>) -> f64 {
    let mut sorted: Vec<f64> = values.iter().copied().collect();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

/// What the monitor decided when asked whether the signal has gone quiet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostCheck {
    /// Nothing to watch (no fix yet, or already lost).
    Nothing,
    /// Not lost yet; check again after this many seconds.
    Recheck(u32),
    BecameLost,
}

#[derive(Debug, Clone)]
pub struct SignalMonitor {
    active: bool,
    interval_secs: u32,
    recent: VecDeque<f64>,
    shown: Level,
    better_fixes: u32,
    worse_since_ms: Option<u64>,
    last_fix_ms: Option<u64>,
    last_fix_gps: bool,
    last_accuracy: f64,
    lost_age_secs: u64,
}

impl Default for SignalMonitor {
    fn default() -> Self {
        Self {
            active: false,
            interval_secs: 5,
            recent: VecDeque::with_capacity(MEDIAN_WINDOW),
            shown: Level::Off,
            better_fixes: 0,
            worse_since_ms: None,
            last_fix_ms: None,
            last_fix_gps: true,
            last_accuracy: -1.0,
            lost_age_secs: 0,
        }
    }
}

impl SignalMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn level(&self) -> Level {
        self.shown
    }

    /// Begins monitoring at [`Level::Searching`]. A monitor that is already running keeps its
    /// state (a user pressing Check and then starting a share keeps the fix they already have).
    pub fn start(&mut self, interval_secs: u32) {
        self.interval_secs = interval_secs.max(1);
        if self.active {
            return;
        }
        *self = Self { active: true, interval_secs: self.interval_secs, shown: Level::Searching, ..Self::default() };
    }

    pub fn stop(&mut self) {
        *self = Self::default();
    }

    /// Location was switched off because the network is gone: show that instead of "lost", and
    /// ignore any update still on its way.
    pub fn pause(&mut self) {
        if self.active {
            self.shown = Level::Paused;
        }
    }

    /// Location was switched back on: searching again, as if the share had just begun.
    pub fn resume(&mut self) {
        if self.shown == Level::Paused {
            *self = Self { active: true, interval_secs: self.interval_secs, shown: Level::Searching, ..Self::default() };
        }
    }

    /// How long without an update means the signal is lost: `max(3 × interval, 10 s)` for
    /// satellite fixes. Network sources only report when the position changes, so a stationary
    /// phone is quiet for long stretches and gets a much longer window.
    pub fn lost_after_secs(&self) -> u32 {
        if self.last_fix_gps { (3 * self.interval_secs).max(10) } else { (12 * self.interval_secs).max(120) }
    }

    pub fn has_fix(&self) -> bool {
        self.last_fix_ms.is_some()
    }

    pub fn on_fix(&mut self, fix: &Fix, now_ms: u64) {
        if !self.active || self.shown == Level::Paused {
            return;
        }
        if self.recent.len() == MEDIAN_WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(fix.accuracy_m);
        let smoothed = median(&self.recent);
        let raw = level_for(smoothed);
        self.last_fix_ms = Some(now_ms);
        self.last_fix_gps = fix.is_gps();
        self.last_accuracy = smoothed;

        match self.shown {
            Level::Off | Level::Searching | Level::Lost => {
                self.shown = raw;
                self.better_fixes = 0;
                self.worse_since_ms = None;
            }
            current => {
                if raw == current {
                    self.better_fixes = 0;
                    self.worse_since_ms = None;
                } else if raw.rank() > current.rank() {
                    self.worse_since_ms = None;
                    self.better_fixes += 1;
                    if self.better_fixes >= IMPROVE_AFTER_FIXES {
                        self.shown = raw;
                        self.better_fixes = 0;
                    }
                } else {
                    self.better_fixes = 0;
                    let since = *self.worse_since_ms.get_or_insert(now_ms);
                    if now_ms.saturating_sub(since) >= DEGRADE_AFTER_MS {
                        self.shown = raw;
                        self.worse_since_ms = None;
                    }
                }
            }
        }
    }

    pub fn check_lost(&mut self, now_ms: u64) -> LostCheck {
        let Some(last) = self.last_fix_ms else { return LostCheck::Nothing };
        if !self.active || !self.shown.is_measured() {
            return LostCheck::Nothing;
        }
        let after_ms = u64::from(self.lost_after_secs()) * 1000;
        let elapsed = now_ms.saturating_sub(last);
        if elapsed >= after_ms {
            self.shown = Level::Lost;
            self.lost_age_secs = elapsed / 1000;
            LostCheck::BecameLost
        } else {
            LostCheck::Recheck(((after_ms - elapsed).div_ceil(1000)).max(1) as u32)
        }
    }

    /// Everything the UI shows for the current state. `in_session` is whether a share is running:
    /// then there is no Check/Stop button and the "what viewers see" row states a fact rather
    /// than a preview.
    pub fn view(&self, in_session: bool) -> SignalView {
        let level = if in_session && self.shown == Level::Off { Level::Searching } else { self.shown };
        let measured = level.is_measured();
        let gps = self.last_fix_gps;

        let (title, viewers, hint) = match level {
            Level::Off => (tr("Not checking"), None, Some(tr("Check the signal before you start, so viewers see you in the right place."))),
            Level::Searching => (
                tr("Searching for GPS…"),
                Some(tr("Nobody can see you yet.")),
                Some(tr("Go outside or stand near a window. Roofs, walls and vehicles block GPS. Keep this screen open.")),
            ),
            Level::Poor => (tr("Poor"), Some(tr("Viewers will only see your rough area.")), Some(tr("Move to open sky and wait a moment for a better fix."))),
            Level::Fair => (tr("Fair"), Some(tr("Viewers may see you on the wrong street.")), Some(tr("A clearer view of the sky will improve this."))),
            Level::Good => (tr("Good"), Some(tr("Viewers will see you at the right building.")), None),
            Level::Excellent => (tr("Excellent"), Some(tr("Viewers will see you in the right spot.")), None),
            Level::Paused => (
                tr("Paused"),
                Some(tr("Viewers see your last known spot.")),
                Some(tr("There is no network, so location is off to save battery. It starts again when the network is back.")),
            ),
            Level::Lost => (
                tr("Signal lost"),
                Some(tr("Viewers see your last known spot.")),
                Some(tr("Move to open sky. Sharing continues and resumes when a signal returns.")),
            ),
        };

        let accuracy = accuracy_text(self.last_accuracy);
        let summary = if measured && !gps {
            tr_args("No GPS fix · network location, {accuracy}", &[("accuracy", &accuracy.to_lowercase())])
        } else if measured && self.last_accuracy >= 10.0 {
            format!("{title} · {}", accuracy.to_lowercase())
        } else if level == Level::Lost {
            tr_args("{title} · last seen {s} s ago", &[("title", &title), ("s", &self.lost_age_secs.to_string())])
        } else {
            title.clone()
        };

        let (icon, icon_class) = match level {
            Level::Excellent | Level::Good => ("location-services-active-symbolic", "success"),
            Level::Fair | Level::Poor => ("location-services-active-symbolic", "warning"),
            Level::Searching => ("find-location-symbolic", "warning"),
            Level::Lost => ("location-services-disabled-symbolic", "error"),
            Level::Paused => ("location-services-disabled-symbolic", "dim-label"),
            Level::Off => ("find-location-symbolic", "dim-label"),
        };

        let button = match (in_session, level) {
            (true, _) => SignalButton::Hidden,
            (false, Level::Off) => SignalButton::Check,
            (false, _) => SignalButton::Stop,
        };

        SignalView {
            level,
            bars: level.bars(),
            css_class: level.css_class(),
            description: tr_args("GPS signal: {summary}", &[("summary", &summary)]),
            accuracy: measured.then(
                || {
                    if self.last_accuracy >= 10.0 { tr_args("{accuracy} (estimate)", &[("accuracy", &accuracy)]) } else { accuracy.clone() }
                },
            ),
            source: measured.then(|| if gps { tr("Satellites (GPS)") } else { tr("Wi-Fi or mobile network (approximate)") }),
            last_update: if measured {
                Some(tr("Just now"))
            } else if level == Level::Lost {
                Some(tr_args("{s} seconds ago", &[("s", &self.lost_age_secs.to_string())]))
            } else {
                None
            },
            viewers_title: if in_session { tr("What viewers see") } else { tr("If you share now") },
            viewers,
            hint,
            summary,
            icon,
            icon_class,
            button,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalButton {
    Hidden,
    Check,
    Stop,
}

/// Everything needed to draw the GPS signal row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalView {
    pub level: Level,
    pub bars: u8,
    pub css_class: &'static str,
    /// The row's subtitle: "Fair · about 50 m".
    pub summary: String,
    /// The row's accessible description. Changes with the level, not every second.
    pub description: String,
    pub icon: &'static str,
    pub icon_class: &'static str,
    pub button: SignalButton,
    pub accuracy: Option<String>,
    pub source: Option<String>,
    pub last_update: Option<String>,
    /// "What viewers see" while sharing, "If you share now" before.
    pub viewers_title: String,
    pub viewers: Option<String>,
    pub hint: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(accuracy_m: f64, description: &str) -> Fix {
        Fix { latitude: 0.0, longitude: 0.0, accuracy_m, speed: None, description: description.into(), unix_time: 0.0 }
    }

    fn gps(accuracy_m: f64) -> Fix {
        fix(accuracy_m, "GPS GGA")
    }

    #[test]
    fn level_follows_geoclue_buckets() {
        for (accuracy, level) in [
            (-1.0, Level::Poor),
            (0.0, Level::Excellent),
            (1.0, Level::Excellent),
            (3.0, Level::Good),
            (50.0, Level::Fair),
            (100.0, Level::Fair),
            (300.0, Level::Poor),
        ] {
            assert_eq!(level_for(accuracy), level, "{accuracy} m");
        }
    }

    #[test]
    fn accuracy_text_rounds_and_hides_buckets() {
        assert_eq!(accuracy_text(-1.0), "Unknown");
        assert_eq!(accuracy_text(0.0), "Precise");
        assert_eq!(accuracy_text(3.0), "Precise");
        assert_eq!(accuracy_text(50.0), "About 50 m");
        assert_eq!(accuracy_text(82.0), "About 80 m");
        assert_eq!(accuracy_text(300.0), "About 300 m");
    }

    #[test]
    fn a_stopped_monitor_is_off_and_ignores_fixes() {
        let mut m = SignalMonitor::new();
        m.on_fix(&gps(3.0), 0);
        assert_eq!(m.level(), Level::Off);
        assert_eq!(m.view(false).button, SignalButton::Check);
    }

    #[test]
    fn starting_searches_and_the_first_fix_is_shown_at_once() {
        let mut m = SignalMonitor::new();
        m.start(5);
        assert_eq!(m.level(), Level::Searching);
        m.on_fix(&gps(3.0), 1_000);
        assert_eq!(m.level(), Level::Good);
    }

    #[test]
    fn improving_needs_two_consecutive_better_fixes() {
        let mut m = SignalMonitor::new();
        m.start(1);
        m.on_fix(&gps(50.0), 0);
        assert_eq!(m.level(), Level::Fair);
        // The median of [50, 3] is 50 (upper median), so feed enough good fixes to move it.
        m.on_fix(&gps(3.0), 1_000);
        m.on_fix(&gps(3.0), 2_000);
        m.on_fix(&gps(3.0), 3_000);
        assert_eq!(m.level(), Level::Good);
    }

    #[test]
    fn degrading_needs_three_seconds() {
        let mut m = SignalMonitor::new();
        m.start(1);
        for t in 0..5 {
            m.on_fix(&gps(0.0), t * 1000);
        }
        assert_eq!(m.level(), Level::Excellent);
        // Three bad fixes make the median bad; the level holds until 3 s have passed.
        m.on_fix(&gps(300.0), 5_000);
        m.on_fix(&gps(300.0), 6_000);
        m.on_fix(&gps(300.0), 7_000);
        assert_eq!(m.level(), Level::Excellent, "the worse level has lasted 0 s");
        m.on_fix(&gps(300.0), 10_500);
        assert_eq!(m.level(), Level::Poor);
    }

    #[test]
    fn a_single_bad_fix_does_not_flicker_the_bar() {
        let mut m = SignalMonitor::new();
        m.start(1);
        for t in 0..5 {
            m.on_fix(&gps(3.0), t * 1000);
        }
        m.on_fix(&gps(300.0), 5_000);
        m.on_fix(&gps(3.0), 6_000);
        assert_eq!(m.level(), Level::Good);
    }

    #[test]
    fn silence_means_lost_after_three_intervals_but_never_below_ten_seconds() {
        let mut m = SignalMonitor::new();
        m.start(1);
        m.on_fix(&gps(3.0), 0);
        assert_eq!(m.lost_after_secs(), 10);
        assert_eq!(m.check_lost(4_000), LostCheck::Recheck(6));
        assert_eq!(m.check_lost(10_000), LostCheck::BecameLost);
        assert_eq!(m.level(), Level::Lost);
        assert_eq!(m.check_lost(20_000), LostCheck::Nothing);

        let mut slow = SignalMonitor::new();
        slow.start(10);
        slow.on_fix(&gps(3.0), 0);
        assert_eq!(slow.lost_after_secs(), 30);
    }

    #[test]
    fn a_fix_after_lost_recovers_immediately() {
        let mut m = SignalMonitor::new();
        m.start(1);
        m.on_fix(&gps(3.0), 0);
        m.check_lost(11_000);
        m.on_fix(&gps(50.0), 12_000);
        assert_eq!(m.level(), Level::Fair);
    }

    #[test]
    fn nothing_is_lost_before_the_first_fix() {
        let mut m = SignalMonitor::new();
        m.start(1);
        assert_eq!(m.check_lost(60_000), LostCheck::Nothing);
        assert_eq!(m.level(), Level::Searching);
    }

    #[test]
    fn network_sources_get_a_longer_window_and_say_so() {
        let mut m = SignalMonitor::new();
        m.start(5);
        m.on_fix(&fix(80.0, "WiFi"), 0);
        assert_eq!(m.lost_after_secs(), 120);
        let v = m.view(false);
        assert_eq!(v.level, Level::Fair);
        assert_eq!(v.summary, "No GPS fix · network location, about 80 m");
        assert_eq!(v.source.as_deref(), Some("Wi-Fi or mobile network (approximate)"));
    }

    #[test]
    fn view_texts_for_a_gps_fix() {
        let mut m = SignalMonitor::new();
        m.start(5);
        m.on_fix(&gps(50.0), 0);
        let v = m.view(true);
        assert_eq!((v.level, v.bars, v.css_class), (Level::Fair, 2, "q-fair"));
        assert_eq!(v.summary, "Fair · about 50 m");
        assert_eq!(v.description, "GPS signal: Fair · about 50 m");
        assert_eq!(v.accuracy.as_deref(), Some("About 50 m (estimate)"));
        assert_eq!(v.source.as_deref(), Some("Satellites (GPS)"));
        assert_eq!(v.viewers_title, "What viewers see");
        assert_eq!(v.button, SignalButton::Hidden);

        let mut m = SignalMonitor::new();
        m.start(5);
        m.on_fix(&gps(0.0), 0);
        let v = m.view(false);
        assert_eq!(v.summary, "Excellent");
        assert_eq!(v.accuracy.as_deref(), Some("Precise"));
        assert_eq!(v.viewers_title, "If you share now");
        assert_eq!(v.button, SignalButton::Stop);
    }

    #[test]
    fn lost_view_names_the_age() {
        let mut m = SignalMonitor::new();
        m.start(1);
        m.on_fix(&gps(3.0), 0);
        m.check_lost(12_000);
        let v = m.view(true);
        assert_eq!(v.summary, "Signal lost · last seen 12 s ago");
        assert_eq!(v.last_update.as_deref(), Some("12 seconds ago"));
        assert_eq!(v.bars, 0);
    }

    #[test]
    fn the_row_is_never_off_in_a_session() {
        let m = SignalMonitor::new();
        let v = m.view(true);
        assert_eq!(v.level, Level::Searching);
        assert_eq!(v.button, SignalButton::Hidden);
        assert_ne!(v.summary, "Not checking");
    }

    #[test]
    fn starting_a_share_after_a_check_keeps_the_fix() {
        let mut m = SignalMonitor::new();
        m.start(5);
        m.on_fix(&gps(3.0), 0);
        m.start(5);
        assert_eq!(m.level(), Level::Good);
    }

    #[test]
    fn every_state_has_the_viewer_sentence_and_hint_the_design_lists() {
        let m = SignalMonitor::new();
        assert!(m.view(false).hint.is_some() && m.view(false).viewers.is_none());
        let mut m = SignalMonitor::new();
        m.start(5);
        let v = m.view(true);
        assert_eq!(v.viewers.as_deref(), Some("Nobody can see you yet."));
        m.on_fix(&gps(3.0), 0);
        assert!(m.view(true).hint.is_none(), "no hint when the signal is good");
    }
}
