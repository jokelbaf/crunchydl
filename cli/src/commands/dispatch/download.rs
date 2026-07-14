//! Download and queue creation commands.

use super::*;

pub(crate) async fn download(paths: &AppPaths, arguments: DownloadArguments) -> Result<()> {
    validate_format_arguments(&arguments)?;
    let client = crate::auth::restore(paths).await?;
    let targets = expand_targets(&client, &arguments).await?;
    let selection = QueueSelection {
        audio_locales: arguments.audio_locales,
        all_audio: arguments.all_audio,
        subtitle_locales: arguments.subtitle_locales,
        no_subtitles: arguments.no_subtitles,
        max_height: arguments.max_height,
        replace: arguments.replace,
        no_chapters: arguments.no_chapters,
        format: arguments.format,
    };
    let mut queue = Queue::load(&paths.queue)?;
    let ids = queue.add(targets, selection)?;
    run_queue(paths, Some(&ids)).await
}

pub(crate) async fn queue(paths: &AppPaths, arguments: QueueArguments) -> Result<()> {
    match arguments.command {
        QueueCommand::Add(arguments) => {
            validate_format_arguments(&arguments)?;
            let client = crate::auth::restore(paths).await?;
            let targets = expand_targets(&client, &arguments).await?;
            let selection = QueueSelection {
                audio_locales: arguments.audio_locales,
                all_audio: arguments.all_audio,
                subtitle_locales: arguments.subtitle_locales,
                no_subtitles: arguments.no_subtitles,
                max_height: arguments.max_height,
                replace: arguments.replace,
                no_chapters: arguments.no_chapters,
                format: arguments.format,
            };
            let mut queue = Queue::load(&paths.queue)?;
            let ids = queue.add(targets, selection)?;
            print_success(&format!(
                "Added {} item(s) to the download queue.",
                ids.len()
            ));
            Ok(())
        }
        QueueCommand::Run => run_queue(paths, None).await,
        QueueCommand::List => {
            let queue = Queue::load(&paths.queue)?;
            if queue.items().is_empty() {
                print_warning("The download queue is empty.");
                return Ok(());
            }
            let colors = io::stdout().is_terminal();
            print_list_heading("Download queue", queue.items().len(), colors);
            print_queue_summary(queue.items(), colors);
            for item in queue.items() {
                print_queue_item(item, colors);
            }
            print_actions(
                &[
                    ("Run pending", "crunchydl queue run"),
                    ("Retry", "crunchydl queue retry [JOB_ID]"),
                    ("Remove", "crunchydl queue remove <JOB_ID>"),
                ],
                colors,
            );
            Ok(())
        }
        QueueCommand::Retry { id } => {
            let mut queue = Queue::load(&paths.queue)?;
            if let Some(id) = id {
                if queue.retry(id)? {
                    print_success("Moved the selected item back to pending.");
                } else {
                    print_warning("That queue item is not failed, so it was left unchanged.");
                }
            } else {
                let count = queue.retry_failed()?;
                print_success(&format!("Moved {count} failed item(s) back to pending."));
            }
            Ok(())
        }
        QueueCommand::Remove { id } => {
            let mut queue = Queue::load(&paths.queue)?;
            if queue.remove(id)? {
                print_success("Removed the selected queue item.");
            } else {
                print_warning(
                    "The item was not removed; it may be downloading or no longer exist.",
                );
            }
            Ok(())
        }
        QueueCommand::ClearCompleted => {
            let mut queue = Queue::load(&paths.queue)?;
            let count = queue.clear_completed()?;
            print_success(&format!("Removed {count} completed item(s)."));
            Ok(())
        }
    }
}

fn print_queue_summary(items: &[QueueItem], colors: bool) {
    let states = [
        (QueueState::Running, "downloading"),
        (QueueState::Pending, "pending"),
        (QueueState::Failed, "failed"),
        (QueueState::Completed, "completed"),
    ];
    let summary = states
        .into_iter()
        .filter_map(|(state, label)| {
            let count = items.iter().filter(|item| item.state == state).count();
            (count > 0).then(|| format!("{count} {label}"))
        })
        .collect::<Vec<_>>()
        .join("  ·  ");
    println!("  {}\n", paint(&summary, "2", colors));
}

fn print_queue_item(item: &QueueItem, colors: bool) {
    let kind = crate::presentation::target_kind_label(&item.target);
    let fallback_title = format!("{kind} {}", item.target.id());
    let title = item.title.as_deref().unwrap_or(&fallback_title);
    let status_width = match item.state {
        QueueState::Pending => 9,
        QueueState::Running => 13,
        QueueState::Completed => 11,
        QueueState::Failed => 8,
    };
    let title_width = terminal_width().saturating_sub(status_width + 5).max(16);
    println!(
        "  {}  {}",
        queue_state_style(item.state, colors),
        paint(&ellipsize(title, title_width), "1", colors)
    );
    println!(
        "     {}",
        paint(&format!("{kind}  ·  {}", item.target.id()), "2", colors)
    );
    println!("     {}", paint(&selection_label(item), "36", colors));
    println!("     {}", paint(&format!("Job  {}", item.id), "2", colors));

    let detail_width = terminal_width().saturating_sub(13).max(20);
    if let Some(error) = &item.failure {
        let failure = ellipsize(&safe_failure(error), detail_width);
        println!("     {}  {}", paint("Error", "1;31", colors), failure);
    } else if let Some(output) = &item.output {
        let output = ellipsize_middle(&compact_path(output), detail_width);
        println!("     {}  {}", paint("Output", "1;32", colors), output);
    }
    println!();
}

pub(crate) fn compact_path(path: &std::path::Path) -> String {
    directories::UserDirs::new()
        .and_then(|directories| {
            path.strip_prefix(directories.home_dir())
                .ok()
                .map(|relative| {
                    if relative.as_os_str().is_empty() {
                        "~".to_string()
                    } else {
                        format!("~/{}", relative.display())
                    }
                })
        })
        .unwrap_or_else(|| path.display().to_string())
}

async fn expand_targets(
    client: &Crunchyroll,
    arguments: &DownloadArguments,
) -> Result<Vec<crunchydl::MediaTarget>> {
    let downloader = crunchydl::Downloader::builder(client.clone()).build();
    let collection = match arguments.kind {
        TargetKind::Episode => {
            return Ok(vec![crunchydl::MediaTarget::Episode(arguments.id.clone())]);
        }
        TargetKind::Movie => {
            return Ok(vec![crunchydl::MediaTarget::Movie(arguments.id.clone())]);
        }
        TargetKind::MusicVideo => {
            return Ok(vec![crunchydl::MediaTarget::MusicVideo(
                arguments.id.clone(),
            )]);
        }
        TargetKind::Season => crunchydl::CollectionTarget::Season(arguments.id.clone()),
        TargetKind::Series => crunchydl::CollectionTarget::Series(arguments.id.clone()),
        TargetKind::MovieListing => crunchydl::CollectionTarget::MovieListing(arguments.id.clone()),
    };
    let options = crunchydl::BatchOptions {
        include_specials: !arguments.exclude_specials,
        season_numbers: arguments.season_numbers.clone(),
    };
    let targets = downloader
        .expand_collection(&collection, &options)
        .await
        .map_err(Error::Download)?;
    if targets.is_empty() {
        return Err(Error::InvalidTarget(
            "the collection contains no items matching the filters".to_string(),
        ));
    }
    Ok(targets)
}
