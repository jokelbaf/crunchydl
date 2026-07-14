use std::io;

/// An ISO-BMFF parsing or packet-reading error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The input ended before a declared structure was complete.
    #[error("truncated ISO-BMFF input while reading {context}")]
    Truncated {
        /// Safe structural context.
        context: &'static str,
    },
    /// A declared box or field is malformed.
    #[error("invalid ISO-BMFF structure: {0}")]
    Invalid(&'static str),
    /// The file uses a valid layout outside the supported AVC/AAC fragment profile.
    #[error("unsupported ISO-BMFF layout: {0}")]
    Unsupported(&'static str),
    /// An arithmetic operation would overflow the supported representation.
    #[error("ISO-BMFF arithmetic overflow while computing {0}")]
    Overflow(&'static str),
    /// Reading or seeking the input failed.
    #[error("media input I/O failed")]
    Io(#[from] io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
