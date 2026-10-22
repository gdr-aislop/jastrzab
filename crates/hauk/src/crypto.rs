//! End-to-end encryption, bit-compatible with the Android app and the web viewer.
//!
//! The key is PBKDF2-HMAC-**SHA1** (65536 iterations) of the UTF-8 password with the session's
//! random 32-byte salt, 256 bits long. Each field of a point is AES-256-CBC with PKCS#7 padding,
//! encrypted under one IV per point, and sent base64-encoded. The viewer derives the same key with
//! WebCrypto (`PBKDF2`, `SHA-1`, 65536, `AES-CBC`) and decrypts each field with the posted `iv`.

use std::fmt;

use aes::Aes256;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use hmac::Hmac;
use sha1::Sha1;

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 32;
pub const IV_LEN: usize = 16;
const PBKDF2_ITERATIONS: u32 = 65536;

/// Standard base64 with padding and no line breaks.
pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn from_b64(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text.trim()).ok()
}

/// A derived AES-256 key. Deriving costs 65536 HMAC rounds, so do it once per session.
#[derive(Clone, PartialEq, Eq)]
pub struct E2eKey([u8; KEY_LEN]);

impl E2eKey {
    pub fn derive(password: &[u8], salt: &[u8]) -> Self {
        let mut key = [0u8; KEY_LEN];
        pbkdf2::pbkdf2::<Hmac<Sha1>>(password, salt, PBKDF2_ITERATIONS, &mut key).expect("HMAC accepts a key of any length");
        Self(key)
    }

    /// Rebuilds a key that was derived earlier and kept (the resume file).
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub fn encrypt(&self, iv: &[u8; IV_LEN], plaintext: &[u8]) -> Vec<u8> {
        cbc::Encryptor::<Aes256>::new((&self.0).into(), iv.into()).encrypt_padded_vec::<Pkcs7>(plaintext)
    }

    /// The inverse of [`encrypt`](Self::encrypt). Used by tests and the interoperability checks.
    pub fn decrypt(&self, iv: &[u8; IV_LEN], ciphertext: &[u8]) -> Option<Vec<u8>> {
        cbc::Decryptor::<Aes256>::new((&self.0).into(), iv.into()).decrypt_padded_vec::<Pkcs7>(ciphertext).ok()
    }
}

impl fmt::Debug for E2eKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("E2eKey(..)")
    }
}

/// `n` random bytes from the operating system.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the operating system's random source");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_sha1_known_answer() {
        // RFC 6070 test vector 4 (4096 iterations is what the RFC lists; here the same function
        // is checked at our iteration count against an independently computed value, below).
        let mut out = [0u8; 20];
        pbkdf2::pbkdf2::<Hmac<Sha1>>(b"password", b"salt", 4096, &mut out).unwrap();
        assert_eq!(hex(&out), "4b007901b765489abead49d926f721d065a429c1");
    }

    #[test]
    fn key_matches_node_webcrypto() {
        // Independently derived with Node's WebCrypto, exactly as frontend/main.js does:
        //   deriveKey({name:"PBKDF2", salt, iterations:65536, hash:"SHA-1"}, key, {name:"AES-CBC", length:256}, ...)
        // for password "correct horse" and salt bytes 0..32.
        let salt: Vec<u8> = (0..32).collect();
        let key = E2eKey::derive(b"correct horse", &salt);
        assert_eq!(hex(key.as_bytes()), KNOWN_KEY_HEX, "update KNOWN_KEY_HEX from the node check script");
    }

    // Output of tests/interop/webcrypto-key.mjs (the viewer's own derivation) for the inputs above.
    const KNOWN_KEY_HEX: &str = "87cdcf1969d741787d6214666ed7d216dd8b690caa9cb8b94650a1da7a70da58";

    #[test]
    fn roundtrip_and_padding() {
        let key = E2eKey::derive(b"pw", &[3; SALT_LEN]);
        let iv = [5u8; IV_LEN];
        for text in ["", "52.1234567", "a longer plaintext than one 16 byte block"] {
            let sealed = key.encrypt(&iv, text.as_bytes());
            assert_eq!(sealed.len() % 16, 0);
            assert!(sealed.len() > text.len(), "PKCS#7 always pads");
            assert_eq!(key.decrypt(&iv, &sealed).unwrap(), text.as_bytes());
        }
    }

    #[test]
    fn wrong_key_does_not_decrypt_to_the_plaintext() {
        let a = E2eKey::derive(b"pw", &[3; SALT_LEN]);
        let b = E2eKey::derive(b"other", &[3; SALT_LEN]);
        let iv = [0u8; IV_LEN];
        let sealed = a.encrypt(&iv, b"secret");
        assert_ne!(b.decrypt(&iv, &sealed).as_deref(), Some(&b"secret"[..]));
    }

    #[test]
    fn base64_has_no_line_breaks() {
        let text = b64(&[0xff; 70]);
        assert!(!text.contains('\n'));
        assert_eq!(from_b64(&text).unwrap(), vec![0xff; 70]);
    }

    #[test]
    fn debug_hides_the_key() {
        let key = E2eKey::derive(b"pw", &[0; SALT_LEN]);
        assert_eq!(format!("{key:?}"), "E2eKey(..)");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
