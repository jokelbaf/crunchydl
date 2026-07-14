//! CLI command surface: parsing at the boundary, dispatch in a separate module.

mod args;
mod dispatch;

pub(crate) use args::Arguments;
pub(crate) use dispatch::{run, run_queue_with_sink};
