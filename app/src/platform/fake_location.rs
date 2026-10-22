//! A scripted location source for development and the headless UI checks (debug builds only).
//!
//! `JASTRZAB_FAKE_LOCATION=<lat>,<lon>[,<accuracies>[,<description>]]` replaces GeoClue.
//! `<accuracies>` is a `/`-separated list of metres that the fixes cycle through
//! (`0/3/50/300`), `none` produces a source that never reports a fix, and `<description>`
//! defaults to `GPS GGA` (anything else looks like a network fix).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use async_channel::Sender;
use jastrzab_core::Fix;
use jastrzab_core::engine::Event;

#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracies: Vec<f64>,
    pub description: String,
}

impl Spec {
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.splitn(4, ',');
        let latitude = parts.next()?.trim().parse().ok()?;
        let longitude = parts.next()?.trim().parse().ok()?;
        let accuracies = match parts.next().map(str::trim) {
            None | Some("") => vec![3.0],
            Some("none") => Vec::new(),
            Some(list) => list.split('/').map(|a| a.trim().parse().ok()).collect::<Option<Vec<f64>>>()?,
        };
        let description = parts.next().map_or("GPS GGA", str::trim).to_string();
        Some(Self { latitude, longitude, accuracies, description })
    }

    fn fix(&self, n: usize, unix_time: f64) -> Option<Fix> {
        if self.accuracies.is_empty() {
            return None;
        }
        Some(Fix {
            // About 5 m of northward drift per fix.
            latitude: self.latitude + n as f64 * 0.000_05,
            longitude: self.longitude,
            accuracy_m: self.accuracies[n % self.accuracies.len()],
            speed: Some(1.4),
            description: self.description.clone(),
            unix_time,
        })
    }
}

#[derive(Clone)]
pub struct FakeLocation {
    spec: Spec,
    events: Sender<Event>,
    timer: Rc<RefCell<Option<glib::SourceId>>>,
    count: Rc<Cell<usize>>,
}

impl FakeLocation {
    pub fn new(spec: Spec, events: Sender<Event>) -> Self {
        Self { spec, events, timer: Rc::new(RefCell::new(None)), count: Rc::new(Cell::new(0)) }
    }

    pub fn start(&self, interval_secs: u32) {
        self.cancel();
        let events = self.events.clone();
        glib::idle_add_local_once(move || {
            let _ = events.try_send(Event::LocationStarted);
        });
        let this = self.clone();
        let id = glib::timeout_add_seconds_local(interval_secs.max(1), move || {
            let n = this.count.get();
            this.count.set(n + 1);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
            if let Some(fix) = this.spec.fix(n, now) {
                let _ = this.events.try_send(Event::Fix(fix));
            }
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(id);
    }

    pub fn stop(&self) {
        self.cancel();
    }

    fn cancel(&self) {
        if let Some(id) = self.timer.borrow_mut().take()
            && let Some(source) = glib::MainContext::default().find_source_by_id(&id)
        {
            source.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_parse() {
        let spec = Spec::parse("52.5,13.4").unwrap();
        assert_eq!((spec.latitude, spec.longitude, spec.accuracies.clone(), spec.description.as_str()), (52.5, 13.4, vec![3.0], "GPS GGA"));
        assert_eq!(Spec::parse("1,2,0/50/300").unwrap().accuracies, [0.0, 50.0, 300.0]);
        assert!(Spec::parse("1,2,none").unwrap().accuracies.is_empty());
        assert_eq!(Spec::parse("1,2,80,WiFi").unwrap().description, "WiFi");
        assert!(Spec::parse("x,2").is_none());
        assert!(Spec::parse("1").is_none());
        assert!(Spec::parse("1,2,a/b").is_none());
    }

    #[test]
    fn fixes_cycle_and_drift() {
        let spec = Spec::parse("10,20,0/50").unwrap();
        assert_eq!(spec.fix(0, 1.0).unwrap().accuracy_m, 0.0);
        assert_eq!(spec.fix(1, 1.0).unwrap().accuracy_m, 50.0);
        assert_eq!(spec.fix(2, 1.0).unwrap().accuracy_m, 0.0);
        assert!(spec.fix(2, 1.0).unwrap().latitude > spec.fix(0, 1.0).unwrap().latitude);
        assert!(Spec::parse("1,2,none").unwrap().fix(0, 0.0).is_none());
    }
}
