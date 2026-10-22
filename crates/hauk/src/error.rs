use std::fmt;

/// Why a Hauk call did not succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The server answered, but line 1 was not `OK`. Carries the whole body (the server's
    /// message, possibly several lines); the original shows all of it to the user.
    Server(String),
    /// The server answered with an HTTP status other than 200.
    Http(u16),
    /// A 200 with an empty body.
    Empty,
    /// `OK`, but fewer lines than the endpoint documents.
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Server(message) => f.write_str(message),
            Error::Http(code) => write!(f, "HTTP {code}"),
            Error::Empty => f.write_str("empty response"),
            Error::Malformed(what) => write!(f, "malformed response: {what}"),
        }
    }
}

impl std::error::Error for Error {}
