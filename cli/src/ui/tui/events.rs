use super::*;

pub(crate) fn handle_message(app: &mut App, message: Message) {
    match message {
        Message::Search { generation, result } if generation == app.generation => match result {
            Ok(items) => {
                app.set_notice(
                    NoticeKind::Success,
                    format!("Found {} result(s)", items.len()),
                );
                app.items = items;
                app.selected = 0;
                app.request_thumbnail();
            }
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
        Message::Search { .. } => {}
        Message::Children(result) => match result {
            Ok(items) if items.is_empty() => {
                app.set_notice(NoticeKind::Warning, "This collection is empty");
            }
            Ok(items) => {
                app.browse_parents.push((app.items.clone(), app.selected));
                app.items = items;
                app.selected = 0;
                app.screen = Screen::Browse;
                app.set_notice(NoticeKind::Success, format!("{} item(s)", app.items.len()));
                app.request_thumbnail();
            }
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
        Message::Capabilities(result) => match result {
            Ok(capabilities) => {
                app.selection.loading = false;
                app.selection.capabilities = Some(capabilities);
                app.set_notice(NoticeKind::Info, "Choose tracks and press Enter to queue");
            }
            Err(error) => {
                app.selection.loading = false;
                app.set_notice(NoticeKind::Error, error.to_string());
            }
        },
        Message::Expanded { result, selection } => match result {
            Ok(targets) if targets.is_empty() => {
                app.set_notice(NoticeKind::Warning, "No playable items matched this batch");
            }
            Ok(targets) => match Queue::load(&app.paths.queue)
                .and_then(|mut queue| queue.add(targets, selection))
            {
                Ok(ids) => {
                    app.show(Screen::Queue);
                    app.set_notice(
                        NoticeKind::Success,
                        format!(
                            "Added {} batch item(s) with your selected tracks",
                            ids.len()
                        ),
                    );
                }
                Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
            },
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
        Message::DownloadEvent(event) => handle_download_event(app, event),
        Message::QueueFinished(result) => {
            app.queue_running = false;
            app.queue_cancellation = None;
            let _ = app.reload_queue();
            match result {
                Ok(()) => app.set_notice(NoticeKind::Success, "Queue finished successfully"),
                Err(error) => app.set_notice(NoticeKind::Error, error),
            }
        }
        Message::Thumbnail { source, image } if app.thumbnail_loading.as_ref() == Some(&source) => {
            app.thumbnail = Some((source, app.picker.new_resize_protocol(image)));
            app.thumbnail_loading = None;
        }
        Message::Thumbnail { .. } => {}
        Message::ThumbnailFailed(source) if app.thumbnail_loading.as_ref() == Some(&source) => {
            app.thumbnail_loading = None;
        }
        Message::ThumbnailFailed(_) => {}
        Message::LoggedOut(result) => match result {
            Ok(_) => {
                app.set_notice(NoticeKind::Success, "Signed out");
                app.should_quit = true;
            }
            Err(error) => app.set_notice(NoticeKind::Error, error.to_string()),
        },
    }
}

pub(crate) fn handle_download_event(app: &mut App, event: crunchydl::DownloadEvent) {
    match event {
        crunchydl::DownloadEvent::StateChanged(state) => {
            app.progress.label = job_state_label(state).to_string();
            app.progress.detail.clear();
            let _ = app.reload_queue();
        }
        crunchydl::DownloadEvent::SegmentCompleted {
            completed,
            total,
            completed_bytes,
            total_bytes,
            track,
            ..
        } => {
            app.progress.label = track.map_or_else(
                || "Downloading media".to_string(),
                |track| {
                    format!(
                        "Downloading {:?} • {}",
                        track.kind,
                        locale_name(&track.locale)
                    )
                },
            );
            if let Some(total_bytes) = total_bytes {
                app.progress.completed = completed_bytes;
                app.progress.total = total_bytes;
                app.progress.detail = format!(
                    "{} / {} • {completed}/{total} segments",
                    human_bytes(completed_bytes),
                    human_bytes(total_bytes)
                );
            } else {
                app.progress.completed = completed as u64;
                app.progress.total = total as u64;
                app.progress.detail = format!("{completed}/{total} segments");
            }
        }
        crunchydl::DownloadEvent::StageProgress {
            state,
            completed,
            total,
        } => {
            app.progress.label = job_state_label(state).to_string();
            app.progress.completed = completed as u64;
            app.progress.total = total as u64;
            app.progress.detail = format!("{completed}/{total}");
        }
        crunchydl::DownloadEvent::TransferRetry { attempt, delay, .. } => app.set_notice(
            NoticeKind::Warning,
            format!(
                "Network interrupted - retry {attempt} in {:.1}s",
                delay.as_secs_f64()
            ),
        ),
        crunchydl::DownloadEvent::Warning(warning) => {
            app.set_notice(NoticeKind::Warning, warning.to_string());
        }
        crunchydl::DownloadEvent::OutputCommitted { output } => {
            app.set_notice(NoticeKind::Success, format!("Saved {}", output.display()));
            let _ = app.reload_queue();
        }
        _ => {}
    }
}
