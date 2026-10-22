//! Adapters between the engine's effects and the platform: GeoClue, libsoup, the notification
//! server, logind, the Secret Service, the network monitor and the file system.

pub mod fake_location;
pub mod geoclue;
pub mod http;
pub mod inhibit;
pub mod location;
pub mod network;
pub mod notifications;
pub mod secrets;
pub mod signals;
pub mod store;
