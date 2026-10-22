//! A running share session and the snapshot of it kept on disk so a killed app can offer to
//! resume.

use hauk::crypto::E2eKey;
use hauk::response::Links;
use serde::{Deserialize, Serialize};

use crate::settings::Mode;

/// Which kind of share a link belongs to; only used to label it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    /// Just this device's location.
    Own,
    /// A group this device created.
    GroupHost,
    /// A group this device joined, or one it was adopted into.
    GroupMember,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: String,
    pub url: String,
    pub kind: LinkKind,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub sid: String,
    /// The server's base URL, ending in `/`.
    pub server: String,
    /// What was actually created, which differs from what was asked for when the server is too
    /// old for groups.
    pub mode: Mode,
    pub expires_at_unix: f64,
    pub interval_secs: u32,
    pub group_pin: Option<String>,
    pub e2e: Option<E2eKey>,
    pub links: Vec<Link>,
    /// `https://host/?%s`, once the server has told us.
    pub link_format: Option<String>,
}

impl Session {
    pub fn primary_link(&self) -> Option<&Link> {
        self.links.first()
    }

    pub fn link(&self, id: &str) -> Option<&Link> {
        self.links.iter().find(|l| l.id == id)
    }

    /// The kind a new link added by *this* device gets.
    pub fn own_kind(&self) -> LinkKind {
        LinkKind::Own
    }

    pub fn add_link(&mut self, link: Link) {
        if self.link(&link.id).is_none() {
            self.links.push(link);
        }
    }

    pub fn remove_link(&mut self, id: &str) -> bool {
        let before = self.links.len();
        self.links.retain(|l| l.id != id);
        self.links.len() != before
    }

    /// Brings the link list in line with what the server says is active, as the original does:
    /// an unknown link is a group share this device was adopted into, and a known link the server
    /// no longer lists has ended. Returns whether anything changed.
    pub fn reconcile(&mut self, active: &Links) -> bool {
        self.link_format = Some(active.format.clone());
        let mut changed = false;
        for id in &active.ids {
            if self.link(id).is_none() {
                self.links.push(Link { id: id.clone(), url: active.url_for(id), kind: LinkKind::GroupMember });
                changed = true;
            }
        }
        let before = self.links.len();
        self.links.retain(|l| active.ids.contains(&l.id));
        changed || self.links.len() != before
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            format: Snapshot::FORMAT,
            sid: self.sid.clone(),
            server: self.server.clone(),
            mode: self.mode,
            expires_at_unix: self.expires_at_unix,
            interval_secs: self.interval_secs,
            group_pin: self.group_pin.clone(),
            e2e_key: self.e2e.as_ref().map(|key| hex(key.as_bytes())),
            link_format: self.link_format.clone(),
            links: self.links.clone(),
        }
    }
}

/// The unfinished session as stored in `session.toml` (mode 0600). It is written when a share
/// starts, when its links change, and deleted when it ends; never once per fix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: u32,
    pub sid: String,
    pub server: String,
    pub mode: Mode,
    pub expires_at_unix: f64,
    pub interval_secs: u32,
    pub group_pin: Option<String>,
    /// The derived end-to-end key, hex. Kept because the password is not (it is in the keyring
    /// and the salt is not recoverable from the server).
    pub e2e_key: Option<String>,
    pub link_format: Option<String>,
    pub links: Vec<Link>,
}

impl Snapshot {
    pub const FORMAT: u32 = 1;

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("snapshot serialises to TOML")
    }

    /// `None` for a file that is unreadable, from another format version, or already expired.
    pub fn from_toml(text: &str, now_unix: f64) -> Option<Self> {
        let snapshot: Snapshot = toml::from_str(text).ok()?;
        (snapshot.format == Self::FORMAT && snapshot.expires_at_unix > now_unix && !snapshot.links.is_empty()).then_some(snapshot)
    }

    pub fn into_session(self) -> Option<Session> {
        let e2e = match &self.e2e_key {
            Some(text) => Some(E2eKey::from_bytes(unhex(text)?)),
            None => None,
        };
        Some(Session {
            sid: self.sid,
            server: self.server,
            mode: self.mode,
            expires_at_unix: self.expires_at_unix,
            interval_secs: self.interval_secs,
            group_pin: self.group_pin,
            e2e,
            links: self.links,
            link_format: self.link_format,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn session() -> Session {
        Session {
            sid: "SID".into(),
            server: "https://h.example/".into(),
            mode: Mode::Alone,
            expires_at_unix: 2_000.0,
            interval_secs: 5,
            group_pin: None,
            e2e: None,
            links: vec![Link { id: "A-1".into(), url: "https://h.example/?A-1".into(), kind: LinkKind::Own }],
            link_format: None,
        }
    }

    fn links(ids: &[&str]) -> Links {
        Links { format: "https://h.example/?%s".into(), ids: ids.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn unknown_links_are_added_as_group_membership_and_missing_ones_removed() {
        let mut s = session();
        assert!(!s.reconcile(&links(&["A-1"])), "nothing changed");
        assert!(s.reconcile(&links(&["A-1", "G-9"])));
        assert_eq!(s.link("G-9").unwrap().kind, LinkKind::GroupMember);
        assert_eq!(s.link("G-9").unwrap().url, "https://h.example/?G-9");
        assert!(s.reconcile(&links(&["G-9"])));
        assert!(s.link("A-1").is_none());
        assert_eq!(s.link_format.as_deref(), Some("https://h.example/?%s"));
    }

    #[test]
    fn add_link_ignores_duplicates_and_remove_reports() {
        let mut s = session();
        s.add_link(s.links[0].clone());
        assert_eq!(s.links.len(), 1);
        assert!(s.remove_link("A-1"));
        assert!(!s.remove_link("A-1"));
    }

    #[test]
    fn snapshot_round_trips_including_the_key() {
        let mut s = session();
        s.e2e = Some(E2eKey::derive(b"pw", &[1; 32]));
        s.group_pin = Some("123456".into());
        let text = s.snapshot().to_toml();
        let back = Snapshot::from_toml(&text, 1_000.0).expect("valid").into_session().expect("key decodes");
        assert_eq!(back.sid, "SID");
        assert_eq!(back.e2e, s.e2e);
        assert_eq!(back.links, s.links);
        assert_eq!(back.group_pin.as_deref(), Some("123456"));
    }

    #[test]
    fn expired_foreign_or_broken_snapshots_are_not_offered() {
        let text = session().snapshot().to_toml();
        assert!(Snapshot::from_toml(&text, 1_999.0).is_some());
        assert!(Snapshot::from_toml(&text, 2_000.0).is_none(), "expired");
        assert!(Snapshot::from_toml(&text.replace("format = 1", "format = 2"), 0.0).is_none());
        assert!(Snapshot::from_toml("garbage [", 0.0).is_none());
        let mut s = session();
        s.links.clear();
        assert!(Snapshot::from_toml(&s.snapshot().to_toml(), 0.0).is_none(), "no links to resume");
    }

    #[test]
    fn a_corrupt_key_does_not_resume() {
        let mut snap = session().snapshot();
        snap.e2e_key = Some("zz".repeat(32));
        assert!(snap.clone().into_session().is_none());
        snap.e2e_key = Some("ab".into());
        assert!(snap.into_session().is_none());
    }
}
