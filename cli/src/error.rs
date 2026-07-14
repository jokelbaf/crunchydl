//! User-safe CLI errors.

use std::path::PathBuf;

/// CLI result alias.
pub(crate) type Result<T> = std::result::Result<T, Error>;

/// An error safe to show directly in a terminal.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("could not determine the operating-system application directories")]
    AppDirectories,
    #[error("configuration at {path} is invalid: {message}")]
    InvalidConfig { path: PathBuf, message: String },
    #[error("could not access {operation} at {path}")]
    Filesystem {
        operation: &'static str,
        path: PathBuf,
    },
    #[error("the operating-system credential store is unavailable")]
    CredentialStore,
    #[error("not logged in; run `crunchydl login` first")]
    NotLoggedIn,
    #[error("login failed; verify the credentials and try again")]
    LoginFailed,
    #[error("the saved session expired; run `crunchydl login` again")]
    SessionExpired,
    #[error("Crunchyroll API error: {0}")]
    Api(crunchydl::ApiError),
    #[error("could not encode command output")]
    OutputEncoding,
    #[error("invalid locale `{0}`; use a BCP 47 locale such as en-US or ja-JP")]
    InvalidLocale(String),
    #[error("invalid media target: {0}")]
    InvalidTarget(String),
    #[error(
        "DRM is not configured; set a .prd or .wvd device with `crunchydl config set --drm-device <PATH>`"
    )]
    DrmNotConfigured,
    #[error(
        "could not infer the DRM backend from the device extension; use a .prd/.wvd file or set --drm-backend explicitly"
    )]
    CannotDetectDrmBackend,
    #[error("this binary was not built with {0} support")]
    #[cfg(any(not(feature = "drm-playready"), not(feature = "drm-widevine")))]
    DrmNotCompiled(&'static str),
    #[error("the configured DRM device could not be loaded or parsed")]
    InvalidDrmDevice,
    #[error("invalid filename or output layout template")]
    InvalidTemplate,
    #[error("download failed: {0}")]
    Download(crunchydl::Error),
    #[error("{0} queued download(s) failed; inspect the queue and retry")]
    QueueFailed(usize),
    #[error("password input failed")]
    PasswordInput,
    #[error("terminal input failed")]
    TerminalInput,
}

impl From<crunchyroll_rs::Error> for Error {
    fn from(error: crunchyroll_rs::Error) -> Self {
        Self::Api(error.into())
    }
}

impl Error {
    pub(crate) const fn exit_code(&self) -> i32 {
        match self {
            Self::NotLoggedIn | Self::SessionExpired | Self::LoginFailed => 3,
            Self::InvalidConfig { .. }
            | Self::InvalidLocale(_)
            | Self::InvalidTarget(_)
            | Self::DrmNotConfigured
            | Self::CannotDetectDrmBackend
            | Self::InvalidDrmDevice
            | Self::InvalidTemplate => 2,
            #[cfg(any(not(feature = "drm-playready"), not(feature = "drm-widevine")))]
            Self::DrmNotCompiled(_) => 2,
            _ => 1,
        }
    }
}
