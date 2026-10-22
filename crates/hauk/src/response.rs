//! Parsing the plain-text responses.
//!
//! A response is HTTP 200 with one datum per line. Line 1 is `OK` on success; otherwise the whole
//! body is a (possibly multi-line, localised) error message.

use crate::error::Error;
use crate::request::ShareMode;
use crate::version::{self, Version};

/// What the transport saw. `version` comes from the `X-Hauk-Version` header; a server that sends
/// none is treated as the oldest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub version: Option<Version>,
    pub body: String,
}

impl Reply {
    pub fn version(&self) -> Version {
        self.version.unwrap_or(Version::new(1, 0, 0))
    }
}

/// `create.php`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOk {
    pub sid: String,
    pub view_url: String,
    /// The PIN to give to people joining; only when a group was created.
    pub group_pin: Option<String>,
    /// The short ID of the first share (backend 1.2+).
    pub link_id: Option<String>,
}

/// `post.php`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostOk {
    /// The active links of the session, when the backend reports them (1.2+).
    pub links: Option<Links>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Links {
    /// A format string with one `%s`, to be filled with a link ID to get its view URL.
    pub format: String,
    pub ids: Vec<String>,
}

impl Links {
    pub fn url_for(&self, id: &str) -> String {
        self.format.replacen("%s", id, 1)
    }
}

/// `new-link.php`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLinkOk {
    pub view_url: String,
    pub link_id: String,
}

/// Returns the body's lines when the reply is a success (`OK` first), after stripping the
/// `OK` line. A failure returns the matching [`Error`].
fn ok_lines(reply: &Reply) -> Result<Vec<&str>, Error> {
    if reply.status != 200 {
        return Err(Error::Http(reply.status));
    }
    let body = reply.body.trim_end_matches(['\r', '\n']);
    if body.is_empty() {
        return Err(Error::Empty);
    }
    let mut lines = body.split('\n').map(|l| l.trim_end_matches('\r'));
    if lines.next() != Some("OK") {
        return Err(Error::Server(body.replace("\r\n", "\n")));
    }
    Ok(lines.collect())
}

/// `mode` is what the session was actually created as, which differs from what was asked for when
/// the backend is too old for groups.
pub fn parse_create(reply: &Reply, mode: ShareMode) -> Result<CreateOk, Error> {
    let lines = ok_lines(reply)?;
    let mut it = lines.into_iter();
    let sid = it.next().ok_or(Error::Malformed("no session ID"))?.to_string();
    let view_url = it.next().ok_or(Error::Malformed("no view URL"))?.to_string();
    let group_pin = if mode == ShareMode::CreateGroup {
        Some(it.next().ok_or(Error::Malformed("no group PIN"))?.to_string())
    } else {
        None
    };
    let link_id = if reply.version().is_at_least(version::LINK_IDS) {
        Some(it.next().ok_or(Error::Malformed("no link ID"))?.to_string())
    } else {
        None
    };
    Ok(CreateOk { sid, view_url, group_pin, link_id })
}

pub fn parse_post(reply: &Reply) -> Result<PostOk, Error> {
    let lines = ok_lines(reply)?;
    if !reply.version().is_at_least(version::LINK_IDS) {
        return Ok(PostOk { links: None });
    }
    let format = lines.first().ok_or(Error::Malformed("no link format"))?.to_string();
    let csv = lines.get(1).copied().unwrap_or("");
    let ids = csv.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect();
    Ok(PostOk { links: Some(Links { format, ids }) })
}

pub fn parse_new_link(reply: &Reply) -> Result<NewLinkOk, Error> {
    let lines = ok_lines(reply)?;
    let view_url = lines.first().ok_or(Error::Malformed("no view URL"))?.to_string();
    let link_id = lines.get(1).ok_or(Error::Malformed("no link ID"))?.to_string();
    Ok(NewLinkOk { view_url, link_id })
}

