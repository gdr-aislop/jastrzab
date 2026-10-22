//! The five calls the sender makes, and how each is encoded.
//!
//! Every call is a `POST` of `application/x-www-form-urlencoded` to `<server>/<endpoint>`.

use crate::crypto::{E2eKey, IV_LEN};

/// How a share is created (`mod`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareMode {
    /// A private single-user share.
    Alone = 0,
    /// Create a group and become its host.
    CreateGroup = 1,
    /// Join someone else's group with its PIN.
    JoinGroup = 2,
}

impl ShareMode {
    pub fn is_group(self) -> bool {
        self != ShareMode::Alone
    }
}

/// How good the position behind a point is (`prv`). The viewer draws these as red (fine, live)
/// and orange (coarse, rough) markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Fine = 0,
    Coarse = 1,
}

/// `api/create.php`: starts a session and its first share.
#[derive(Debug, Clone, PartialEq)]
pub struct Create {
    pub username: Option<String>,
    pub password: String,
    /// A requested custom link ID (`lid`).
    pub link_id: Option<String>,
    pub duration_secs: u64,
    pub interval_secs: u32,
    pub mode: ShareMode,
    /// Whether someone may later adopt this share into a group. Only sent for [`ShareMode::Alone`].
    pub adoptable: bool,
    /// Required for the group modes.
    pub nickname: Option<String>,
    /// Required for [`ShareMode::JoinGroup`].
    pub group_pin: Option<String>,
    /// When present the session is end-to-end encrypted and this is the key derivation salt.
    pub e2e_salt: Option<[u8; 32]>,
}

/// `api/post.php`: one location point.
#[derive(Debug, Clone, PartialEq)]
pub struct Post {
    pub sid: String,
    pub latitude: f64,
    pub longitude: f64,
    /// Unix time in seconds, with a fraction.
    pub time: f64,
    pub provider: Provider,
    /// Metres per second.
    pub speed: Option<f32>,
    /// Horizontal accuracy in metres.
    pub accuracy: Option<f32>,
    /// Encrypts the point when the session is end-to-end encrypted.
    pub e2e: Option<E2ePost>,
}

/// What encrypting one point needs: the session's key and a fresh IV for this point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct E2ePost {
    pub key: E2eKey,
    pub iv: [u8; IV_LEN],
}

/// `api/new-link.php`: another share for the same session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLink {
    pub sid: String,
    pub adoptable: bool,
}

/// `api/adopt.php`: pulls another user's solo share into this group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopt {
    pub sid: String,
    pub nickname: String,
    /// The ID of the share being adopted (`aid`).
    pub target_id: String,
    /// The PIN of this session's group.
    pub group_pin: String,
}

/// `api/stop.php`: ends one link, or the whole session when `link_id` is `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    pub sid: String,
    pub link_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Create(Create),
    Post(Post),
    NewLink(NewLink),
    Adopt(Adopt),
    Stop(Stop),
}

type Form = Vec<(&'static str, String)>;

impl Request {
    /// Path below the server URL, which always ends in `/`.
    pub fn endpoint(&self) -> &'static str {
        match self {
            Request::Create(_) => "api/create.php",
            Request::Post(_) => "api/post.php",
            Request::NewLink(_) => "api/new-link.php",
            Request::Adopt(_) => "api/adopt.php",
            Request::Stop(_) => "api/stop.php",
        }
    }

    pub fn form(&self) -> Form {
        match self {
            Request::Create(r) => r.form(),
            Request::Post(r) => r.form(),
            Request::NewLink(r) => {
                vec![("sid", r.sid.clone()), ("ado", flag(r.adoptable))]
            }
            Request::Adopt(r) => vec![("sid", r.sid.clone()), ("nic", r.nickname.clone()), ("aid", r.target_id.clone()), ("pin", r.group_pin.clone())],
            Request::Stop(r) => {
                let mut form = vec![("sid", r.sid.clone())];
                if let Some(link_id) = &r.link_id {
                    form.push(("lid", link_id.clone()));
                }
                form
            }
        }
    }

    /// The `application/x-www-form-urlencoded` body.
    pub fn body(&self) -> String {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        for (name, value) in self.form() {
            serializer.append_pair(name, &value);
        }
        serializer.finish()
    }
}

