//! Request batching for the connection to the server.
//!
//! `post.php` takes one point per request, so batching here means never sending a request that
//! is redundant or already superseded, and never sending faster than the radio can afford:
//!
//! * at most one request is in flight (a stop of the whole session is the one exception and does
//!   not go through here);
//! * control requests (new link, adopt, stop one link) go first, in order;
//! * only the **latest** fix is kept: a fix that arrives while a post is in flight or waiting
//!   replaces the waiting one, and a superseded point is dropped rather than sent late;
//! * at most one post per update interval, and none closer than the minimum distance to the last
//!   point that was sent;
//! * a failed post is retried with exponential backoff (2, 4, 8 … 60 s) with the latest fix, so
//!   a long outage never builds a backlog, and coming back online retries at once.

use std::collections::VecDeque;

use crate::geo::{Fix, distance_m};

const FIRST_BACKOFF_SECS: u32 = 2;
const MAX_BACKOFF_SECS: u32 = 60;

pub type HttpId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Control,
    Post,
}

/// What the uplink wants done next.
#[derive(Debug, Clone, PartialEq)]
pub enum Poll<C> {
    /// A request is already in flight; wait for it.
    Busy,
    /// Nothing to send.
    Idle,
    /// Something is waiting but must not be sent before this many seconds have passed.
    Wait(u32),
    Control(C),
    Post(Fix),
}

#[derive(Debug, Clone)]
pub struct Uplink<C> {
    control: VecDeque<C>,
    pending_fix: Option<Fix>,
    /// The fix inside the post that is in flight, kept so a failed post can be retried.
    inflight_fix: Option<Fix>,
    in_flight: Option<(HttpId, Kind)>,
    last_post_ms: Option<u64>,
    last_sent_position: Option<(f64, f64)>,
    backoff_secs: u32,
    retry_at_ms: Option<u64>,
}

impl<C> Default for Uplink<C> {
    fn default() -> Self {
        Self {
            control: VecDeque::new(),
            pending_fix: None,
            inflight_fix: None,
            in_flight: None,
            last_post_ms: None,
            last_sent_position: None,
            backoff_secs: 0,
            retry_at_ms: None,
        }
    }
}

