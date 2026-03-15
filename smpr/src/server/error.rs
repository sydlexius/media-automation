use std::fmt;

/// Error type for all media server API operations.
#[derive(Debug)]
pub enum MediaServerError {
    /// HTTP error with status code and response body snippet.
    Http { status: u16, body: String },
    /// Server unreachable or request timed out.
    Connection(String),
    /// Response body is not valid JSON.
    Parse(String),
    /// Valid JSON but missing expected fields.
    Protocol(String),
}

impl fmt::Display for MediaServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http { status, body } => write!(f, "HTTP {status}: {body}"),
            Self::Connection(msg) => write!(f, "connection error: {msg}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
        }
    }
}

impl std::error::Error for MediaServerError {}

impl From<ureq::Error> for MediaServerError {
    fn from(err: ureq::Error) -> Self {
        // With http_status_as_error(false), ureq returns HTTP 4xx/5xx as Ok(Response).
        // Only transport/connection errors reach here.
        Self::Connection(err.to_string())
    }
}
