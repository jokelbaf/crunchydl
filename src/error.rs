//! The library error type.
//!
//! [`Error`] is a non-exhaustive enum with source chaining. Display
//! implementations in this crate never print signed URL query parameters,
//! authorization headers, tokens, PSSH bodies, or content keys. API diagnostics
//! expose a stable category, HTTP status, and only secret-safe service messages.

use std::fmt;

use crate::selection::SelectionError;

/// Stable category for an error returned by `crunchyroll-rs`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiErrorKind {
    /// The HTTP request failed or returned an unsuccessful response.
    Request,
    /// Crunchyroll returned a response that could not be decoded.
    ResponseDecode,
    /// Authentication failed.
    Authentication,
    /// The request contained invalid input.
    InvalidInput,
    /// Crunchyroll or Cloudflare blocked the request.
    Blocked,
    /// The API client encountered an internal error.
    Internal,
}

/// A redacted Crunchyroll API error with inspectable category and HTTP status.
pub struct ApiError {
    kind: ApiErrorKind,
    status: Option<u16>,
    message: Option<String>,
    source: Box<crunchyroll_rs::Error>,
}

impl ApiError {
    /// Return the stable error category.
    #[must_use]
    pub const fn kind(&self) -> ApiErrorKind {
        self.kind
    }

    /// Return the HTTP response status when one was available.
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        self.status
    }

    /// Return a short service message when it passed secret-safety checks.
    #[must_use]
    pub fn service_message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

impl fmt::Debug for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiError")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self.kind {
            ApiErrorKind::Request => "request failed",
            ApiErrorKind::ResponseDecode => "response could not be decoded",
            ApiErrorKind::Authentication => "authentication failed",
            ApiErrorKind::InvalidInput => "request input was rejected",
            ApiErrorKind::Blocked => "request was blocked",
            ApiErrorKind::Internal => "API client failed internally",
        };
        formatter.write_str(description)?;
        if let Some(status) = self.status {
            write!(formatter, " (HTTP {status})")?;
        }
        if let Some(message) = &self.message {
            write!(formatter, ": {message}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

impl From<crunchyroll_rs::Error> for ApiError {
    fn from(source: crunchyroll_rs::Error) -> Self {
        use crunchyroll_rs::error::ErrorKind;

        let (kind, status) = match source.kind() {
            ErrorKind::Request { status } => (
                ApiErrorKind::Request,
                status.as_ref().map(|value| value.as_u16()),
            ),
            ErrorKind::Decode { .. } => (ApiErrorKind::ResponseDecode, None),
            ErrorKind::Authentication => (ApiErrorKind::Authentication, None),
            ErrorKind::Input => (ApiErrorKind::InvalidInput, None),
            ErrorKind::Block { .. } => (ApiErrorKind::Blocked, Some(403)),
            ErrorKind::Internal => (ApiErrorKind::Internal, None),
        };
        let message = source
            .message()
            .and_then(safe_service_message)
            .or_else(|| status_message(status).map(str::to_string));
        Self {
            kind,
            status,
            message,
            source: Box::new(source),
        }
    }
}

fn status_message(status: Option<u16>) -> Option<&'static str> {
    match status {
        Some(420) => Some("too many active playback sessions"),
        _ => None,
    }
}

fn safe_service_message(message: &str) -> Option<String> {
    let message = message.trim();
    let lower = message.to_ascii_lowercase();
    let forbidden = [
        "://",
        "authorization",
        "bearer ",
        "playback token",
        "license response",
        "pssh",
    ];
    (message.len() <= 240
        && !message.is_empty()
        && !message.contains(['{', '}', '[', ']', '\"', '\n', '\r'])
        && !forbidden.iter().any(|value| lower.contains(value)))
    .then(|| message.to_string())
}

/// The error type returned by [`crate::Downloader`] operations.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The Crunchyroll API returned an error or unexpected data.
    ///
    /// The underlying `crunchyroll-rs` error is available through
    /// [`std::error::Error::source`].
    #[error("crunchyroll API error: {0}")]
    Api(#[source] ApiError),

    /// The requested media, version, or locale is not available for playback.
    #[error("media unavailable: {0}")]
    Unavailable(String),

    /// A track, quality, locale, or CDN selection could not be satisfied.
    #[error(transparent)]
    Selection(#[from] SelectionError),

    /// A collection could not be canonicalized into logical media.
    #[error(transparent)]
    Batch(#[from] crate::BatchError),

    /// Opening or maintaining a playback session failed.
    #[error("playback session error: {0}")]
    Playback(String),

    /// A DASH manifest could not be parsed or had an unsupported shape.
    #[error("manifest error: {0}")]
    Manifest(String),

    /// Acquiring a DRM license failed.
    #[error("license error")]
    License(#[from] drm::Error),

    /// No content key was found for a track's default key id.
    #[error("missing content key for kid {kid}")]
    MissingKey {
        /// The default key id, hex-encoded, that had no matching content key.
        kid: String,
    },

    /// A segment transfer failed after exhausting retries.
    #[error("transfer error: {0}")]
    Transfer(String),

    /// Resume state did not match the current plan and cannot be reused.
    #[error("resume mismatch: {0}")]
    ResumeMismatch(String),

    /// Decrypting a media fragment failed.
    #[error("decrypt error: {0}")]
    Decrypt(String),

    /// Downloading or processing a subtitle resource failed.
    #[error("subtitle error: {0}")]
    Subtitle(String),

    /// Resolving or downloading a required font failed.
    #[error("font error: {0}")]
    Font(String),

    /// Parsing assembled media (ISO-BMFF) failed.
    #[error("media parse error")]
    MediaParse(#[from] media::Error),

    /// Selected dub timelines differ beyond the configured tolerance.
    #[error("incompatible audio timeline for {locale}")]
    IncompatibleTimelines {
        /// Audio locale whose timeline is incompatible.
        locale: String,
    },

    /// Muxing the output container failed.
    #[error("mux error: {0}")]
    Mux(String),

    /// The finished output failed independent verification.
    #[error("verification error: {0}")]
    Verification(String),

    /// A filesystem operation failed.
    #[error("filesystem error: {0}")]
    Filesystem(String),

    /// The operation was cancelled by the caller.
    #[error("operation cancelled")]
    Cancelled,
}

impl From<crunchyroll_rs::Error> for Error {
    fn from(error: crunchyroll_rs::Error) -> Self {
        Self::Api(error.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_does_not_leak_source_url() {
        // Our own Display strings must not embed signed material. The `Api`
        // variant prints a generic message; underlying details are only
        // reachable through `source()`, which is the caller's explicit choice.
        let err = Error::MissingKey {
            kid: "00112233445566778899aabbccddeeff".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "missing content key for kid 00112233445566778899aabbccddeeff"
        );
    }

    #[test]
    fn service_messages_reject_secret_bearing_or_structured_values() {
        assert_eq!(
            safe_service_message("stream limit exceeded"),
            Some("stream limit exceeded".to_string())
        );
        assert_eq!(
            safe_service_message("failed at https://example.test?a=b"),
            None
        );
        assert_eq!(safe_service_message(r#"{"token":"secret"}"#), None);
        assert_eq!(
            status_message(Some(420)),
            Some("too many active playback sessions")
        );
    }
}
