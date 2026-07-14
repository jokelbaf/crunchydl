//! PlayReady backend.

use playready::{Cdm, Device, Pssh};

use crate::{
    BoxFuture, ContentKey, DrmProvider, DrmRequest, Error, KeyId, KeySet, LicenseRequest,
    LicenseTransport,
};

/// PlayReady provider backed by caller-supplied `.prd` device bytes.
///
/// Device contents are never exposed through formatting or serialization.
pub struct PlayReadyProvider {
    cdm: Cdm,
}

impl PlayReadyProvider {
    /// Parse and verify PlayReady device bytes.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Device`] when the device is malformed or fails
    /// certificate/key verification.
    pub fn from_device_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let device = Device::from_bytes(bytes).map_err(|_| Error::Device)?;
        device.verify().map_err(|_| Error::Device)?;
        Ok(Self {
            cdm: Cdm::from_device(device),
        })
    }
}

impl DrmProvider for PlayReadyProvider {
    fn acquire_keys<'a>(
        &'a self,
        request: DrmRequest,
        transport: &'a dyn LicenseTransport,
    ) -> BoxFuture<'a, Result<KeySet, Error>> {
        Box::pin(async move {
            let pssh = Pssh::from_bytes(&request.pssh)
                .map_err(|_| Error::Initialization("invalid PlayReady PSSH".to_string()))?;
            let wrm_header = pssh.wrm_headers().into_iter().next().ok_or_else(|| {
                Error::Initialization("PlayReady PSSH has no WRM header".to_string())
            })?;
            let session = self.cdm.open_session();
            let challenge = session
                .get_license_challenge(wrm_header)
                .map_err(|_| Error::Challenge)?;

            let challenge = if challenge.starts_with("<?xml") {
                challenge.into_bytes()
            } else {
                format!("<?xml version=\"1.0\" encoding=\"utf-8\"?>{challenge}").into_bytes()
            };
            let response = transport
                .send(LicenseRequest {
                    endpoint: request.endpoint,
                    content_id: request.content_id,
                    playback_token: request.playback_token,
                    content_type: request.content_type,
                    challenge,
                })
                .await?;
            let response = std::str::from_utf8(response.as_bytes()).map_err(|_| Error::License)?;
            let backend_keys = session
                .get_keys_from_challenge_response(response)
                .map_err(|_| Error::License)?;
            let mut keys = KeySet::new();
            for (kid, key) in backend_keys {
                let kid: [u8; 16] = kid.into();
                keys.insert(KeyId::new(kid), ContentKey::try_from_slice(key.as_ref())?);
            }
            if keys.is_empty() {
                return Err(Error::License);
            }
            Ok(keys)
        })
    }
}
