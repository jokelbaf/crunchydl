//! Optional GPL-3.0 Widevine backend.

use widevine::{Cdm, Device, KeyType, LicenseType, Pssh};

use crate::{
    BoxFuture, ContentKey, DrmProvider, DrmRequest, Error, KeyId, KeySet, LicenseRequest,
    LicenseTransport,
};

/// Widevine provider backed by caller-supplied `.wvd` device bytes.
///
/// Enabling this provider pulls the GPL-3.0 `widevine` dependency.
pub struct WidevineProvider {
    cdm: Cdm,
}

impl WidevineProvider {
    /// Parse Widevine `.wvd` bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Device`] when the device is malformed.
    pub fn from_device_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let device = Device::read_wvd(std::io::Cursor::new(bytes)).map_err(|_| Error::Device)?;
        Ok(Self {
            cdm: Cdm::new(device),
        })
    }
}

impl DrmProvider for WidevineProvider {
    fn acquire_keys<'a>(
        &'a self,
        request: DrmRequest,
        transport: &'a dyn LicenseTransport,
    ) -> BoxFuture<'a, Result<KeySet, Error>> {
        Box::pin(async move {
            let pssh = Pssh::from_bytes(&request.pssh)
                .map_err(|_| Error::Initialization("invalid Widevine PSSH".to_string()))?;
            let license_request = self
                .cdm
                .open()
                .get_license_request(pssh, LicenseType::STREAMING)
                .map_err(|_| Error::Challenge)?;
            let challenge = license_request.challenge().map_err(|_| Error::Challenge)?;
            let response = transport
                .send(LicenseRequest {
                    endpoint: request.endpoint,
                    content_id: request.content_id,
                    playback_token: request.playback_token,
                    content_type: request.content_type,
                    challenge,
                })
                .await?;
            let backend_keys = license_request
                .get_keys(response.as_bytes())
                .map_err(|_| Error::License)?;
            let mut keys = KeySet::new();
            for key in backend_keys.of_type(KeyType::CONTENT) {
                keys.insert(KeyId::new(key.kid), ContentKey::try_from_slice(&key.key)?);
            }
            if keys.is_empty() {
                return Err(Error::License);
            }
            Ok(keys)
        })
    }
}
