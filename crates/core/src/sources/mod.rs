//! Pure parsers from API responses (or config) to [`Line`](crate::Line)s.
//! HTTP, caching and timeouts live in the Worker.

pub mod github;
pub mod spotify;
pub mod uptime;
pub mod weather;

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self(format!("bad response: {e}"))
    }
}
