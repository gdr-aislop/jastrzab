//! Prints an end-to-end encrypted `post.php` body for a fixed point, as JSON for
//! `tests/interop/webcrypto-open.mjs`, which decrypts it the way the web viewer does.
use hauk::crypto::{E2eKey, IV_LEN, SALT_LEN, b64};
use hauk::request::{E2ePost, Post, Provider, Request};

fn main() {
    let password = "correct horse";
    let salt: [u8; SALT_LEN] = hauk::crypto::random_bytes();
    let post = Post {
        sid: "SID".into(),
        latitude: 52.5200066,
        longitude: 13.404954,
        time: 1_700_000_000.25,
        provider: Provider::Fine,
        speed: Some(1.5),
        accuracy: Some(4.0),
        e2e: Some(E2ePost { key: E2eKey::derive(password.as_bytes(), &salt), iv: hauk::crypto::random_bytes::<IV_LEN>() }),
    };
    let fields: Vec<String> = Request::Post(post)
        .form()
        .into_iter()
        .map(|(name, value)| format!("\"{name}\":\"{value}\""))
        .collect();
    println!("{{\"password\":\"{password}\",\"salt\":\"{}\",{}}}", b64(&salt), fields.join(","));
}
