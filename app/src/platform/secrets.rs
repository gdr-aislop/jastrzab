//! The server password and the end-to-end encryption password, kept in the Secret Service
//! (gnome-keyring on PureOS) through libsecret rather than in a plain settings file.
//! libsecret's futures run on the GLib main loop, so no thread or runtime is involved.

use std::collections::HashMap;

use jastrzab_core::paths::APP_ID;
use libsecret::{Schema, SchemaAttributeType, SchemaFlags};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ServerPassword,
    EncryptionPassword,
}

impl Kind {
    fn attribute(self) -> &'static str {
        match self {
            Kind::ServerPassword => "server-password",
            Kind::EncryptionPassword => "encryption-password",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::ServerPassword => "Jastrzab server password",
            Kind::EncryptionPassword => "Jastrzab encryption password",
        }
    }
}

fn schema() -> Schema {
    Schema::new(
        APP_ID,
        SchemaFlags::NONE,
        HashMap::from([("kind", SchemaAttributeType::String), ("server", SchemaAttributeType::String)]),
    )
}

fn attributes<'a>(kind: Kind, server: &'a str) -> HashMap<&'a str, &'a str> {
    HashMap::from([("kind", kind.attribute()), ("server", server)])
}

/// The stored secret for this server, or an empty string if there is none or the keyring is
/// unavailable (which is logged, not shown: the server will say if a password is needed).
pub async fn lookup(kind: Kind, server: &str) -> String {
    match libsecret::password_lookup_future(Some(&schema()), attributes(kind, server)).await {
        Ok(secret) => secret.map(|s| s.to_string()).unwrap_or_default(),
        Err(err) => {
            tracing::warn!(%err, "couldn't read from the keyring");
            String::new()
        }
    }
}

/// Stores the secret; an empty one deletes the entry.
pub async fn store(kind: Kind, server: &str, secret: &str) {
    let result = if secret.is_empty() {
        libsecret::password_clear_future(Some(&schema()), attributes(kind, server)).await
    } else {
        libsecret::password_store_future(Some(&schema()), attributes(kind, server), Some(libsecret::COLLECTION_DEFAULT), kind.label(), secret).await
    };
    if let Err(err) = result {
        tracing::warn!(%err, "couldn't write to the keyring");
    }
}
