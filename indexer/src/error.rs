use std::fmt;

#[derive(Debug)]
pub enum Error {
    Rpc(String),
    InvalidData(String),
    Database(rusqlite::Error),
    Io(std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc(message) => write!(formatter, "TrueNorth RPC: {message}"),
            Self::InvalidData(message) => write!(formatter, "invalid index data: {message}"),
            Self::Database(error) => write!(formatter, "SQLite: {error}"),
            Self::Io(error) => write!(formatter, "indexer I/O: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
