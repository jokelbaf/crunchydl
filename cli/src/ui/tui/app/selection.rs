use crate::ui::tui::*;

impl App {
    pub(crate) fn configure_current(&mut self) {
        let Some(item) = self.current().cloned() else {
            return;
        };
        let return_screen = self.screen;
        if let Some(target) = item.target {
            let client = self.client.clone();
            let sender = self.sender.clone();
            self.selection = Selection {
                source: Some(SelectionSource::Media(target.clone())),
                title: item.title,
                loading: true,
                ..Selection::default()
            };
            self.previous_screen = return_screen;
            self.screen = Screen::Selection;
            self.set_notice(NoticeKind::Info, "Inspecting available tracks...");
            tokio::spawn(async move {
                let downloader = crunchydl::Downloader::builder(client).build();
                let result = async {
                    let media = downloader.resolve_target(&target).await?;
                    downloader
                        .inspect(&media, &crunchydl::CancellationToken::new())
                        .await
                }
                .await
                .map_err(Error::Download);
                let _ = sender.send(Message::Capabilities(result));
            });
            return;
        }
        let source = match item.kind {
            crunchydl::CatalogKind::Series => crunchydl::CollectionTarget::Series(item.id.clone()),
            crunchydl::CatalogKind::Season => crunchydl::CollectionTarget::Season(item.id.clone()),
            crunchydl::CatalogKind::MovieListing => {
                crunchydl::CollectionTarget::MovieListing(item.id.clone())
            }
            _ => return,
        };
        self.selection = Selection {
            source: Some(SelectionSource::Collection(source)),
            title: item.title,
            catalog_audio: item
                .audio_locales
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            catalog_subtitles: item
                .subtitle_locales
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            ..Selection::default()
        };
        self.previous_screen = return_screen;
        self.screen = Screen::Selection;
        self.set_notice(
            NoticeKind::Info,
            "Choose defaults that will be applied to every item in this batch",
        );
    }

    pub(crate) fn add_selection_to_queue(&mut self) -> Result<()> {
        if self.selection.loading {
            return Ok(());
        }
        let source = self
            .selection
            .source
            .clone()
            .ok_or_else(|| Error::InvalidTarget("no selected media".to_string()))?;
        let selection = self.selection.queue_selection();
        match source {
            SelectionSource::Media(target) => {
                Queue::load(&self.paths.queue)?
                    .add_named([(target, Some(self.selection.title.clone()))], selection)?;
                self.show(Screen::Queue);
                self.set_notice(NoticeKind::Success, "Added to the download queue");
            }
            SelectionSource::Collection(collection) => {
                let client = self.client.clone();
                let sender = self.sender.clone();
                let options = crunchydl::BatchOptions {
                    include_specials: self.selection.include_specials,
                    season_numbers: Vec::new(),
                };
                self.set_notice(NoticeKind::Info, "Expanding collection...");
                tokio::spawn(async move {
                    let downloader = crunchydl::Downloader::builder(client).build();
                    let result = downloader
                        .expand_collection(&collection, &options)
                        .await
                        .map_err(Error::Download);
                    let _ = sender.send(Message::Expanded { result, selection });
                });
            }
        }
        Ok(())
    }
}
