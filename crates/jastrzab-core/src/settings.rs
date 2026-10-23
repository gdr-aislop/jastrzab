//! The user's settings and the share form's last values, as one serialisable record.
//!
//! Secrets (the server password and the end-to-end encryption password) are not here: they live
//! in the Secret Service keyring and reach the engine with each start request.

use serde::{Deserialize, Serialize};

pub const MIN_INTERVAL_SECS: u32 = 1;
pub const MAX_DURATION_SECS: u64 = 31 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DurationUnit {
    #[default]
    Minutes,
    Hours,
    Days,
}

impl DurationUnit {
    pub fn seconds(self) -> u64 {
        match self {
            DurationUnit::Minutes => 60,
            DurationUnit::Hours => 3600,
            DurationUnit::Days => 86_400,
        }
    }

    pub const ALL: [DurationUnit; 3] = [DurationUnit::Minutes, DurationUnit::Hours, DurationUnit::Days];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Alone,
    CreateGroup,
    JoinGroup,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Alone, Mode::CreateGroup, Mode::JoinGroup];

    pub fn is_group(self) -> bool {
        self != Mode::Alone
    }

    pub fn share_mode(self) -> hauk::ShareMode {
        match self {
            Mode::Alone => hauk::ShareMode::Alone,
            Mode::CreateGroup => hauk::ShareMode::CreateGroup,
            Mode::JoinGroup => hauk::ShareMode::JoinGroup,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyKind {
    /// Whatever the system is configured to use.
    #[default]
    System,
    None,
    Http,
    Socks,
}

/// How strictly the server's TLS certificate is checked. The two weaker choices apply only to
/// `.onion` hosts (Tor onion services have no CA-issued certificate), exactly as in Hauk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsPolicy {
    #[default]
    Always,
    /// For `.onion` hosts, accept a certificate that does not chain to a trusted root.
    OnionAnyChain,
    /// For `.onion` hosts, accept any certificate at all.
    OnionAnyCertificate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // Connection
    pub server_url: String,
    pub username: String,
    pub preferred_link_id: String,
    // Location updates
    pub interval_secs: u32,
    pub min_distance_m: f64,
    // Privacy
    pub e2e_enabled: bool,
    // Network
    pub proxy: ProxyKind,
    pub proxy_host: String,
    pub proxy_port: u16,
    pub timeout_secs: u32,
    pub tls: TlsPolicy,
    // Behaviour
    pub confirm_stop: bool,
    /// Believe the system when it says there is no network: stop sending and, after a while, switch
    /// the GPS off. Off for a server on a network that has no route to the internet, which the
    /// system counts as offline.
    pub pause_when_offline: bool,
    // Diagnostics
    pub anonymize_logs: bool,
    // The share form's last values
    pub duration: u32,
    pub unit: DurationUnit,
    pub nickname: String,
    pub adoptable: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            username: String::new(),
            preferred_link_id: String::new(),
            // Hauk's default is 1 s; every post wakes the radio, so Jastrzab defaults to 5 s.
            interval_secs: 5,
            min_distance_m: 0.0,
            e2e_enabled: false,
            proxy: ProxyKind::System,
            proxy_host: "localhost".into(),
            proxy_port: 9050,
            timeout_secs: 10,
            tls: TlsPolicy::Always,
            confirm_stop: true,
            pause_when_offline: true,
            anonymize_logs: true,
            duration: 2,
            unit: DurationUnit::Hours,
            nickname: String::new(),
            adoptable: true,
        }
    }
}

impl Settings {
    /// Clamps values a hand-edited file could get wrong.
    pub fn sanitized(mut self) -> Self {
        self.interval_secs = self.interval_secs.max(MIN_INTERVAL_SECS);
        self.timeout_secs = self.timeout_secs.max(1);
        if !self.min_distance_m.is_finite() || self.min_distance_m < 0.0 {
            self.min_distance_m = 0.0;
        }
        self.duration = self.duration.max(1);
        self.server_url = self.server_url.trim().to_string();
        self
    }

