//! Location from GeoClue 2 over GDBus on the system bus.
//!
//! GDBus is used rather than a D-Bus crate so that no extra threads or async runtime are needed:
//! GLib already runs one worker thread for all bus connections, and replies come back on the main
//! context. The client asks for `EXACT` accuracy, sets its `TimeThreshold` to the update interval
//! (so the process is not woken for fixes it would drop), and never sets `DistanceThreshold`,
//! which would suppress updates from a stationary phone and make a healthy signal look lost.

use std::cell::RefCell;
use std::rc::Rc;

use async_channel::Sender;
use gio::prelude::*;
use jastrzab_core::Fix;
use jastrzab_core::engine::{Event, LocationFailure};
use jastrzab_core::paths::APP_ID;

const BUS_NAME: &str = "org.freedesktop.GeoClue2";
const MANAGER_PATH: &str = "/org/freedesktop/GeoClue2/Manager";
const MANAGER_IFACE: &str = "org.freedesktop.GeoClue2.Manager";
const CLIENT_IFACE: &str = "org.freedesktop.GeoClue2.Client";
const LOCATION_IFACE: &str = "org.freedesktop.GeoClue2.Location";
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
/// `GCLUE_ACCURACY_LEVEL_EXACT`.
const ACCURACY_LEVEL_EXACT: u32 = 8;
/// Starting a client may wait for the user to answer the system's location prompt.
const START_TIMEOUT_MSEC: i32 = 120_000;
const CALL_TIMEOUT_MSEC: i32 = 15_000;

#[derive(Default)]
struct State {
    conn: Option<gio::DBusConnection>,
    client: Option<String>,
    subscription: Option<gio::SignalSubscription>,
    /// Bumped by every `start` and `stop`, so an asynchronous start that finishes after a stop
    /// knows it has been overtaken.
    generation: u64,
}

#[derive(Clone)]
pub struct GeoClue {
    state: Rc<RefCell<State>>,
    events: Sender<Event>,
}

impl GeoClue {
    pub fn new(events: Sender<Event>) -> Self {
        Self { state: Rc::new(RefCell::new(State::default())), events }
    }

    /// Starts (or retunes) location updates. Answers with `LocationStarted` or `LocationFailed`.
    pub fn start(&self, interval_secs: u32) {
        let generation = {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.generation
        };
        let this = self.clone();
        glib::spawn_future_local(async move {
            let outcome = this.start_inner(interval_secs, generation).await;
            if this.state.borrow().generation != generation {
                return; // stopped or restarted meanwhile: someone else owns the answer
            }
            let event = match outcome {
                Ok(()) => Event::LocationStarted,
                Err(failure) => Event::LocationFailed(failure),
            };
            let _ = this.events.try_send(event);
        });
    }

    pub fn stop(&self) {
        let (conn, client) = {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.subscription = None;
            (state.conn.clone(), state.client.take())
        };
        if let (Some(conn), Some(client)) = (conn, client) {
            glib::spawn_future_local(async move { release_client(&conn, &client).await });
        }
    }

