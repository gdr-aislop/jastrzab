//! Pulling a share ID out of what the user pasted.

/// Accepts a bare ID (`XPHU-64H8`) or a view URL (`https://host/?XPHU-64H8`) and returns the ID.
/// Mirrors the original's `\?([A-Za-z0-9-]+)` match, falling back to the whole text when it is
/// already a plausible ID. `None` for anything else.
pub fn extract(input: &str) -> Option<String> {
    let input = input.trim();
    let candidate = match input.split_once('?') {
        Some((_, query)) => query.split(['&', '#']).next().unwrap_or(""),
        None => input,
    };
    let valid = !candidate.is_empty() && candidate.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    valid.then(|| candidate.to_string())
}

#[cfg(test)]
mod tests {
    use super::extract;

    #[test]
    fn from_url_and_bare() {
        assert_eq!(extract("https://example.com/?XPHU-64H8").as_deref(), Some("XPHU-64H8"));
        assert_eq!(extract("  XPHU-64H8 ").as_deref(), Some("XPHU-64H8"));
        assert_eq!(extract("https://example.com/?abc#x").as_deref(), Some("abc"));
    }

    #[test]
    fn rejects_other_text() {
        assert_eq!(extract(""), None);
        assert_eq!(extract("two words"), None);
        assert_eq!(extract("https://example.com/"), None);
        assert_eq!(extract("https://example.com/?"), None);
    }
}
