//! Runs the whole sender protocol against a real Hauk backend.
//!
//! Ignored unless `HAUK_TEST_URL` (for example `http://127.0.0.1:8080/`) and `HAUK_TEST_PASSWORD`
//! are set:
//!
//! ```sh
//! HAUK_TEST_URL=http://127.0.0.1:8080/ HAUK_TEST_PASSWORD=testpw cargo test -p hauk --test live_server -- --ignored
//! ```
//!
//! The HTTP here is a few lines over `TcpStream` so this crate stays free of an HTTP dependency.

use std::io::{Read, Write};
use std::net::TcpStream;

use hauk::crypto::{E2eKey, IV_LEN, SALT_LEN, from_b64};
use hauk::request::{Adopt, Create, E2ePost, NewLink, Post, Provider, Request, ShareMode, Stop};
use hauk::response::{self, Reply};
use hauk::Version;

fn target() -> Option<(String, String)> {
    Some((std::env::var("HAUK_TEST_URL").ok()?, std::env::var("HAUK_TEST_PASSWORD").ok()?))
}

fn http(base: &str, method: &str, path: &str, body: &str) -> Reply {
    let rest = base.strip_prefix("http://").expect("an http:// test server");
    let (authority, prefix) = rest.split_once('/').unwrap_or((rest, ""));
    let mut stream = TcpStream::connect(authority).expect("connect to the test server");
    let request = format!(
        "{method} /{prefix}{path} HTTP/1.0\r\nHost: {authority}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").expect("a response");
    let status = head.lines().next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let version = head
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{}: ", hauk::VERSION_HEADER)))
        .and_then(Version::parse);
    Reply { status, version, body: body.to_string() }
}

fn send(base: &str, request: &Request) -> Reply {
    http(base, "POST", request.endpoint(), &request.body())
}

fn fetch(base: &str, link_id: &str) -> serde_like::Json {
    let reply = http(base, "GET", &format!("api/fetch.php?id={link_id}"), "");
    assert_eq!(reply.status, 200, "{}", reply.body);
    serde_like::Json(reply.body)
}

/// Just enough JSON inspection for these checks, to avoid a dependency.
mod serde_like {
    pub struct Json(pub String);
    impl Json {
        pub fn contains(&self, needle: &str) -> bool {
            self.0.contains(needle)
        }
    }
}

fn create(password: &str, mode: ShareMode, nickname: Option<&str>, pin: Option<&str>, salt: Option<[u8; SALT_LEN]>) -> Create {
    Create {
        username: None,
        password: password.into(),
        link_id: None,
        duration_secs: 600,
        interval_secs: 5,
        mode,
        adoptable: true,
        nickname: nickname.map(Into::into),
        group_pin: pin.map(Into::into),
        e2e_salt: salt,
    }
}

fn point(sid: &str, lat: f64, e2e: Option<E2ePost>) -> Request {
    Request::Post(Post {
        sid: sid.into(),
        latitude: lat,
        longitude: 13.404954,
        time: 1_700_000_000.0,
        provider: Provider::Fine,
        speed: Some(1.5),
        accuracy: Some(4.0),
        e2e,
    })
}

#[test]
#[ignore = "needs a Hauk backend: set HAUK_TEST_URL and HAUK_TEST_PASSWORD"]
fn solo_share_lifecycle() {
    let Some((base, password)) = target() else { return };

    let reply = send(&base, &Request::Create(create(&password, ShareMode::Alone, None, None, None)));
    assert!(reply.version.is_some(), "the backend sends its version");
    let created = response::parse_create(&reply, ShareMode::Alone).expect("create");
    let link_id = created.link_id.clone().expect("a link ID from a 1.2+ backend");

    let reply = send(&base, &point(&created.sid, 52.5200066, None));
    let posted = response::parse_post(&reply).expect("post");
    let links = posted.links.expect("link list");
    assert_eq!(links.ids, [link_id.clone()]);
    assert!(links.url_for(&link_id).ends_with(&link_id));

    let shown = fetch(&base, &link_id);
    assert!(shown.contains("52.5200066"), "the viewer endpoint serves the point: {}", shown.0);

    let reply = send(&base, &Request::NewLink(NewLink { sid: created.sid.clone(), adoptable: false }));
    let extra = response::parse_new_link(&reply).expect("new link");
    let reply = send(&base, &point(&created.sid, 52.5201, None));
    assert_eq!(response::parse_post(&reply).unwrap().links.unwrap().ids.len(), 2);

    let reply = send(&base, &Request::Stop(Stop { sid: created.sid.clone(), link_id: Some(extra.link_id) }));
    response::parse_ok(&reply).expect("stop one link");
    let reply = send(&base, &point(&created.sid, 52.5202, None));
    assert_eq!(response::parse_post(&reply).unwrap().links.unwrap().ids, [link_id.clone()]);

    let reply = send(&base, &Request::Stop(Stop { sid: created.sid.clone(), link_id: None }));
    response::parse_ok(&reply).expect("stop the session");
    let reply = send(&base, &point(&created.sid, 52.5203, None));
    assert!(response::parse_post(&reply).is_err(), "the session is gone: {}", reply.body);
}

