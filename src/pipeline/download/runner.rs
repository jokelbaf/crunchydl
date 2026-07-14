//! Download request resolution and staging orchestration.

use super::*;

pub(crate) async fn run(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: DownloadRequest,
) -> Result<DownloadResult, Error> {
    let cancellation = request.cancellation.clone();
    let states = StateEmitter::new(runtime.events.clone(), cancellation.clone());
    let result = run_inner(api, crunchyroll, platform, runtime, request, &states).await;
    match result {
        Ok(value) => {
            runtime
                .events
                .emit(DownloadEvent::StateChanged(JobState::Completed));
            Ok(value)
        }
        Err(Error::Cancelled) => {
            runtime
                .events
                .emit(DownloadEvent::StateChanged(JobState::Cancelled));
            Err(Error::Cancelled)
        }
        Err(error) => {
            runtime
                .events
                .emit(DownloadEvent::StateChanged(JobState::Failed));
            Err(error)
        }
    }
}

pub(crate) async fn run_inner(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: DownloadRequest,
    states: &StateEmitter,
) -> Result<DownloadResult, Error> {
    let cancellation = request.cancellation.clone();
    states.enter(JobState::Created)?;
    states.enter(JobState::ResolvingMedia)?;
    let media = request.media.resolve();
    validate_media(&media)?;
    let archive_key = ArchiveKey {
        media_id: media.content_id.clone(),
        selection_fingerprint: crate::redaction::fingerprint([
            format!("{:?}", request.planning),
            format!("{:?}", request.subtitles),
            format!("{:?}", request.synchronization),
            format!("{:?}", request.output.container),
        ]),
    };
    if let Some(archive) = &runtime.archive
        && let Some(entry) = archive.find(&archive_key)?
    {
        return Ok(DownloadResult {
            output: entry.output,
            media_id: media.content_id,
            tracks: entry.tracks,
            warnings: Vec::new(),
        });
    }
    std::fs::create_dir_all(&request.output.root)
        .map_err(|error| path_error(&request.output.root, error))?;

    states.enter(JobState::OpeningPlaybackSessions)?;
    let mut guard = SessionGuard::new(api);
    states.enter(JobState::PlanningTracks)?;
    let prepared = crate::plan::prepare_with_guard(
        api,
        platform,
        &media,
        &request.planning,
        &cancellation,
        &mut guard,
    )
    .await;
    let result = match prepared {
        Ok(prepared) => {
            run_prepared(
                api,
                crunchyroll,
                platform,
                runtime,
                &request,
                &media,
                &archive_key,
                prepared,
                states,
            )
            .await
        }
        Err(error) => Err(error),
    };
    let finalization = guard.finalize().await;
    match (result, finalization) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) | (Err(error), _) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_prepared(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: &DownloadRequest,
    media: &ResolvedMedia,
    archive_key: &ArchiveKey,
    prepared: PreparedPlan,
    states: &StateEmitter,
) -> Result<DownloadResult, Error> {
    validate_container_contract(request, &prepared)?;
    for warning in &prepared.public.warnings {
        runtime.events.emit(DownloadEvent::Warning(warning.clone()));
    }
    let height = prepared
        .public
        .tracks
        .iter()
        .filter_map(|track| track.dimensions.map(|(_, height)| height))
        .max();
    let audio = prepared
        .public
        .tracks
        .iter()
        .filter(|track| track.kind == PlannedTrackKind::Audio)
        .map(|track| track.locale.to_string())
        .collect::<Vec<_>>();
    let mut relative = request.output.layout.as_ref().map_or_else(
        || {
            request
                .output
                .filename
                .render(media, height, &audio, request.output.max_component_length)
                .into()
        },
        |layout| layout.render(media, height, &audio, request.output.max_component_length),
    );
    relative.set_extension(match request.output.container {
        crate::Container::Matroska => "mkv",
        crate::Container::Mp4 => "mp4",
    });
    let destination = output_path(&request.output.root, &relative)?;
    if destination.exists() && request.output.overwrite != OverwritePolicy::Replace {
        return Err(Error::Filesystem(format!(
            "output already exists: {}",
            destination.display()
        )));
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| path_error(parent, error))?;
    }
    let staging = request
        .output
        .root
        .join(".crunchydl-staging")
        .join(safe_component(&media.content_id))
        .join(&prepared.public.fingerprint[..16]);
    std::fs::create_dir_all(&staging).map_err(|error| path_error(&staging, error))?;
    let outcome = execute_pipeline(
        api,
        crunchyroll,
        platform,
        runtime,
        request,
        media,
        archive_key,
        &prepared,
        &staging,
        &destination,
        states,
    )
    .await;
    cleanup_staging(&staging, request.output.retention, outcome.is_ok());
    outcome
}
