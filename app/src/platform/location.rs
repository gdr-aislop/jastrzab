//! The location source: GeoClue on a device, or a scripted one for development.

use async_channel::Sender;
use jastrzab_core::engine::Event;

use super::fake_location::{FakeLocation, Spec};
use super::geoclue::GeoClue;

pub enum Location {
    GeoClue(GeoClue),
    Fake(FakeLocation),
}

impl Location {
    pub fn new(events: Sender<Event>) -> Self {
        // A scripted source is a development aid and exists only in debug builds.
        if cfg!(debug_assertions)
            && let Ok(text) = std::env::var("JASTRZAB_FAKE_LOCATION")
        {
            match Spec::parse(&text) {
                Some(spec) => {
                    tracing::warn!("using the scripted location source from JASTRZAB_FAKE_LOCATION");
                    return Location::Fake(FakeLocation::new(spec, events));
                }
                None => tracing::warn!("ignoring an unreadable JASTRZAB_FAKE_LOCATION"),
            }
        }
        Location::GeoClue(GeoClue::new(events))
    }

    /// Always answered, asynchronously, with `LocationStarted` or `LocationFailed`.
    pub fn start(&self, interval_secs: u32) {
        match self {
            Location::GeoClue(source) => source.start(interval_secs),
            Location::Fake(source) => source.start(interval_secs),
        }
    }

    pub fn stop(&self) {
        match self {
            Location::GeoClue(source) => source.stop(),
            Location::Fake(source) => source.stop(),
        }
    }
}