fn flag(value: bool) -> String {
    if value { "1" } else { "0" }.to_string()
}

impl Create {
    fn form(&self) -> Form {
        let mut form = Form::new();
        if let Some(username) = &self.username {
            form.push(("usr", username.clone()));
        }
        if let Some(link_id) = &self.link_id {
            form.push(("lid", link_id.clone()));
        }
        if let Some(salt) = &self.e2e_salt {
            form.push(("salt", crate::crypto::b64(salt)));
        }
        form.push(("pwd", self.password.clone()));
        form.push(("dur", self.duration_secs.to_string()));
        form.push(("int", self.interval_secs.to_string()));
        form.push(("e2e", flag(self.e2e_salt.is_some())));
        form.push(("mod", (self.mode as u8).to_string()));
        match self.mode {
            ShareMode::Alone => form.push(("ado", flag(self.adoptable))),
            ShareMode::CreateGroup => {
                form.push(("nic", self.nickname.clone().unwrap_or_default()));
            }
            ShareMode::JoinGroup => {
                form.push(("nic", self.nickname.clone().unwrap_or_default()));
                form.push(("pin", self.group_pin.clone().unwrap_or_default()));
            }
        }
        form
    }
}

impl Post {
    fn form(&self) -> Form {
        let mut form = vec![("sid", self.sid.clone())];
        let provider = (self.provider as u8).to_string();
        let lat = self.latitude.to_string();
        let lon = self.longitude.to_string();
        let time = self.time.to_string();
        let speed = self.speed.map(|v| v.to_string());
        let accuracy = self.accuracy.map(|v| v.to_string());
        match &self.e2e {
            None => {
                form.push(("lat", lat));
                form.push(("lon", lon));
                form.push(("prv", provider));
                form.push(("time", time));
                if let Some(v) = speed {
                    form.push(("spd", v));
                }
                if let Some(v) = accuracy {
                    form.push(("acc", v));
                }
            }
            Some(e2e) => {
                // Every field is encrypted with the same key and IV; only the IV travels in the
                // clear. The viewer decrypts each field independently with that IV.
                let seal = |text: &str| crate::crypto::b64(&e2e.key.encrypt(&e2e.iv, text.as_bytes()));
                form.push(("iv", crate::crypto::b64(&e2e.iv)));
                form.push(("lat", seal(&lat)));
                form.push(("lon", seal(&lon)));
                form.push(("prv", seal(&provider)));
                form.push(("time", seal(&time)));
                if let Some(v) = speed {
                    form.push(("spd", seal(&v)));
                }
                if let Some(v) = accuracy {
                    form.push(("acc", seal(&v)));
                }
            }
        }
        form
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(form: &Form) -> Vec<&'static str> {
        form.iter().map(|(k, _)| *k).collect()
    }

