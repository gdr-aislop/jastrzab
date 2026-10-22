//! Whether the system thinks it is online, so posting pauses while offline and resumes the moment
//! a connection returns instead of polling.

use async_channel::Sender;
use gio::prelude::*;
use jastrzab_core::engine::Event;

/// Sends the current state now and again on every change. The monitor is kept alive by the
/// connected handler for the life of the process.
pub fn watch(events: Sender<Event>) {
    let monitor = gio::NetworkMonitor::default();
    let _ = events.try_send(Event::Network(monitor.is_network_available()));
    monitor.connect_network_changed(move |_, available| {
        let _ = events.try_send(Event::Network(available));
    });
}
