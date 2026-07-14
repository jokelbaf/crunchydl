//! Durable queue execution and per-item download requests.

use super::*;

pub(crate) async fn run_queue(paths: &AppPaths, only: Option<&[uuid::Uuid]>) -> Result<()> {
    let progress = Arc::new(CliProgress::new());
    let sink: Arc<dyn crunchydl::EventSink> = progress.clone();
    let cancellation = crunchydl::CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    let signal = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            signal_cancellation.cancel();
        }
    });
    let result = run_queue_inner(
        paths,
        only,
        true,
        sink,
        cancellation,
        Some(progress.clone()),
    )
    .await;
    progress.finish();
    signal.abort();
    result
}

pub(crate) async fn run_queue_with_sink(
    paths: &AppPaths,
    sink: Arc<dyn crunchydl::EventSink>,
    cancellation: crunchydl::CancellationToken,
) -> Result<()> {
    run_queue_inner(paths, None, false, sink, cancellation, None).await
}

async fn run_queue_inner(
    paths: &AppPaths,
    only: Option<&[uuid::Uuid]>,
    terminal_output: bool,
    sink: Arc<dyn crunchydl::EventSink>,
    cancellation: crunchydl::CancellationToken,
    progress: Option<Arc<CliProgress>>,
) -> Result<()> {
    let mut queue = Queue::load(&paths.queue)?;
    let mut pending = queue.pending();
    if let Some(ids) = only {
        pending.retain(|item| ids.contains(&item.id));
    }
    if pending.is_empty() {
        if terminal_output {
            print_warning("There are no pending downloads.");
        }
        return Ok(());
    }
    let config = Config::load(paths)?;
    let client = crate::auth::restore(paths).await?;
    let downloader = downloader(&client, &config, paths, sink).await?;
    let mut failed = 0;
    let mut last_error = None;
    let total = pending.len();
    for (index, item) in pending.into_iter().enumerate() {
        if cancellation.is_cancelled() {
            return Err(Error::Download(crunchydl::Error::Cancelled));
        }
        queue.mark_running(item.id)?;
        if let Some(progress) = &progress {
            progress.start(&item, index + 1, total);
        }
        let result = match downloader.media_request(&item.target).await {
            Ok(media) => {
                queue.set_title(item.id, media.resolve().title)?;
                download_item(&downloader, &config, &item, media, &cancellation).await
            }
            Err(error) => Err(Error::Download(error)),
        };
        match result {
            Ok(output) => {
                if let Some(progress) = &progress {
                    progress.success(&output);
                }
                queue.mark_completed(item.id, output)?;
            }
            Err(error) => {
                if cancellation.is_cancelled() {
                    queue.mark_pending(item.id)?;
                    return Err(error);
                }
                if let Some(progress) = &progress {
                    progress.failure(&error.to_string());
                }
                queue.mark_failed(item.id, &error.to_string())?;
                last_error = Some(error);
                failed += 1;
            }
        }
    }
    match (failed, last_error) {
        (0, _) => Ok(()),
        (1, Some(error)) => Err(error),
        (count, _) => Err(Error::QueueFailed(count)),
    }
}

async fn download_item(
    downloader: &crunchydl::Downloader,
    config: &Config,
    item: &QueueItem,
    media: crunchydl::MediaRequest,
    cancellation: &crunchydl::CancellationToken,
) -> Result<PathBuf> {
    let audio = if item.selection.all_audio {
        crunchydl::AudioSelection::All
    } else if item.selection.audio_locales.is_empty() {
        crunchydl::AudioSelection::Original
    } else {
        let locales = parse_locales(item.selection.audio_locales.clone())?;
        if locales.len() == 1 {
            crunchydl::AudioSelection::Locale(locales[0].clone())
        } else {
            crunchydl::AudioSelection::Locales(locales)
        }
    };
    let subtitle_locales = if item.selection.no_subtitles {
        crunchydl::SubtitleLocales::None
    } else if item.selection.subtitle_locales.is_empty() {
        crunchydl::SubtitleLocales::All
    } else {
        crunchydl::SubtitleLocales::Explicit(parse_locales(
            item.selection.subtitle_locales.clone(),
        )?)
    };
    let planning = crunchydl::PlanningOptions {
        audio,
        subtitles: crunchydl::SubtitleSelection::default().with_locales(subtitle_locales),
        video_quality: item.selection.max_height.map_or(
            crunchydl::QualitySelection::Best,
            crunchydl::QualitySelection::MaxHeight,
        ),
        chapters: if item.selection.no_chapters {
            crunchydl::ChapterSelection::Disabled
        } else {
            crunchydl::ChapterSelection::SkipEvents
        },
        ..crunchydl::PlanningOptions::default()
    };
    let mut output =
        crunchydl::OutputOptions::new(&config.output_dir).map_err(|_| Error::InvalidTemplate)?;
    output.filename = crunchydl::FilenameTemplate::compile(&config.filename)
        .map_err(|_| Error::InvalidTemplate)?;
    output.layout = config
        .output_layout
        .as_deref()
        .map(crunchydl::OutputLayoutTemplate::compile)
        .transpose()
        .map_err(|_| Error::InvalidTemplate)?;
    if item.selection.replace {
        output.overwrite = crunchydl::OverwritePolicy::Replace;
    }
    output.container = match item.selection.format {
        QueueFormat::Matroska => crunchydl::Container::Matroska,
        QueueFormat::Mp4 => crunchydl::Container::Mp4,
    };
    let request = crunchydl::DownloadRequest {
        media,
        planning,
        output,
        transfer: crunchydl::TransferOptions::default(),
        subtitles: crunchydl::SubtitleProcessingOptions::default(),
        synchronization: crunchydl::SynchronizationOptions::default(),
        cancellation: cancellation.clone(),
    };
    downloader
        .download(request)
        .await
        .map(|result| result.output)
        .map_err(Error::Download)
}