impl<C> Uplink<C> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_control(&mut self, item: C) {
        self.control.push_back(item);
    }

    pub fn has_pending_fix(&self) -> bool {
        self.pending_fix.is_some()
    }

    pub fn in_flight(&self) -> Option<(HttpId, Kind)> {
        self.in_flight
    }

    pub fn is_backing_off(&self) -> bool {
        self.backoff_secs > 0
    }

    /// Queues a fix as the one to send next, replacing any fix still waiting. Returns whether it
    /// was queued: a fix closer than `min_distance_m` to the last point sent is dropped.
    pub fn offer_fix(&mut self, fix: Fix, min_distance_m: f64) -> bool {
        if min_distance_m > 0.0
            && let Some(last) = self.last_sent_position
            && distance_m(last, (fix.latitude, fix.longitude)) < min_distance_m
        {
            return false;
        }
        self.pending_fix = Some(fix);
        true
    }

    /// Decides what to send. Call again after every change (a fix, a completion, a timer, the
    /// network coming back). When it returns `Control` or `Post` the item is removed from the
    /// queue and the caller must call [`begin`](Self::begin) with the request's id.
    pub fn poll(&mut self, now_ms: u64, interval_ms: u64, online: bool) -> Poll<C> {
        if self.in_flight.is_some() {
            return Poll::Busy;
        }
        if !online {
            return Poll::Idle;
        }
        if let Some(item) = self.control.pop_front() {
            return Poll::Control(item);
        }
        let Some(fix) = &self.pending_fix else { return Poll::Idle };
        let mut ready_at = self.last_post_ms.map_or(0, |t| t + interval_ms);
        if let Some(retry) = self.retry_at_ms {
            ready_at = ready_at.max(retry);
        }
        if now_ms < ready_at {
            return Poll::Wait(((ready_at - now_ms).div_ceil(1000)).max(1) as u32);
        }
        let fix = fix.clone();
        self.pending_fix = None;
        self.inflight_fix = Some(fix.clone());
        Poll::Post(fix)
    }

    pub fn begin(&mut self, id: HttpId, kind: Kind, now_ms: u64) {
        self.in_flight = Some((id, kind));
        if kind == Kind::Post {
            self.last_post_ms = Some(now_ms);
        }
    }

    /// The request with `id` succeeded. Returns its kind, or `None` if `id` is not the request
    /// in flight (a stale completion).
    pub fn finish_ok(&mut self, id: HttpId) -> Option<Kind> {
        let (current, kind) = self.in_flight?;
        if current != id {
            return None;
        }
        self.in_flight = None;
        if kind == Kind::Post {
            self.backoff_secs = 0;
            self.retry_at_ms = None;
            if let Some(fix) = self.inflight_fix.take() {
                self.last_sent_position = Some((fix.latitude, fix.longitude));
            }
        }
        Some(kind)
    }

    /// The request with `id` failed. A failed post is queued for retry (unless a newer fix has
    /// arrived) and starts or lengthens the backoff. Returns its kind, or `None` if stale.
    pub fn finish_err(&mut self, id: HttpId, now_ms: u64) -> Option<Kind> {
        let (current, kind) = self.in_flight?;
        if current != id {
            return None;
        }
        self.in_flight = None;
        if kind == Kind::Post {
            self.backoff_secs = if self.backoff_secs == 0 { FIRST_BACKOFF_SECS } else { (self.backoff_secs * 2).min(MAX_BACKOFF_SECS) };
            self.retry_at_ms = Some(now_ms + u64::from(self.backoff_secs) * 1000);
            if let Some(fix) = self.inflight_fix.take()
                && self.pending_fix.is_none()
            {
                self.pending_fix = Some(fix);
            }
        }
        Some(kind)
    }

    /// The network came back: forget the backoff so the waiting fix goes out immediately.
    pub fn network_restored(&mut self) {
        self.backoff_secs = 0;
        self.retry_at_ms = None;
    }

    /// Drops every queued post and control request (the session is ending). A request already in
    /// flight is left to finish.
    pub fn clear(&mut self) {
        self.control.clear();
        self.pending_fix = None;
        self.inflight_fix = None;
        self.backoff_secs = 0;
        self.retry_at_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(lat: f64) -> Fix {
        Fix { latitude: lat, longitude: 0.0, accuracy_m: 3.0, speed: None, description: "GPS GGA".into(), unix_time: lat }
    }

    const IV: u64 = 5_000;

    fn send_post(up: &mut Uplink<&'static str>, now: u64, id: HttpId) -> Fix {
        let Poll::Post(f) = up.poll(now, IV, true) else { panic!("expected a post") };
        up.begin(id, Kind::Post, now);
        f
    }

    #[test]
    fn the_first_fix_goes_out_at_once() {
        let mut up = Uplink::<&str>::new();
        assert_eq!(up.poll(0, IV, true), Poll::Idle);
        up.offer_fix(fix(1.0), 0.0);
        assert_eq!(send_post(&mut up, 0, 1).latitude, 1.0);
        assert_eq!(up.poll(0, IV, true), Poll::Busy);
    }

    #[test]
    fn at_most_one_post_per_interval() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        send_post(&mut up, 1_000, 1);
        up.finish_ok(1);
        up.offer_fix(fix(2.0), 0.0);
        assert_eq!(up.poll(2_000, IV, true), Poll::Wait(4));
        assert_eq!(up.poll(5_999, IV, true), Poll::Wait(1));
        assert_eq!(send_post(&mut up, 6_000, 2).latitude, 2.0);
    }

    #[test]
    fn only_the_latest_waiting_fix_is_sent() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        send_post(&mut up, 0, 1);
        for lat in [2.0, 3.0, 4.0] {
            up.offer_fix(fix(lat), 0.0);
        }
        up.finish_ok(1);
        assert_eq!(send_post(&mut up, 5_000, 2).latitude, 4.0, "2 and 3 were superseded and never sent");
        up.finish_ok(2);
        assert_eq!(up.poll(60_000, IV, true), Poll::Idle);
    }

    #[test]
    fn control_requests_go_first_and_in_order() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        up.push_control("a");
        up.push_control("b");
        assert_eq!(up.poll(0, IV, true), Poll::Control("a"));
        up.begin(1, Kind::Control, 0);
        assert_eq!(up.poll(0, IV, true), Poll::Busy);
        up.finish_ok(1);
        assert_eq!(up.poll(0, IV, true), Poll::Control("b"));
        up.begin(2, Kind::Control, 0);
        up.finish_ok(2);
        assert!(matches!(up.poll(0, IV, true), Poll::Post(_)));
    }

    #[test]
    fn control_requests_do_not_count_against_the_post_interval() {
        let mut up = Uplink::<&str>::new();
        up.push_control("a");
        let Poll::Control(_) = up.poll(0, IV, true) else { panic!() };
        up.begin(1, Kind::Control, 0);
        up.finish_ok(1);
        up.offer_fix(fix(1.0), 0.0);
        assert!(matches!(up.poll(1, IV, true), Poll::Post(_)));
    }

    #[test]
    fn minimum_distance_drops_close_points_after_the_first_send() {
        let mut up = Uplink::<&str>::new();
        assert!(up.offer_fix(fix(1.0), 50.0), "nothing sent yet");
        send_post(&mut up, 0, 1);
        up.finish_ok(1);
        // 0.0001 degrees of latitude is about 11 m.
        assert!(!up.offer_fix(fix(1.0001), 50.0));
        assert!(up.offer_fix(fix(1.001), 50.0), "about 111 m away");
        assert!(up.offer_fix(fix(1.0001), 0.0), "no minimum, nothing is dropped");
    }

    #[test]
    fn failed_posts_back_off_and_are_retried_with_the_same_fix() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        send_post(&mut up, 0, 1);
        assert_eq!(up.finish_err(1, 100), Some(Kind::Post));
        assert!(up.is_backing_off());
        // Backoff 2 s from t=100, but the interval gate is 5 s from the send at t=0.
        assert_eq!(up.poll(1_000, IV, true), Poll::Wait(4));
        assert_eq!(send_post(&mut up, 5_000, 2).latitude, 1.0, "the failed fix is retried");
        up.finish_err(2, 5_100);
        assert_eq!(up.poll(5_200, IV, true), Poll::Wait(5), "interval gate (10 s) beats the 4 s backoff");
    }

    #[test]
    fn backoff_doubles_up_to_a_minute_and_resets_on_success() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        let mut now = 0;
        let mut waits = Vec::new();
        for id in 1..=8 {
            // Use a zero interval so only the backoff gates the retry.
            let Poll::Post(_) = up.poll(now, 0, true) else { panic!("post {id}") };
            up.begin(id, Kind::Post, now);
            up.finish_err(id, now);
            let Poll::Wait(secs) = up.poll(now, 0, true) else { panic!("wait {id}") };
            waits.push(secs);
            now += u64::from(secs) * 1000;
        }
        assert_eq!(waits, [2, 4, 8, 16, 32, 60, 60, 60]);
        let Poll::Post(_) = up.poll(now, 0, true) else { panic!() };
        up.begin(9, Kind::Post, now);
        up.finish_ok(9);
        assert!(!up.is_backing_off());
    }

    #[test]
    fn a_newer_fix_wins_over_the_failed_one() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        send_post(&mut up, 0, 1);
        up.offer_fix(fix(2.0), 0.0);
        up.finish_err(1, 10);
        assert_eq!(send_post(&mut up, 60_000, 2).latitude, 2.0);
        up.finish_ok(2);
        assert_eq!(up.poll(120_000, IV, true), Poll::Idle, "the failed fix was not kept as well");
    }

    #[test]
    fn coming_back_online_retries_immediately() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        send_post(&mut up, 0, 1);
        up.finish_err(1, 0);
        assert!(matches!(up.poll(1_000, 0, true), Poll::Wait(_)));
        up.network_restored();
        assert!(matches!(up.poll(1_000, 0, true), Poll::Post(_)));
    }

    #[test]
    fn nothing_is_sent_while_offline() {
        let mut up = Uplink::<&str>::new();
        up.push_control("a");
        up.offer_fix(fix(1.0), 0.0);
        assert_eq!(up.poll(0, IV, false), Poll::Idle);
        assert_eq!(up.poll(0, IV, true), Poll::Control("a"));
    }

    #[test]
    fn stale_completions_are_ignored() {
        let mut up = Uplink::<&str>::new();
        up.offer_fix(fix(1.0), 0.0);
        send_post(&mut up, 0, 7);
        assert_eq!(up.finish_ok(6), None);
        assert_eq!(up.finish_err(6, 0), None);
        assert_eq!(up.in_flight(), Some((7, Kind::Post)));
    }

    #[test]
    fn clear_drops_everything_queued() {
        let mut up = Uplink::<&str>::new();
        up.push_control("a");
        up.offer_fix(fix(1.0), 0.0);
        up.clear();
        assert_eq!(up.poll(0, IV, true), Poll::Idle);
    }
}
