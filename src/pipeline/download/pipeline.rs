//! Transfer, decryption, subtitle processing, and commit execution.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_pipeline(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    platform: &StreamPlatform,
    runtime: &RuntimeConfiguration,
    request: &DownloadRequest,
    media: &ResolvedMedia,
    archive_key: &ArchiveKey,
    prepared: &PreparedPlan,
    staging: &Path,
    destination: &Path,
    states: &StateEmitter,
) -> Result<DownloadResult, Error> {
    let refresher = PlaybackRefresher::new(
        api,
        platform,
        media,
        &request.planning,
        &request.cancellation,
    );
    let result = execute_pipeline_inner(
        api,
        crunchyroll,
        runtime,
        request,
        media,
        archive_key,
        prepared,
        staging,
        destination,
        states,
        &refresher,
    )
    .await;
    let finalization = refresher.finalize().await;
    match (result, finalization) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) | (Err(error), _) => Err(error),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_pipeline_inner(
    api: &ProductionApi,
    crunchyroll: &crunchyroll_rs::Crunchyroll,
    runtime: &RuntimeConfiguration,
    request: &DownloadRequest,
    media: &ResolvedMedia,
    archive_key: &ArchiveKey,
    prepared: &PreparedPlan,
    staging: &Path,
    destination: &Path,
    states: &StateEmitter,
    refresher: &PlaybackRefresher<'_>,
) -> Result<DownloadResult, Error> {
    let transfer = TransferEngine::with_client(crunchyroll.client(), request.transfer.clone())?
        .event_sink(runtime.events.clone());
    let transport = CrunchyrollLicenseTransport::new(crunchyroll.clone());
    let mut warnings = prepared.public.warnings.clone();
    states.enter(JobState::AcquiringLicenses)?;
    let mut licensed = Vec::with_capacity(prepared.sources.len());
    for (index, source) in prepared.sources.iter().enumerate() {
        let transfer_plan =
            source_transfer_plan(source, &prepared.public.fingerprint, &media.content_id);
        let init_path = transfer
            .transfer_init(&transfer_plan, staging, &request.cancellation)
            .await?;
        let init = std::fs::read(&init_path).map_err(|error| path_error(&init_path, error))?;
        let key = if let Some(drm) = &source.drm {
            let config = runtime
                .drm
                .as_ref()
                .ok_or_else(|| Error::License(drm::Error::License))?;
            let info = inspect_encryption(&init)?;
            let pssh = select_pssh(drm, config.system)?;
            let endpoint = license_endpoint(
                config.system,
                config.endpoint_override.as_deref(),
                &source.playback_drm,
            )?;
            let request_data = DrmRequest {
                endpoint,
                content_id: source.diagnostic.version_id.clone(),
                playback_token: drm.token.clone(),
                content_type: match source.diagnostic.kind {
                    PlannedTrackKind::Video => ContentType::Video,
                    PlannedTrackKind::Audio => ContentType::Audio,
                },
                pssh,
            };
            let keys = config
                .provider
                .acquire_keys(request_data, &transport)
                .await?;
            Some(keys.require(info.default_kid)?.clone())
        } else {
            None
        };
        licensed.push((source, transfer_plan, key));
        runtime.events.emit(DownloadEvent::StageProgress {
            state: JobState::AcquiringLicenses,
            completed: index + 1,
            total: prepared.sources.len(),
        });
    }

    states.enter(JobState::Downloading)?;
    let mut transferred = Vec::with_capacity(licensed.len());
    let licensed_count = licensed.len();
    for (index, (source, transfer_plan, key)) in licensed.into_iter().enumerate() {
        let result = transfer
            .transfer_with_refresh(transfer_plan, staging, &request.cancellation, refresher)
            .await?;
        transferred.push((source, result, key));
        runtime.events.emit(DownloadEvent::StageProgress {
            state: JobState::Downloading,
            completed: index + 1,
            total: licensed_count,
        });
    }

    states.enter(JobState::Decrypting)?;
    let mut assembled = Vec::with_capacity(transferred.len());
    let transferred_count = transferred.len();
    for (index, (source, transferred, key)) in transferred.into_iter().enumerate() {
        let output = staging.join(format!("track-{index:03}.mp4"));
        assemble_track(
            &transferred.init,
            &transferred.segments,
            &output,
            key.as_ref(),
            &request.cancellation,
        )?;
        let file = File::open(&output).map_err(|error| path_error(&output, error))?;
        let parsed = FragmentedMp4::open(BufReader::new(file))?;
        if parsed.tracks().len() != 1 {
            return Err(Error::MediaParse(media::Error::Unsupported(
                "assembled representation must contain one track",
            )));
        }
        assembled.push((output, source.diagnostic.clone()));
        runtime.events.emit(DownloadEvent::StageProgress {
            state: JobState::Decrypting,
            completed: index + 1,
            total: transferred_count,
        });
    }

    states.enter(JobState::ProcessingSubtitles)?;
    let subtitles = download_selected(api, &prepared.subtitles, &request.subtitles).await?;
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::ProcessingSubtitles,
        completed: subtitles.len(),
        total: prepared.subtitles.len(),
    });
    let missing = MissingFontResolver;
    let resolver: &dyn FontResolver = runtime.font_resolver.as_deref().unwrap_or(&missing);
    let (fonts, font_warnings) =
        crate::resolve_referenced_fonts(&subtitles, resolver, runtime.font_policy)?;
    for warning in &font_warnings {
        runtime.events.emit(DownloadEvent::Warning(warning.clone()));
    }
    warnings.extend(font_warnings);

    states.enter(JobState::Muxing)?;
    let temporary = staging.join(match request.output.container {
        crate::Container::Matroska => "output.mkv.part",
        crate::Container::Mp4 => "output.mp4.part",
    });
    let output_tracks = match request.output.container {
        crate::Container::Matroska => mux_matroska(
            &temporary,
            &assembled,
            &subtitles,
            &fonts,
            &prepared.public.chapters,
            media,
            &request.synchronization,
            &request.cancellation,
        )?,
        crate::Container::Mp4 => crate::mp4_output::write_and_verify(
            &temporary,
            &assembled,
            &request.synchronization,
            &request.cancellation,
        )?,
    };
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::Muxing,
        completed: 1,
        total: 1,
    });

    states.enter(JobState::Verifying)?;
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::Verifying,
        completed: 1,
        total: 1,
    });
    states.enter(JobState::Committing)?;
    commit_output(
        &temporary,
        destination,
        request.output.overwrite,
        runtime.archive.as_deref(),
        archive_key,
        &output_tracks,
    )?;
    runtime.events.emit(DownloadEvent::StageProgress {
        state: JobState::Committing,
        completed: 1,
        total: 1,
    });
    runtime.events.emit(DownloadEvent::OutputCommitted {
        output: destination.to_path_buf(),
    });
    Ok(DownloadResult {
        output: destination.to_path_buf(),
        media_id: media.content_id.clone(),
        tracks: output_tracks,
        warnings,
    })
}
