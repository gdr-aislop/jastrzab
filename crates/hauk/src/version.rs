use std::fmt;

/// A backend or client version, `major.minor[.patch]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

/// Group shares need a backend of at least this version.
pub const GROUP_SHARES: Version = Version::new(1, 1, 0);
/// `create.php` and `post.php` only report link IDs from this version.
pub const LINK_IDS: Version = Version::new(1, 2, 0);
/// End-to-end encryption needs a backend of at least this version.
pub const E2E: Version = Version::new(1, 5, 0);

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// Parses `1.6.3`, `1.6` and `1.6.3-dev`. Anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let numeric = text.split(['-', '+', ' ']).next()?;
        let mut parts = numeric.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self { major, minor, patch })
    }

    pub fn is_at_least(self, other: Version) -> bool {
        self >= other
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_forms() {
        assert_eq!(Version::parse("1.6.3"), Some(Version::new(1, 6, 3)));
        assert_eq!(Version::parse(" 1.6 "), Some(Version::new(1, 6, 0)));
        assert_eq!(Version::parse("2"), Some(Version::new(2, 0, 0)));
        assert_eq!(Version::parse("1.6.3-dev"), Some(Version::new(1, 6, 3)));
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "x", "1.x", "1.2.3.4", ".1"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn gates() {
        let old = Version::parse("1.0.2").unwrap();
        let new = Version::parse("1.6.3").unwrap();
        assert!(!old.is_at_least(GROUP_SHARES));
        assert!(!old.is_at_least(LINK_IDS));
        assert!(new.is_at_least(E2E));
        assert!(Version::new(1, 5, 0).is_at_least(E2E));
        assert!(!Version::new(1, 4, 9).is_at_least(E2E));
    }
}
