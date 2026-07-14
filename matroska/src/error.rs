use std::io;

/// A Matroska validation or writing error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The requested value is invalid for the supported Matroska profile.
    #[error("invalid Matroska input: {0}")]
    Invalid(&'static str),
    /// A fallible upstream packet source failed.
    #[error("Matroska packet source failed: {0}")]
    PacketSource(String),
    /// The caller cancelled while packets were being streamed.
    #[error("Matroska muxing cancelled")]
    Cancelled,
    /// A checked size, timestamp, or file offset overflowed.
    #[error("Matroska arithmetic overflow while computing {0}")]
    Overflow(&'static str),
    /// The output stream failed.
    #[error("Matroska output I/O failed")]
    Io(#[from] io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;
