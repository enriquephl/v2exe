use chrono::{DateTime, Utc};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Connection failure or timeout.
    #[error("network error: {0}")]
    Network(String),
    /// Any other non-2xx response.
    #[error("unexpected HTTP status {0}")]
    Status(u16),
    #[error("not found")]
    NotFound,
    /// `reset` is `None` when the recovery time is unknown.
    #[error("rate limited")]
    RateLimited { reset: Option<DateTime<Utc>> },
    /// The node can only be viewed when logged in.
    #[error("node {node} requires login")]
    Restricted { node: String },
    /// The response could not be parsed.
    #[error("failed to decode response: {0}")]
    Decode(String),
    /// Invalid base URL or proxy.
    #[error("invalid configuration: {0}")]
    Config(String),
}
