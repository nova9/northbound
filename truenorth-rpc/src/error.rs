use std::fmt;

#[derive(Debug)]
pub enum Error {
    Transport(String),
    Remote {
        method: String,
        code: i64,
        message: String,
    },
    InvalidData(String),
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) => write!(formatter, "TrueNorth RPC: {message}"),
            Self::Remote { method, code, .. } => {
                write!(formatter, "TrueNorth RPC {method} failed with code {code}")
            }
            Self::InvalidData(message) => write!(formatter, "invalid RPC data: {message}"),
            Self::Io(error) => write!(formatter, "RPC cookie I/O: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
