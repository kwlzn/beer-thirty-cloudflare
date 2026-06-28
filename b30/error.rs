//
// Error types shared across the (pure) parsing/rendering modules and the
// (wasm-only) worker glue. Nothing here depends on the `worker` crate except
// the `From` impl at the bottom, which is gated to the wasm target so the pure
// modules compile and test on the host.
//

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppError {
    /// Bad request / header construction on our side.
    Client(String),
    /// Network/transport failure talking to an upstream.
    Network(String),
    /// Upstream responded but we couldn't parse what we expected.
    Parse(String),
    /// Upstream actively refused us (bot challenge, auth/credentials rotated).
    /// Distinct from `NotFound` so callers can avoid caching it.
    Blocked(String),
    /// Upstream responded fine, but there is simply no match for the query.
    NotFound,
    /// Anything else.
    Internal(String),
}

impl std::error::Error for AppError {}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Client(msg) => write!(f, "Client error: {msg}"),
            AppError::Network(msg) => write!(f, "Network error: {msg}"),
            AppError::Parse(msg) => write!(f, "Parse error: {msg}"),
            AppError::Blocked(msg) => write!(f, "Blocked error: {msg}"),
            AppError::NotFound => write!(f, "Not found"),
            AppError::Internal(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(target_arch = "wasm32")]
impl From<AppError> for worker::Error {
    fn from(error: AppError) -> Self {
        worker::Error::from(error.to_string())
    }
}