    async fn start_inner(&self, interval_secs: u32, generation: u64) -> Result<(), LocationFailure> {
        let conn = match self.state.borrow().conn.clone() {
            Some(conn) => conn,
            None => {
                let conn = gio::bus_get_future(gio::BusType::System).await.map_err(unavailable)?;
                self.state.borrow_mut().conn = Some(conn.clone());
                conn
            }
        };

        // Already running: only the update interval changes.
        if let Some(client) = self.state.borrow().client.clone() {
            set_property(&conn, &client, "TimeThreshold", interval_secs.to_variant()).await.map_err(unavailable)?;
            return Ok(());
        }

        let reply = conn
            .call_future(Some(BUS_NAME), MANAGER_PATH, MANAGER_IFACE, "GetClient", None, None, gio::DBusCallFlags::NONE, CALL_TIMEOUT_MSEC)
            .await
            .map_err(classify)?;
        let (client,): (glib::variant::ObjectPath,) = reply.get().ok_or_else(|| LocationFailure::Unavailable("unexpected GetClient reply".into()))?;
        let client = client.to_string();
        if self.state.borrow().generation != generation {
            release_client(&conn, &client).await;
            return Ok(());
        }

        set_property(&conn, &client, "DesktopId", APP_ID.to_variant()).await.map_err(unavailable)?;
        set_property(&conn, &client, "RequestedAccuracyLevel", ACCURACY_LEVEL_EXACT.to_variant()).await.map_err(unavailable)?;
        set_property(&conn, &client, "TimeThreshold", interval_secs.to_variant()).await.map_err(unavailable)?;

        let events = self.events.clone();
        let reader = conn.clone();
        let subscription = conn.subscribe_to_signal(
            Some(BUS_NAME),
            Some(CLIENT_IFACE),
            Some("LocationUpdated"),
            Some(&client),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some((_, new)) = signal.parameters.get::<(glib::variant::ObjectPath, glib::variant::ObjectPath)>() else { return };
                let (events, conn, path) = (events.clone(), reader.clone(), new.to_string());
                glib::spawn_future_local(async move {
                    if let Some(fix) = read_location(&conn, &path).await {
                        let _ = events.try_send(Event::Fix(fix));
                    }
                });
            },
        );
        {
            let mut state = self.state.borrow_mut();
            state.subscription = Some(subscription);
            state.client = Some(client.clone());
        }

        conn.call_future(Some(BUS_NAME), &client, CLIENT_IFACE, "Start", None, None, gio::DBusCallFlags::NONE, START_TIMEOUT_MSEC)
            .await
            .map_err(classify)?;

        // A location may already be known; don't wait for the first change to show it.
        if let Ok(reply) = get_property(&conn, &client, "Location").await
            && let Some(path) = reply.get::<glib::variant::ObjectPath>()
            && path.as_str() != "/"
            && let Some(fix) = read_location(&conn, path.as_str()).await
        {
            let _ = self.events.try_send(Event::Fix(fix));
        }
        Ok(())
    }
}

