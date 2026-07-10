use serde::Serialize;

/// Error returned by every tauri command. Serialized as `{ kind, message }`.
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub kind: ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Connect,
    Query,
    Cancelled,
    Io,
    Config,
    Driver,
    Unsupported,
    Internal,
}

impl AppError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
    pub fn connect(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Connect, m)
    }
    pub fn query(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Query, m)
    }
    pub fn io(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Io, m)
    }
    pub fn config(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Config, m)
    }
    pub fn driver(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Driver, m)
    }
    pub fn unsupported(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, m)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "cancelled")
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::io(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
