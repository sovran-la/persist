use std::fmt;

/// Persistence errors.
#[derive(Debug)]
pub enum Error {
    /// IO error (file not found, permission denied, etc.)
    Io(std::io::Error),
    /// Failed to parse stored data.
    Parse(String),
    /// Type mismatch when using get_as.
    TypeMismatch {
        expected: &'static str,
        actual: &'static str,
    },
    /// Coercion failed when using get_coerce.
    CoercionFailed {
        from: &'static str,
        to: &'static str,
        reason: String,
    },
    /// Custom error from a Store implementation.
    Custom(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Parse(msg) => write!(f, "parse error: {msg}"),
            Error::TypeMismatch { expected, actual } => {
                write!(f, "type mismatch: expected {expected}, got {actual}")
            }
            Error::CoercionFailed { from, to, reason } => {
                write!(f, "coercion failed ({from} -> {to}): {reason}")
            }
            Error::Custom(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
