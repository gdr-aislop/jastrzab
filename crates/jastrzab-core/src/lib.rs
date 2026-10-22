//! Platform-independent Jastrzab logic.
//!
//! Nothing here touches a toolkit, a socket, a clock or the file system (apart from the path
//! helpers and the settings and resume files' serialisation): the [`engine`] is a state machine
//! that takes events and the current time and returns effects, which the app executes.

pub mod geo;
pub mod i18n;
pub mod paths;
pub mod signal;

pub use geo::Fix;
pub use paths::AppPaths;
pub mod settings;
pub mod uplink;
pub mod session;
pub mod view;
pub mod engine;
