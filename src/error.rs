use std::fmt::{self, Display};
use std::io;

#[derive(Debug)]
pub enum Error {
    Parse(String),
    Source(String),
    Usage(String),
    World(String),
    Membership(String),
    Relation(String),
    BootstrapLimit(String),
    Implementation(String),
    Io(io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A stable, user-facing category for CLI diagnostics.
    pub fn category(&self) -> &'static str {
        match self {
            Self::Parse(_) => "parse error",
            Self::Source(_) => "source error",
            Self::Usage(_) => "usage error",
            Self::World(_) => "world error",
            Self::Membership(_) => "membership error",
            Self::Relation(_) => "relation error",
            Self::BootstrapLimit(_) => "bootstrap limit",
            Self::Implementation(_) => "implementation error",
            Self::Io(_) => "I/O error",
        }
    }
}

impl Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message)
            | Self::Source(message)
            | Self::Usage(message)
            | Self::World(message)
            | Self::Membership(message)
            | Self::Relation(message)
            | Self::BootstrapLimit(message)
            | Self::Implementation(message) => formatter.write_str(message),
            Self::Io(error) => Display::fmt(error, formatter),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
