//! Keyboard input handlers for each screen.

mod catalog;
mod global;
mod queue;

pub(crate) use catalog::{handle_browse_key, handle_search_key, handle_selection_key};
pub(crate) use global::handle_key;
pub(crate) use queue::{
    handle_account_key, handle_help_key, handle_queue_key, handle_settings_key, move_index,
};
