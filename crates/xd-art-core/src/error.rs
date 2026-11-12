//! The crate error type.

use std::fmt;
use std::path::PathBuf;

/// Shorthand for results returned by this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong inside xd-art-core.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// A file could not be read or written.
    Io {
        /// File that failed.
        path: PathBuf,
        /// Underlying error.
        source: std::io::Error,
    },
    /// An image or palette file exists but its contents could not be decoded.
    Decode {
        /// File that failed.
        path: PathBuf,
        /// Short description from the decoder.
        reason: String,
    },
    /// Writing an image failed after the file was opened.
    Encode {
        /// File that failed.
        path: PathBuf,
        /// Short description from the encoder.
        reason: String,
    },
    /// The file extension or format is not supported.
    Unsupported(String),
    /// A setting is outside its valid range.
    Invalid {
        /// Name of the setting.
        name: &'static str,
        /// What was wrong with it.
        reason: String,
    },
    /// The work was cancelled through a [`crate::cancel::Cancel`] token.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "could not access {}: {source}", path.display()),
            Self::Decode { path, reason } => {
                write!(f, "could not decode {}: {reason}", path.display())
            }
            Self::Encode { path, reason } => {
                write!(f, "could not encode {}: {reason}", path.display())
            }
            Self::Unsupported(what) => write!(f, "unsupported format: {what}"),
            Self::Invalid { name, reason } => write!(f, "invalid {name}: {reason}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
