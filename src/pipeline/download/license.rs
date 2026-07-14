//! DRM endpoint selection and playback refresh.

use super::*;

pub(crate) struct PlaybackRefresher<'a> {
    api: &'a ProductionApi,
    platform: &'a StreamPlatform,
    media: &'a ResolvedMedia,
    options: &'a PlanningOptions,
    cancellation: &'a CancellationToken,
    sessions: tokio::sync::Mutex<Vec<SessionGuard<'a, ProductionApi>>>,
}

impl<'a> PlaybackRefresher<'a> {
    pub(crate) fn new(
        api: &'a ProductionApi,
        platform: &'a StreamPlatform,
        media: &'a ResolvedMedia,
        options: &'a PlanningOptions,
        cancellation: &'a CancellationToken,
    ) -> Self {
        Self {
            api,
            platform,
            media,
            options,
            cancellation,
            sessions: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) async fn finalize(&self) -> Result<(), Error> {
        let sessions = {
            let mut sessions = self.sessions.lock().await;
            std::mem::take(&mut *sessions)
        };
        let mut first_error = None;
        for guard in sessions {
            if let Err(error) = guard.finalize().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl crate::RepresentationRefresher for PlaybackRefresher<'_> {
    fn refresh<'a>(
        &'a self,
        expired: &'a crate::RepresentationTransferPlan,
    ) -> Pin<Box<dyn Future<Output = Result<crate::RepresentationTransferPlan, Error>> + Send + 'a>>
    {
        Box::pin(async move {
            let mut guard = SessionGuard::new(self.api);
            let prepared = match crate::plan::prepare_with_guard(
                self.api,
                self.platform,
                self.media,
                self.options,
                self.cancellation,
                &mut guard,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    let _ = guard.finalize().await;
                    return Err(error);
                }
            };
            let source = prepared
                .sources
                .iter()
                .find(|source| {
                    source.diagnostic.version_id == expired.version_id
                        && source.diagnostic.representation_fingerprint
                            == expired.representation_fingerprint
                })
                .ok_or_else(|| {
                    Error::ResumeMismatch(
                        "refreshed playback omitted the selected representation".into(),
                    )
                })?;
            let refreshed =
                source_transfer_plan(source, &expired.plan_fingerprint, &expired.media_id);
            self.sessions.lock().await.push(guard);
            Ok(refreshed)
        })
    }
}

pub(crate) fn select_pssh(
    drm: &crunchyroll_rs::media::MediaStreamDRM,
    system: DrmSystem,
) -> Result<Vec<u8>, Error> {
    let encoded = drm
        .types
        .iter()
        .find_map(|kind| match (system, kind) {
            (DrmSystem::PlayReady, MediaStreamDRMType::Playready { pro, pssh }) => pssh
                .as_ref()
                .and_then(|values| values.first())
                .or(pro.as_ref()),
            (DrmSystem::Widevine, MediaStreamDRMType::Widevine { pssh }) => pssh.first(),
            _ => None,
        })
        .ok_or_else(|| Error::License(drm::Error::License))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| Error::License(drm::Error::License))
}

pub(crate) fn license_endpoint(
    system: DrmSystem,
    endpoint_override: Option<&str>,
    playback_drm: &StreamDrm,
) -> Result<String, Error> {
    let endpoint = endpoint_override.unwrap_or(&playback_drm.drm_url).trim();
    if endpoint.is_empty() {
        return Err(Error::License(drm::Error::License));
    }
    if endpoint_override.is_some() || system.matches_name(playback_drm.name.trim()) {
        return Ok(endpoint.to_string());
    }
    let (base, advertised_name) = endpoint
        .rsplit_once('/')
        .ok_or_else(|| Error::License(drm::Error::License))?;
    if playback_drm.name.trim().is_empty()
        || !advertised_name.eq_ignore_ascii_case(playback_drm.name.trim())
    {
        return Err(Error::License(drm::Error::License));
    }
    Ok(format!("{base}/{}", system.endpoint_name()))
}
