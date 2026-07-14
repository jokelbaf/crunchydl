//! DRM license acquisition and Common Encryption decryption.
//!
//! Backend-specific device material and license state never leave this crate.
//! Callers provide a [`LicenseTransport`] and receive a zeroizing [`KeySet`].

#![forbid(unsafe_code)]

mod cenc;
mod error;
mod key;
mod license;
#[cfg(feature = "playready")]
mod playready;
#[cfg(feature = "widevine")]
mod widevine;

pub use cenc::{CencDecrypter, EncryptionInfo, EncryptionScheme, inspect_encryption};
pub use error::Error;
pub use key::{ContentKey, KeyId, KeySet};
pub use license::{
    BoxFuture, ContentType, DrmProvider, DrmRequest, LicenseRequest, LicenseResponse,
    LicenseTransport, acquire_track_keys,
};
#[cfg(feature = "playready")]
pub use playready::PlayReadyProvider;
#[cfg(feature = "widevine")]
pub use widevine::WidevineProvider;
