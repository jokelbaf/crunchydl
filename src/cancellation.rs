//! Cheap cooperative cancellation for long-running operations.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::Error;

/// A clonable handle used to cooperatively cancel planning and transfers.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a token in the active state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Calling this more than once is harmless.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
