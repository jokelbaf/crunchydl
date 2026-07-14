//! Transfer configuration, request descriptors, and engine state.

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::{Error, EventSink, TransferTrack};

/// Retry and concurrency limits for the transfer engine.
#[derive(Clone, Debug)]
pub struct TransferOptions {
    /// Maximum number of media segments in flight.
    pub concurrency: usize,
    /// Maximum attempts per URL, including the first request.
    pub max_attempts: u32,
    /// Initial exponential-backoff delay.
    pub base_retry_delay: Duration,
    /// Upper bound for one retry delay.
    pub max_retry_delay: Duration,
}

impl Default for TransferOptions {
    fn default() -> Self {
        Self {
            concurrency: 4,
            max_attempts: 4,
            base_retry_delay: Duration::from_millis(250),
            max_retry_delay: Duration::from_secs(8),
        }
    }
}

/// One exact HTTP resource in a representation.
#[derive(Clone)]
pub struct SegmentRequest {
    pub(crate) url: String,
    pub(crate) range: Option<(u64, u64)>,
    pub(crate) identity: String,
    pub(crate) expected_bytes: Option<u64>,
}

impl SegmentRequest {
    /// Construct a request. `identity` must be stable across signed-URL refreshes
    /// and must not contain credentials or query tokens.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        range: Option<(u64, u64)>,
        identity: impl Into<String>,
        expected_bytes: Option<u64>,
    ) -> Self {
        Self {
            url: url.into(),
            range,
            identity: identity.into(),
            expected_bytes,
        }
    }

    /// Stable, redacted identity used by resume validation.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }
}

impl fmt::Debug for SegmentRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SegmentRequest")
            .field("identity", &self.identity)
            .field("range", &self.range)
            .field("expected_bytes", &self.expected_bytes)
            .finish_non_exhaustive()
    }
}

/// Exact resources and stable descriptors for one selected representation.
#[derive(Clone, Debug)]
pub struct RepresentationTransferPlan {
    /// Media id used to isolate staging.
    pub media_id: String,
    /// Playback version id.
    pub version_id: String,
    /// Fingerprint of the complete redacted download plan.
    pub plan_fingerprint: String,
    /// Stable selected-representation fingerprint.
    pub representation_fingerprint: String,
    /// Optional safe track role supplied by the download orchestrator.
    pub track: Option<TransferTrack>,
    /// Initialization request.
    pub init: SegmentRequest,
    /// Media segment requests in presentation order.
    pub segments: Vec<SegmentRequest>,
}

/// A refresh hook used only after an authorization-style segment failure.
pub trait RepresentationRefresher: Send + Sync {
    /// Reopen playback and return the rematched representation.
    fn refresh<'a>(
        &'a self,
        expired: &'a RepresentationTransferPlan,
    ) -> Pin<Box<dyn Future<Output = Result<RepresentationTransferPlan, Error>> + Send + 'a>>;
}

/// A refresher that reports expiration as a terminal transfer error.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoRefresh;

impl RepresentationRefresher for NoRefresh {
    fn refresh<'a>(
        &'a self,
        _expired: &'a RepresentationTransferPlan,
    ) -> Pin<Box<dyn Future<Output = Result<RepresentationTransferPlan, Error>> + Send + 'a>> {
        Box::pin(async {
            Err(Error::Transfer(
                "signed segment URL expired and no refresher was configured".to_string(),
            ))
        })
    }
}

/// Paths of a fully downloaded staged representation.
#[derive(Clone, Debug)]
pub struct TransferResult {
    /// Initialization fragment.
    pub init: PathBuf,
    /// Media fragments in exact segment order.
    pub segments: Vec<PathBuf>,
    /// Durable resume journal.
    pub journal: PathBuf,
}

/// Reusable HTTP transfer engine.
#[derive(Clone)]
pub struct TransferEngine {
    pub(super) client: reqwest::Client,
    pub(super) options: TransferOptions,
    pub(super) events: Arc<dyn EventSink>,
}

impl fmt::Debug for TransferEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferEngine")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}