#[test]
#[ignore = "needs a Hauk backend: set HAUK_TEST_URL and HAUK_TEST_PASSWORD"]
fn wrong_password_is_a_server_error_with_the_servers_words() {
    let Some((base, _)) = target() else { return };
    let reply = send(&base, &Request::Create(create("definitely-wrong", ShareMode::Alone, None, None, None)));
    match response::parse_create(&reply, ShareMode::Alone) {
        Err(hauk::Error::Server(message)) => assert!(!message.is_empty()),
        other => panic!("expected a server error, got {other:?}"),
    }
}

#[test]
#[ignore = "needs a Hauk backend: set HAUK_TEST_URL and HAUK_TEST_PASSWORD"]
fn encrypted_share_stores_only_ciphertext() {
    let Some((base, password)) = target() else { return };
    let salt: [u8; SALT_LEN] = hauk::crypto::random_bytes();
    let key = E2eKey::derive(b"correct horse", &salt);

    let reply = send(&base, &Request::Create(create(&password, ShareMode::Alone, None, None, Some(salt))));
    let created = response::parse_create(&reply, ShareMode::Alone).expect("create");
    let link_id = created.link_id.unwrap();

    let iv: [u8; IV_LEN] = hauk::crypto::random_bytes();
    let reply = send(&base, &point(&created.sid, 48.8566, Some(E2ePost { key: key.clone(), iv })));
    response::parse_post(&reply).expect("post");

    let shown = fetch(&base, &link_id);
    assert!(shown.contains("\"encrypted\":true"), "{}", shown.0);
    assert!(!shown.contains("48.8566"), "the server must not see the plaintext: {}", shown.0);
    assert!(shown.contains(&hauk::crypto::b64(&salt).replace('/', "\\/")), "the viewer gets the salt: {}", shown.0);

    // The stored latitude decrypts with the key the viewer derives.
    let body = &shown.0;
    let start = body.find("\"points\":[[").expect("a point") + "\"points\":[[".len();
    let fields: Vec<&str> = body[start..].split(']').next().unwrap().split(',').map(|f| f.trim_matches('"')).collect();
    let stored_iv = from_b64(&fields[0].replace("\\/", "/")).unwrap();
    let lat = key.decrypt(&stored_iv.try_into().unwrap(), &from_b64(&fields[1].replace("\\/", "/")).unwrap()).unwrap();
    assert_eq!(String::from_utf8(lat).unwrap(), "48.8566");

    send(&base, &Request::Stop(Stop { sid: created.sid, link_id: None }));
}

#[test]
#[ignore = "needs a Hauk backend: set HAUK_TEST_URL and HAUK_TEST_PASSWORD"]
fn group_host_join_and_adoption() {
    let Some((base, password)) = target() else { return };

    let reply = send(&base, &Request::Create(create(&password, ShareMode::CreateGroup, Some("Host"), None, None)));
    let host = response::parse_create(&reply, ShareMode::CreateGroup).expect("create group");
    let pin = host.group_pin.clone().expect("a group PIN");
    assert_eq!(pin.len(), 6);

    let reply = send(&base, &Request::Create(create(&password, ShareMode::JoinGroup, Some("Guest"), Some(&pin), None)));
    let guest = response::parse_create(&reply, ShareMode::JoinGroup).expect("join group");
    assert_eq!(guest.group_pin, None);

    // A third person shares alone, then the host adopts that share into the group.
    let reply = send(&base, &Request::Create(create(&password, ShareMode::Alone, None, None, None)));
    let solo = response::parse_create(&reply, ShareMode::Alone).expect("solo");
    let reply = send(
        &base,
        &Request::Adopt(Adopt { sid: host.sid.clone(), nickname: "Adopted".into(), target_id: solo.link_id.clone().unwrap(), group_pin: pin }),
    );
    response::parse_ok(&reply).expect("adopt");

    for sid in [&host.sid, &guest.sid, &solo.sid] {
        send(&base, &Request::Stop(Stop { sid: sid.clone(), link_id: None }));
    }
}