    /// Parses the settings file; a missing or unreadable file gives the defaults.
    pub fn from_toml(text: &str) -> Self {
        match toml::from_str::<Settings>(text) {
            Ok(settings) => settings.sanitized(),
            Err(err) => {
                tracing::warn!(%err, "couldn't read the settings file; using the defaults");
                Settings::default()
            }
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("settings serialise to TOML")
    }

    /// The server's base URL with a scheme and a trailing `/`, or `None` when none is configured.
    pub fn server_base(&self) -> Option<String> {
        normalize_server_url(&self.server_url)
    }
}

/// `example.com`, `http://example.com` and `https://example.com/` → a base URL ending in `/`.
/// A missing scheme means HTTPS. `None` for empty input or anything with whitespace.
pub fn normalize_server_url(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() || input.chars().any(char::is_whitespace) {
        return None;
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let (scheme, rest) = with_scheme.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") || rest.split('/').next().is_none_or(str::is_empty) {
        return None;
    }
    Some(if with_scheme.ends_with('/') { with_scheme } else { format!("{with_scheme}/") })
}

/// Whether `url`'s host ends in `.onion` (the only hosts the weaker TLS policies apply to).
pub fn is_onion(url: &str) -> bool {
    let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = authority.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = host.rsplit_once(':').map_or(host, |(h, p)| if p.bytes().all(|b| b.is_ascii_digit()) { h } else { host });
    host.to_ascii_lowercase().ends_with(".onion")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let settings = Settings::default();
        assert_eq!(Settings::from_toml(&settings.to_toml()), settings);
    }

    #[test]
    fn a_new_share_defaults_to_two_hours_and_a_saved_choice_is_kept() {
        let fresh = Settings::from_toml("");
        assert_eq!((fresh.duration, fresh.unit), (2, DurationUnit::Hours));
        let saved = Settings::from_toml("duration = 30\nunit = \"minutes\"\n");
        assert_eq!((saved.duration, saved.unit), (30, DurationUnit::Minutes));
    }

    #[test]
    fn unknown_and_missing_keys_are_tolerated() {
        let settings = Settings::from_toml("server_url = \"https://h.example/\"\nfuture_key = 1\n");
        assert_eq!(settings.server_url, "https://h.example/");
        assert_eq!(settings.interval_secs, 5);
    }

    #[test]
    fn garbage_gives_defaults() {
        assert_eq!(Settings::from_toml("this is [not toml"), Settings::default());
    }

    #[test]
    fn sanitizing_clamps_nonsense() {
        let s = Settings { interval_secs: 0, timeout_secs: 0, min_distance_m: -4.0, duration: 0, ..Settings::default() }.sanitized();
        assert_eq!((s.interval_secs, s.timeout_secs, s.duration), (1, 1, 1));
        assert_eq!(s.min_distance_m, 0.0);
        let s = Settings { min_distance_m: f64::NAN, ..Settings::default() }.sanitized();
        assert_eq!(s.min_distance_m, 0.0);
    }

    #[test]
    fn server_urls_are_normalised() {
        for (input, expected) in [
            ("example.com", Some("https://example.com/")),
            ("  http://example.com:8080  ", Some("http://example.com:8080/")),
            ("https://example.com/hauk/", Some("https://example.com/hauk/")),
            ("https://example.com/hauk", Some("https://example.com/hauk/")),
            ("", None),
            ("ftp://example.com", None),
            ("https://", None),
            ("two words", None),
        ] {
            assert_eq!(normalize_server_url(input).as_deref(), expected, "{input:?}");
        }
    }

    #[test]
    fn onion_hosts() {
        assert!(is_onion("http://abcdefghijklmnop.onion/"));
        assert!(is_onion("https://user@abc.onion:8443/x"));
        assert!(is_onion("ABC.ONION"));
        assert!(!is_onion("https://example.com/?x=.onion"));
        assert!(!is_onion("https://onion.example.com/"));
    }

    #[test]
    fn units() {
        assert_eq!(DurationUnit::Hours.seconds(), 3600);
        assert_eq!(Mode::JoinGroup.share_mode(), hauk::ShareMode::JoinGroup);
        assert!(Mode::CreateGroup.is_group() && !Mode::Alone.is_group());
    }
}
