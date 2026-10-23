//! Whether the system thinks it is online, so posting pauses while offline and resumes the moment
//! a connection returns instead of polling. The engine also switches the GPS off after a while
//! without a network (see `Engine::pause_for_offline`).
//!
//! The source is `GNetworkMonitor`: with NetworkManager running, GLib's backend follows its `State`
//! and `Connectivity` properties on the system bus (available from "connected to a site" up;
//! `ASLEEP`, `DISCONNECTED` and `CONNECTED_LOCAL`, a link with no route out, count as gone). A
//! second D-Bus client for the same two properties would add nothing, and would not work inside a
//! Flatpak, where GLib uses the NetworkMonitor portal instead.

use async_channel::Sender;
use gio::prelude::*;
use jastrzab_core::engine::Event;

/// Sends the current state now and again on every change. The monitor is kept alive by the
/// connected handler for the life of the process.
pub fn watch(events: Sender<Event>) {
    let monitor = gio::NetworkMonitor::default();
    // GLib picks NetworkManager's backend when NetworkManager is running (and the portal's inside
    // a Flatpak); the name in the log says which one the app is really listening to.
    tracing::info!(backend = monitor.type_().name(), available = monitor.is_network_available(), connectivity = ?monitor.connectivity(), "watching the network");
    let _ = events.try_send(Event::Network(monitor.is_network_available()));

    // `network-changed` alone is not enough: with NetworkManager it comes from the kernel's route
    // changes, which can run ahead of NetworkManager's own state (and report the old one), while
    // `network-available` follows NetworkManager's `State` property directly. The engine ignores
    // a repeat of what it already knows.
    let report = move |monitor: &gio::NetworkMonitor, why: &'static str| {
        let available = monitor.is_network_available();
        tracing::info!(why, available, connectivity = ?monitor.connectivity(), "the network changed");
        let _ = events.try_send(Event::Network(available));
    };
    let reporter = std::rc::Rc::new(report);
    monitor.connect_network_changed({
        let report = reporter.clone();
        move |monitor, _| report(monitor, "route change")
    });
    monitor.connect_network_available_notify({
        let report = reporter.clone();
        move |monitor| report(monitor, "network manager")
    });
}
