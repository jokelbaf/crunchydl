use crate::ui::tui::*;

impl App {
    pub(crate) fn start_queue(&mut self) {
        if self.queue_running {
            return;
        }
        let paths = self.paths.clone();
        let sender = self.sender.clone();
        let event_sender = sender.clone();
        let cancellation = crunchydl::CancellationToken::new();
        let runner_cancellation = cancellation.clone();
        let sink: std::sync::Arc<dyn crunchydl::EventSink> = std::sync::Arc::new(move |event| {
            let _ = event_sender.send(Message::DownloadEvent(event));
        });
        self.queue_running = true;
        self.queue_cancellation = Some(cancellation);
        self.progress = DownloadProgress::default();
        self.set_notice(NoticeKind::Info, "Starting pending downloads...");
        tokio::spawn(async move {
            let result = crate::command::run_queue_with_sink(&paths, sink, runner_cancellation)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(Message::QueueFinished(result));
        });
    }

    pub(crate) fn retry_selected(&mut self) -> Result<()> {
        let Some(item) = self.current_queue() else {
            return Ok(());
        };
        let id = item.id;
        if Queue::load(&self.paths.queue)?.retry(id)? {
            self.reload_queue()?;
            self.set_notice(NoticeKind::Success, "Moved item back to pending");
        } else {
            self.set_notice(NoticeKind::Warning, "Only failed items can be retried");
        }
        Ok(())
    }

    pub(crate) fn retry_all(&mut self) -> Result<()> {
        let count = Queue::load(&self.paths.queue)?.retry_failed()?;
        self.reload_queue()?;
        self.set_notice(
            NoticeKind::Success,
            format!("Moved {count} failed item(s) back to pending"),
        );
        Ok(())
    }

    pub(crate) fn remove_selected(&mut self) {
        if let Some(item) = self.current_queue() {
            if item.state == QueueState::Running {
                self.set_notice(
                    NoticeKind::Warning,
                    "Cancel the active download before removing it",
                );
            } else {
                self.confirmation = Some(Confirmation::Remove(item.id));
            }
        }
    }

    pub(crate) fn confirm(&mut self) -> Result<()> {
        let Some(action) = self.confirmation.take() else {
            return Ok(());
        };
        match action {
            Confirmation::Remove(id) => {
                Queue::load(&self.paths.queue)?.remove(id)?;
                self.reload_queue()?;
                self.set_notice(NoticeKind::Success, "Removed queue item");
            }
            Confirmation::ClearCompleted => {
                let count = Queue::load(&self.paths.queue)?.clear_completed()?;
                self.reload_queue()?;
                self.set_notice(
                    NoticeKind::Success,
                    format!("Removed {count} completed item(s)"),
                );
            }
            Confirmation::Logout => {
                let paths = self.paths.clone();
                let sender = self.sender.clone();
                self.set_notice(NoticeKind::Info, "Signing out...");
                tokio::spawn(async move {
                    let _ = sender.send(Message::LoggedOut(crate::auth::logout(&paths).await));
                });
            }
        }
        Ok(())
    }
}
