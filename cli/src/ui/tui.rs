//! Full-screen Ratatui frontend for discovery, selection, queue, and settings.

use std::collections::BTreeSet;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Gauge, List, ListItem, ListState, Padding, Paragraph, Wrap,
};
use ratatui_image::StatefulImage;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use tokio::sync::mpsc;

use crate::config::{Config, DrmBackend};
use crate::error::{Error, Result};
use crate::paths::AppPaths;
use crate::presentation::{
    AccountSummary, account_summary, ellipsize, human_bytes, kind_label, locale_label_from_code,
    locale_name, safe_failure, selection_label, yes_no,
};
use crate::queue::{Queue, QueueFormat, QueueItem, QueueSelection, QueueState};

mod app;
mod events;
mod input;
mod lifecycle;
mod model;
mod render;
mod terminal;

pub(crate) use app::App;
pub(crate) use events::handle_message;
pub(crate) use input::{
    handle_account_key, handle_browse_key, handle_help_key, handle_key, handle_queue_key,
    handle_search_key, handle_selection_key, handle_settings_key, move_index,
};
pub(crate) use lifecycle::run;
pub(crate) use model::*;
pub(crate) use render::*;
pub(crate) use terminal::*;