    fn get<'a>(form: &'a Form, name: &str) -> Option<&'a str> {
        form.iter().find(|(k, _)| *k == name).map(|(_, v)| v.as_str())
    }

    fn create(mode: ShareMode) -> Create {
        Create {
            username: None,
            password: "hunter 2&".into(),
            link_id: None,
            duration_secs: 1800,
            interval_secs: 5,
            mode,
            adoptable: true,
            nickname: Some("Ann".into()),
            group_pin: Some("123456".into()),
            e2e_salt: None,
        }
    }

    #[test]
    fn create_alone_sends_adoptable_but_no_nickname() {
        let form = Request::Create(create(ShareMode::Alone)).form();
        assert_eq!(names(&form), ["pwd", "dur", "int", "e2e", "mod", "ado"]);
        assert_eq!(get(&form, "mod"), Some("0"));
        assert_eq!(get(&form, "ado"), Some("1"));
        assert_eq!(get(&form, "e2e"), Some("0"));
    }

    #[test]
    fn create_group_and_join_group() {
        let form = Request::Create(create(ShareMode::CreateGroup)).form();
        assert_eq!(get(&form, "mod"), Some("1"));
        assert_eq!(get(&form, "nic"), Some("Ann"));
        assert_eq!(get(&form, "pin"), None);
        assert_eq!(get(&form, "ado"), None);

        let form = Request::Create(create(ShareMode::JoinGroup)).form();
        assert_eq!(get(&form, "mod"), Some("2"));
        assert_eq!(get(&form, "pin"), Some("123456"));
    }

    #[test]
    fn create_optional_fields_and_encryption_flag() {
        let mut c = create(ShareMode::Alone);
        c.username = Some("u".into());
        c.link_id = Some("my-link".into());
        c.e2e_salt = Some([7; 32]);
        let form = Request::Create(c).form();
        assert_eq!(get(&form, "usr"), Some("u"));
        assert_eq!(get(&form, "lid"), Some("my-link"));
        assert_eq!(get(&form, "e2e"), Some("1"));
        assert_eq!(get(&form, "salt").map(str::len), Some(44)); // 32 bytes of base64
    }

    #[test]
    fn body_is_form_urlencoded() {
        let body = Request::Create(create(ShareMode::Alone)).body();
        assert!(body.starts_with("pwd=hunter+2%26&dur=1800&int=5&e2e=0&mod=0&ado=1"), "{body}");
    }

    fn post() -> Post {
        Post { sid: "S".into(), latitude: 52.5, longitude: -1.25, time: 1_700_000_000.5, provider: Provider::Fine, speed: Some(1.5), accuracy: None, e2e: None }
    }

    #[test]
    fn plain_post() {
        let form = Request::Post(post()).form();
        assert_eq!(names(&form), ["sid", "lat", "lon", "prv", "time", "spd"]);
        assert_eq!(get(&form, "lat"), Some("52.5"));
        assert_eq!(get(&form, "lon"), Some("-1.25"));
        assert_eq!(get(&form, "time"), Some("1700000000.5"));
        assert_eq!(get(&form, "prv"), Some("0"));
    }

    #[test]
    fn encrypted_post_has_iv_and_no_plaintext() {
        let mut p = post();
        p.accuracy = Some(12.0);
        p.provider = Provider::Coarse;
        p.e2e = Some(E2ePost { key: E2eKey::derive(b"pw", &[1; 32]), iv: [9; IV_LEN] });
        let form = Request::Post(p).form();
        assert_eq!(names(&form), ["sid", "iv", "lat", "lon", "prv", "time", "spd", "acc"]);
        assert_eq!(get(&form, "iv").map(str::len), Some(24)); // 16 bytes of base64
        assert!(!form.iter().any(|(k, v)| *k != "sid" && *k != "iv" && v.contains("52.5")));
    }

    #[test]
    fn other_calls() {
        let form = Request::NewLink(NewLink { sid: "S".into(), adoptable: false }).form();
        assert_eq!(form, [("sid", "S".to_string()), ("ado", "0".to_string())]);
        let form = Request::Adopt(Adopt { sid: "S".into(), nickname: "Bo".into(), target_id: "T".into(), group_pin: "123456".into() }).form();
        assert_eq!(names(&form), ["sid", "nic", "aid", "pin"]);
        assert_eq!(Request::Stop(Stop { sid: "S".into(), link_id: None }).form().len(), 1);
        assert_eq!(get(&Request::Stop(Stop { sid: "S".into(), link_id: Some("L".into()) }).form(), "lid"), Some("L"));
    }

    #[test]
    fn endpoints() {
        assert_eq!(Request::Create(create(ShareMode::Alone)).endpoint(), "api/create.php");
        assert_eq!(Request::Stop(Stop { sid: "S".into(), link_id: None }).endpoint(), "api/stop.php");
    }
}
