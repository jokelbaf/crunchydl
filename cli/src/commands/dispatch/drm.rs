//! CLI DRM provider construction.

use super::*;

pub(crate) async fn downloader(
    client: &Crunchyroll,
    config: &Config,
    paths: &AppPaths,
    events: Arc<dyn crunchydl::EventSink>,
) -> Result<crunchydl::Downloader> {
    let device_path = config.drm_device.as_ref().ok_or(Error::DrmNotConfigured)?;
    let backend = config.drm_backend.resolve(device_path)?;
    let bytes = tokio::fs::read(device_path)
        .await
        .map_err(|_| Error::InvalidDrmDevice)?;
    let (provider, system): (Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem) = match backend
    {
        DrmBackend::Auto => unreachable!("auto backend was resolved above"),
        DrmBackend::PlayReady => playready_provider(&bytes)?,
        DrmBackend::Widevine => widevine_provider(&bytes)?,
    };
    let archive = Arc::new(crunchydl::JsonArchive::new(&paths.archive));
    let builder = crunchydl::Downloader::builder(client.clone());
    let builder = if let Some(endpoint) = config
        .license_endpoint
        .as_deref()
        .filter(|endpoint| !endpoint.trim().is_empty())
    {
        builder.drm_with_license_endpoint(provider, system, endpoint)
    } else {
        builder.drm(provider, system)
    };
    Ok(builder.archive(archive).event_sink(events).build())
}

#[cfg(feature = "drm-playready")]
fn playready_provider(
    bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    let provider = crunchydl::PlayReadyProvider::from_device_bytes(bytes)
        .map_err(|_| Error::InvalidDrmDevice)?;
    Ok((Arc::new(provider), crunchydl::DrmSystem::PlayReady))
}

#[cfg(not(feature = "drm-playready"))]
fn playready_provider(
    _bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    Err(Error::DrmNotCompiled("PlayReady"))
}

#[cfg(feature = "drm-widevine")]
fn widevine_provider(
    bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    let provider = crunchydl::WidevineProvider::from_device_bytes(bytes)
        .map_err(|_| Error::InvalidDrmDevice)?;
    Ok((Arc::new(provider), crunchydl::DrmSystem::Widevine))
}

#[cfg(not(feature = "drm-widevine"))]
fn widevine_provider(
    _bytes: &[u8],
) -> Result<(Arc<dyn crunchydl::DrmProvider>, crunchydl::DrmSystem)> {
    Err(Error::DrmNotCompiled("Widevine"))
}