async fn release_client(conn: &gio::DBusConnection, client: &str) {
    let _ = conn.call_future(Some(BUS_NAME), client, CLIENT_IFACE, "Stop", None, None, gio::DBusCallFlags::NONE, CALL_TIMEOUT_MSEC).await;
    if let Ok(path) = glib::variant::ObjectPath::try_from(client.to_string()) {
        let _ = conn
            .call_future(
                Some(BUS_NAME),
                MANAGER_PATH,
                MANAGER_IFACE,
                "DeleteClient",
                Some(&(path,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                CALL_TIMEOUT_MSEC,
            )
            .await;
    }
}

async fn set_property(conn: &gio::DBusConnection, path: &str, name: &str, value: glib::Variant) -> Result<(), glib::Error> {
    let args = (CLIENT_IFACE, name, value).to_variant();
    conn.call_future(Some(BUS_NAME), path, PROPERTIES_IFACE, "Set", Some(&args), None, gio::DBusCallFlags::NONE, CALL_TIMEOUT_MSEC)
        .await
        .map(|_| ())
}

async fn get_property(conn: &gio::DBusConnection, path: &str, name: &str) -> Result<glib::Variant, glib::Error> {
    let reply = conn
        .call_future(
            Some(BUS_NAME),
            path,
            PROPERTIES_IFACE,
            "Get",
            Some(&(CLIENT_IFACE, name).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MSEC,
        )
        .await?;
    Ok(reply.child_value(0).as_variant().unwrap_or(reply))
}

async fn read_location(conn: &gio::DBusConnection, path: &str) -> Option<Fix> {
    let reply = conn
        .call_future(
            Some(BUS_NAME),
            path,
            PROPERTIES_IFACE,
            "GetAll",
            Some(&(LOCATION_IFACE,).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MSEC,
        )
        .await
        .map_err(|err| tracing::debug!(%err, "couldn't read a location from GeoClue"))
        .ok()?;
    fix_from_properties(&reply.child_value(0), unix_now())
}

fn unix_now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Builds a [`Fix`] from the `a{sv}` of a GeoClue `Location`. `now` stands in when the location
/// carries no timestamp.
fn fix_from_properties(dict: &glib::Variant, now: f64) -> Option<Fix> {
    let props = glib::VariantDict::new(Some(dict));
    let float = |name: &str| props.lookup::<f64>(name).ok().flatten();
    let latitude = float("Latitude")?;
    let longitude = float("Longitude")?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }
    // Unknown values are reported as -1 (accuracy) or a negative speed.
    let speed = float("Speed").filter(|s| *s >= 0.0).map(|s| s as f32);
    let unix_time = props
        .lookup::<(u64, u64)>("Timestamp")
        .ok()
        .flatten()
        .filter(|(secs, _)| *secs > 0)
        .map_or(now, |(secs, micros)| secs as f64 + micros as f64 / 1e6);
    Some(Fix {
        latitude,
        longitude,
        accuracy_m: float("Accuracy").unwrap_or(-1.0),
        speed,
        description: props.lookup::<String>("Description").ok().flatten().unwrap_or_default(),
        unix_time,
    })
}

fn unavailable(mut err: glib::Error) -> LocationFailure {
    gio::DBusError::strip_remote_error(&mut err);
    LocationFailure::Unavailable(err.message().to_string())
}

/// GeoClue refuses a client with `AccessDenied` when the location agent (Phosh) says no.
fn classify(err: glib::Error) -> LocationFailure {
    let text = format!("{} {}", gio::DBusError::remote_error(&err).map(|n| n.to_string()).unwrap_or_default(), err.message());
    let lower = text.to_lowercase();
    if lower.contains("accessdenied") || lower.contains("not authorized") || lower.contains("notauthorized") || lower.contains("denied") {
        LocationFailure::Denied
    } else {
        unavailable(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(entries: &[(&str, glib::Variant)]) -> glib::Variant {
        let dict = glib::VariantDict::new(None);
        for (key, value) in entries {
            dict.insert_value(key, value);
        }
        dict.end()
    }

    #[test]
    fn a_geoclue_location_becomes_a_fix() {
        let dict = location(&[
            ("Latitude", 52.52.to_variant()),
            ("Longitude", 13.405.to_variant()),
            ("Accuracy", 3.0.to_variant()),
            ("Speed", 1.25.to_variant()),
            ("Description", "GPS GGA".to_variant()),
            ("Timestamp", (1_700_000_000u64, 500_000u64).to_variant()),
        ]);
        let fix = fix_from_properties(&dict, 5.0).expect("valid");
        assert_eq!((fix.latitude, fix.longitude, fix.accuracy_m), (52.52, 13.405, 3.0));
        assert_eq!(fix.speed, Some(1.25));
        assert!(fix.is_gps());
        assert_eq!(fix.unix_time, 1_700_000_000.5);
    }

    #[test]
    fn unknown_speed_accuracy_and_timestamp_are_handled() {
        let dict = location(&[
            ("Latitude", 1.0.to_variant()),
            ("Longitude", 2.0.to_variant()),
            ("Speed", (-1.0f64).to_variant()),
            ("Timestamp", (0u64, 0u64).to_variant()),
        ]);
        let fix = fix_from_properties(&dict, 42.0).expect("valid");
        assert_eq!((fix.speed, fix.accuracy_m, fix.unix_time), (None, -1.0, 42.0));
        assert_eq!(fix.description, "");
    }

    #[test]
    fn impossible_or_missing_positions_are_dropped() {
        assert!(fix_from_properties(&location(&[("Latitude", 91.0.to_variant()), ("Longitude", 0.0.to_variant())]), 0.0).is_none());
        assert!(fix_from_properties(&location(&[("Latitude", 1.0.to_variant())]), 0.0).is_none());
        assert!(fix_from_properties(&location(&[]), 0.0).is_none());
    }
}
