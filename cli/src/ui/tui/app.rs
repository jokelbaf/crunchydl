use super::*;

mod navigation;
mod queue;
mod selection;
mod settings;

pub(crate) struct App {
    pub(crate) client: crunchyroll_rs::Crunchyroll,
    pub(crate) account: AccountSummary,
    pub(crate) config: Config,
    pub(crate) paths: AppPaths,
    pub(crate) screen: Screen,
    pub(crate) previous_screen: Screen,
    pub(crate) query: String,
    pub(crate) items: Vec<crunchydl::CatalogItem>,
    pub(crate) selected: usize,
    pub(crate) queue_items: Vec<QueueItem>,
    pub(crate) queue_selected: usize,
    pub(crate) settings_selected: usize,
    pub(crate) settings_editing: Option<SettingsField>,
    pub(crate) edit_buffer: String,
    pub(crate) confirmation: Option<Confirmation>,
    pub(crate) notice: Notice,
    pub(crate) search_deadline: Option<Instant>,
    pub(crate) generation: u64,
    pub(crate) sender: mpsc::UnboundedSender<Message>,
    pub(crate) selection: Selection,
    pub(crate) browse_parents: Vec<(Vec<crunchydl::CatalogItem>, usize)>,
    pub(crate) queue_running: bool,
    pub(crate) queue_cancellation: Option<crunchydl::CancellationToken>,
    pub(crate) progress: DownloadProgress,
    pub(crate) picker: Picker,
    pub(crate) thumbnail: Option<(String, StatefulProtocol)>,
    pub(crate) thumbnail_loading: Option<String>,
    pub(crate) image_client: reqwest::Client,
    pub(crate) should_quit: bool,
}
