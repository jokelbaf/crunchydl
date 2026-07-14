//! Account login and refresh-token persistence.

use std::fs;

use crunchyroll_rs::Crunchyroll;
use crunchyroll_rs::crunchyroll::{DeviceIdentifier, SessionToken};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::atomic_write;
use crate::error::{Error, Result};
use crate::paths::AppPaths;

const SCHEMA_VERSION: u32 = 1;
const KEYRING_SERVICE: &str = "dev.jokelbaf.crunchydl";
const KEYRING_ACCOUNT: &str = "default";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionMetadata {
    schema_version: u32,
    device_id: String,
    device_type: String,
    device_name: Option<String>,
}

impl SessionMetadata {
    fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            device_id: uuid::Uuid::new_v4().to_string(),
            device_type: "ANDROIDTV".to_string(),
            device_name: Some("crunchydl CLI".to_string()),
        }
    }

    fn device(&self) -> DeviceIdentifier {
        DeviceIdentifier {
            device_id: self.device_id.clone(),
            device_type: self.device_type.clone(),
            device_name: self.device_name.clone(),
        }
    }
}

pub(crate) async fn login(
    paths: &AppPaths,
    email: &str,
    password: Zeroizing<String>,
) -> Result<Crunchyroll> {
    let metadata = load_metadata(paths)?.unwrap_or_else(SessionMetadata::new);
    let client = Crunchyroll::builder()
        .login_with_credentials(email, password.as_str(), metadata.device())
        .await
        .map_err(|_| Error::LoginFailed)?;
    persist_session(paths, &client, &metadata).await?;
    Ok(client)
}

pub(crate) async fn restore(paths: &AppPaths) -> Result<Crunchyroll> {
    let metadata = load_metadata(paths)?.ok_or(Error::NotLoggedIn)?;
    let token = read_secret().await?;
    let client = Crunchyroll::builder()
        .login_with_refresh_token(token.as_str(), metadata.device())
        .await
        .map_err(|_| Error::SessionExpired)?;
    persist_session(paths, &client, &metadata).await?;
    Ok(client)
}

pub(crate) async fn logout(paths: &AppPaths) -> Result<bool> {
    let deleted_secret = tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|_| Error::CredentialStore)?;
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(Error::CredentialStore),
        }
    })
    .await
    .map_err(|_| Error::CredentialStore)??;
    let deleted_metadata = match fs::remove_file(&paths.session) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(Error::Filesystem {
                operation: "remove session metadata",
                path: paths.session.clone(),
            });
        }
    };
    Ok(deleted_secret || deleted_metadata)
}

async fn persist_session(
    paths: &AppPaths,
    client: &Crunchyroll,
    metadata: &SessionMetadata,
) -> Result<()> {
    let token = match client.session_token().await {
        SessionToken::RefreshToken(token) => Zeroizing::new(token),
        SessionToken::EtpRt(_) | SessionToken::Anonymous => return Err(Error::LoginFailed),
    };
    write_secret(token).await?;
    let document = serde_json::to_vec_pretty(metadata).map_err(|_| Error::Filesystem {
        operation: "serialize session metadata",
        path: paths.session.clone(),
    })?;
    atomic_write(&paths.session, &document)
}

async fn read_secret() -> Result<Zeroizing<String>> {
    tokio::task::spawn_blocking(|| {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|_| Error::CredentialStore)?;
        entry
            .get_password()
            .map(Zeroizing::new)
            .map_err(|error| match error {
                keyring::Error::NoEntry => Error::NotLoggedIn,
                _ => Error::CredentialStore,
            })
    })
    .await
    .map_err(|_| Error::CredentialStore)?
}

async fn write_secret(secret: Zeroizing<String>) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|_| Error::CredentialStore)?;
        entry
            .set_password(secret.as_str())
            .map_err(|_| Error::CredentialStore)
    })
    .await
    .map_err(|_| Error::CredentialStore)?
}

fn load_metadata(paths: &AppPaths) -> Result<Option<SessionMetadata>> {
    let bytes = match fs::read(&paths.session) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(Error::Filesystem {
                operation: "read session metadata",
                path: paths.session.clone(),
            });
        }
    };
    let metadata: SessionMetadata =
        serde_json::from_slice(&bytes).map_err(|_| Error::InvalidConfig {
            path: paths.session.clone(),
            message: "invalid session metadata; log in again".to_string(),
        })?;
    if metadata.schema_version != SCHEMA_VERSION {
        return Err(Error::InvalidConfig {
            path: paths.session.clone(),
            message: "unsupported session schema; log in again".to_string(),
        });
    }
    Ok(Some(metadata))
}