/// `adopt.php` and `stop.php`: success is just `OK`.
pub fn parse_ok(reply: &Reply) -> Result<(), Error> {
    ok_lines(reply).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(version: &str, body: &str) -> Reply {
        Reply { status: 200, version: Version::parse(version), body: body.to_string() }
    }

    #[test]
    fn create_solo_new_backend() {
        let r = reply("1.6.3", "OK\nSID\nhttps://h/?ABCD-1234\nABCD-1234\n");
        let ok = parse_create(&r, ShareMode::Alone).unwrap();
        assert_eq!(ok.sid, "SID");
        assert_eq!(ok.view_url, "https://h/?ABCD-1234");
        assert_eq!(ok.group_pin, None);
        assert_eq!(ok.link_id.as_deref(), Some("ABCD-1234"));
    }

    #[test]
    fn create_group_returns_pin_before_link_id() {
        let r = reply("1.6.3", "OK\nSID\nhttps://h/?G\n123456\nG-ID\n");
        let ok = parse_create(&r, ShareMode::CreateGroup).unwrap();
        assert_eq!(ok.group_pin.as_deref(), Some("123456"));
        assert_eq!(ok.link_id.as_deref(), Some("G-ID"));
    }

    #[test]
    fn join_group_has_no_pin_line() {
        let r = reply("1.6.3", "OK\nSID\nhttps://h/?G\nG-ID\n");
        let ok = parse_create(&r, ShareMode::JoinGroup).unwrap();
        assert_eq!(ok.group_pin, None);
        assert_eq!(ok.link_id.as_deref(), Some("G-ID"));
    }

    #[test]
    fn create_old_backend_has_no_link_id() {
        let r = reply("1.0.0", "OK\nSID\nhttps://h/?X\n");
        let ok = parse_create(&r, ShareMode::Alone).unwrap();
        assert_eq!(ok.link_id, None);
    }

    #[test]
    fn missing_header_means_oldest() {
        let r = Reply { status: 200, version: None, body: "OK\nSID\nhttps://h/?X\n".into() };
        assert_eq!(parse_create(&r, ShareMode::Alone).unwrap().link_id, None);
    }

    #[test]
    fn truncated_ok_is_malformed() {
        let r = reply("1.6.3", "OK\nSID\n");
        assert_eq!(parse_create(&r, ShareMode::Alone), Err(Error::Malformed("no view URL")));
    }

    #[test]
    fn server_errors_keep_the_whole_body() {
        let r = reply("1.6.3", "Invalid password!\r\nTry again.\r\n");
        assert_eq!(parse_ok(&r), Err(Error::Server("Invalid password!\nTry again.".into())));
    }

    #[test]
    fn http_and_empty() {
        assert_eq!(parse_ok(&Reply { status: 502, version: None, body: "OK".into() }), Err(Error::Http(502)));
        assert_eq!(parse_ok(&reply("1.6.3", "")), Err(Error::Empty));
        assert_eq!(parse_ok(&reply("1.6.3", "\n")), Err(Error::Empty));
    }

    #[test]
    fn post_lists_links() {
        let r = reply("1.6.3", "OK\nhttps://h/?%s\nA-1,B-2\n");
        let links = parse_post(&r).unwrap().links.unwrap();
        assert_eq!(links.ids, ["A-1", "B-2"]);
        assert_eq!(links.url_for("A-1"), "https://h/?A-1");
    }

    #[test]
    fn post_with_no_links_and_old_backend() {
        let r = reply("1.6.3", "OK\nhttps://h/?%s\n\n");
        assert_eq!(parse_post(&r).unwrap().links.unwrap().ids, Vec::<String>::new());
        let r = reply("1.1.0", "OK\n");
        assert_eq!(parse_post(&r).unwrap().links, None);
    }

    #[test]
    fn session_expired_surfaces_as_server_error() {
        let r = reply("1.6.3", "Session expired!\n");
        assert!(matches!(parse_post(&r), Err(Error::Server(_))));
    }

    #[test]
    fn new_link() {
        let r = reply("1.6.3", "OK\nhttps://h/?N-1\nN-1\n");
        let ok = parse_new_link(&r).unwrap();
        assert_eq!((ok.view_url.as_str(), ok.link_id.as_str()), ("https://h/?N-1", "N-1"));
    }
}
