use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum GeneratorError {
    Usage(String),
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    Xml {
        path: PathBuf,
        detail: String,
    },
    Invalid(String),
    Registry(String),
    Cia402(String),
    ProcBuf(String),
    Publication(String),
}

impl GeneratorError {
    pub fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for GeneratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Invalid(message) => formatter.write_str(message),
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} {} failed: {source}", path.display()),
            Self::Json { path, source } => {
                write!(
                    formatter,
                    "invalid product JSON {}: {source}",
                    path.display()
                )
            }
            Self::Xml { path, detail } => {
                write!(formatter, "invalid ESI XML {}: {detail}", path.display())
            }
            Self::Registry(detail) => write!(formatter, "invalid Domain plan: {detail}"),
            Self::Cia402(detail) => write!(formatter, "invalid CiA 402 configuration: {detail}"),
            Self::ProcBuf(detail) => write!(formatter, "invalid ProcBuf configuration: {detail}"),
            Self::Publication(detail) => write!(formatter, "artifact publication failed: {detail}"),
        }
    }
}

impl Error for GeneratorError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, GeneratorError>;
