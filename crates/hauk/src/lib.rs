//! The client side of the [Hauk](https://github.com/bilde2910/Hauk) wire protocol.
//!
//! Sans-IO: this crate builds the form-encoded request bodies and parses the plain-text
//! responses, and does the optional end-to-end encryption, but never touches a socket or a clock.
//! Transport, proxies and TLS belong to the caller. Parameter names, response line order and the
//! encryption parameters are those of Hauk 1.6.3 (`android/.../Constants.java` and the
//! `*Packet.java` classes), because the PHP backend and the web viewer must interoperate.

pub mod crypto;
pub mod error;
pub mod request;
pub mod response;
pub mod share_id;
pub mod version;

pub use error::Error;
pub use request::{Adopt, Create, NewLink, Post, Provider, Request, ShareMode, Stop};
pub use version::Version;

/// Name of the response header carrying the backend's version.
pub const VERSION_HEADER: &str = "X-Hauk-Version";
