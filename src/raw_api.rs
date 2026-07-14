//! Narrow raw Crunchyroll operations not exposed by `crunchyroll-rs`.

use crunchyroll_rs::Crunchyroll;
use drm::{BoxFuture, Error as DrmError, LicenseRequest, LicenseResponse, LicenseTransport};
use reqwest::header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, PRAGMA};

/// License transport using the authenticated client and current access token
/// from `crunchyroll-rs`.
///
/// The endpoint is supplied from `crunchyroll-rs` playback metadata, or from
/// an explicit caller override used for tests and proxies.
pub struct CrunchyrollLicenseTransport {
    crunchyroll: Crunchyroll,
}

impl CrunchyrollLicenseTransport {
    /// Construct a transport from the same authenticated client used for
    /// playback.
    #[must_use]
    pub fn new(crunchyroll: Crunchyroll) -> Self {
        Self { crunchyroll }
    }
}

impl LicenseTransport for CrunchyrollLicenseTransport {
    fn send<'a>(
        &'a self,
        request: LicenseRequest,
    ) -> BoxFuture<'a, Result<LicenseResponse, DrmError>> {
        Box::pin(async move {
            let access_token = self.crunchyroll.access_token().await;
            let response = self
                .crunchyroll
                .client()
                .post(&request.endpoint)
                .header(AUTHORIZATION, format!("Bearer {access_token}"))
                .header(CONTENT_TYPE, "application/octet-stream")
                .header(PRAGMA, "no-cache")
                .header(CACHE_CONTROL, "no-cache")
                .header("x-cr-content-id", &request.content_id)
                .header("x-cr-video-token", &request.playback_token)
                .body(request.challenge)
                .send()
                .await
                .map_err(|_| DrmError::Transport("request failed".to_string()))?;
            let status = response.status();
            if !status.is_success() {
                return Err(DrmError::Transport(format!(
                    "server returned HTTP {}",
                    status.as_u16()
                )));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response
                .bytes()
                .await
                .map_err(|_| DrmError::Transport("response body failed".to_string()))?
                .to_vec();
            LicenseResponse::from_http(content_type.as_deref(), body)
        })
    }
}
