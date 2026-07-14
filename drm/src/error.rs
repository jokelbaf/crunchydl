//! DRM error types.

/// An error produced while acquiring a license or decrypting media.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Device material could not be parsed or verified.
    #[error("invalid DRM device material")]
    Device,
    /// Protection-system initialization data was invalid or unsupported.
    #[error("invalid or unsupported DRM initialization data: {0}")]
    Initialization(String),
    /// A license challenge could not be generated.
    #[error("failed to create DRM license challenge")]
    Challenge,
    /// The license transport failed without exposing request secrets.
    #[error("DRM license transport failed: {0}")]
    Transport(String),
    /// The license response envelope or license was invalid.
    #[error("invalid DRM license response")]
    License,
    /// A content key was not exactly 16 bytes.
    #[error("content key has invalid length {0}; expected 16")]
    InvalidKeyLength(usize),
    /// No content key matched the requested key id.
    #[error("no content key matched kid {0}")]
    MissingKey(String),
    /// The fragmented MP4 layout is not supported by the bounded decrypter.
    #[error("unsupported encrypted MP4 layout: {0}")]
    UnsupportedLayout(String),
    /// An encrypted fragment could not be decrypted.
    #[error("CENC fragment decryption failed")]
    Decrypt,
    /// Assembled output could not be written.
    #[error("failed to write decrypted media")]
    Io(#[from] std::io::Error),
}
