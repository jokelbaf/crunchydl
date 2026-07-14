use crate::ui::tui::*;

impl App {
    pub(crate) fn set_notice(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Notice {
            kind,
            text: text.into(),
        };
    }

    pub(crate) fn current(&self) -> Option<&crunchydl::CatalogItem> {
        self.items.get(self.selected)
    }

    pub(crate) fn current_queue(&self) -> Option<&QueueItem> {
        self.queue_items.get(self.queue_selected)
    }

    pub(crate) fn reload_queue(&mut self) -> Result<()> {
        self.queue_items = Queue::load(&self.paths.queue)?.items().to_vec();
        self.queue_selected = self
            .queue_selected
            .min(self.queue_items.len().saturating_sub(1));
        Ok(())
    }

    pub(crate) fn show(&mut self, screen: Screen) {
        self.previous_screen = self.screen;
        self.screen = screen;
        if screen == Screen::Queue
            && let Err(error) = self.reload_queue()
        {
            self.set_notice(NoticeKind::Error, error.to_string());
        }
    }

    pub(crate) fn schedule_search(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.search_deadline = if self.query.trim().is_empty() {
            self.items.clear();
            self.selected = 0;
            self.set_notice(
                NoticeKind::Info,
                "Start typing to search the Crunchyroll catalog",
            );
            None
        } else {
            self.set_notice(NoticeKind::Info, "Waiting for input...");
            Some(Instant::now() + SEARCH_DEBOUNCE)
        };
    }

    pub(crate) fn start_search(&mut self) {
        let client = self.client.clone();
        let query = self.query.clone();
        let generation = self.generation;
        let sender = self.sender.clone();
        self.search_deadline = None;
        self.set_notice(NoticeKind::Info, "Searching...");
        tokio::spawn(async move {
            let result = crate::catalog::search(&client, &query, 40).await;
            let _ = sender.send(Message::Search { generation, result });
        });
    }

    pub(crate) fn request_thumbnail(&mut self) {
        let source = self
            .current()
            .and_then(crunchydl::CatalogItem::best_artwork)
            .map(|image| image.source.clone());
        let Some(source) = source else {
            self.thumbnail = None;
            self.thumbnail_loading = None;
            return;
        };
        if self
            .thumbnail
            .as_ref()
            .is_some_and(|(current, _)| current == &source)
            || self.thumbnail_loading.as_ref() == Some(&source)
        {
            return;
        }
        self.thumbnail = None;
        self.thumbnail_loading = Some(source.clone());
        let client = self.image_client.clone();
        let cache = self.paths.thumbnail_cache.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            match crate::thumbnail::load(&client, &cache, &source).await {
                Ok(image) => {
                    let _ = sender.send(Message::Thumbnail { source, image });
                }
                Err(()) => {
                    let _ = sender.send(Message::ThumbnailFailed(source));
                }
            }
        });
    }

    pub(crate) fn open_current(&mut self) {
        let Some(item) = self.current().cloned() else {
            return;
        };
        match item.kind {
            crunchydl::CatalogKind::Series
            | crunchydl::CatalogKind::Season
            | crunchydl::CatalogKind::MovieListing => {
                let client = self.client.clone();
                let sender = self.sender.clone();
                self.set_notice(NoticeKind::Info, format!("Loading {}...", item.title));
                tokio::spawn(async move {
                    let _ = sender.send(Message::Children(
                        crate::catalog::children(&client, &item).await,
                    ));
                });
            }
            _ => self.configure_current(),
        }
    }
}
