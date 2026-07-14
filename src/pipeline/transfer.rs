//! Bounded, ordered, resumable HTTP segment transfer.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::task::JoinSet;

use crate::staging::{CompletedSegment, ResumeJournal, StagingLayout, atomic_write};
use crate::{CancellationToken, DownloadEvent, Error, EventSink, NoopSink};

mod support;
use support::*;

mod io;
mod types;

pub use types::{
    NoRefresh, RepresentationRefresher, RepresentationTransferPlan, SegmentRequest, TransferEngine,
    TransferOptions, TransferResult,
};
impl TransferEngine {
    /// Construct an engine with a dedicated rustls HTTP client and no events.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transfer`] if the HTTP client cannot be constructed or
    /// if an option invariant is invalid.
    pub fn new(options: TransferOptions) -> Result<Self, Error> {
        Self::with_client(reqwest::Client::new(), options)
    }

    /// Construct an engine around a caller-configured HTTP client.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transfer`] if concurrency or attempts is zero.
    pub fn with_client(client: reqwest::Client, options: TransferOptions) -> Result<Self, Error> {
        if options.concurrency == 0 || options.max_attempts == 0 {
            return Err(Error::Transfer(
                "transfer concurrency and max attempts must be nonzero".to_string(),
            ));
        }
        Ok(Self {
            client,
            options,
            events: Arc::new(NoopSink),
        })
    }

    /// Deliver structured progress and retry events to `sink`.
    #[must_use]
    pub fn event_sink(mut self, sink: impl EventSink + 'static) -> Self {
        self.events = Arc::new(sink);
        self
    }

    /// Download or resume a representation beneath `staging_root`.
    ///
    /// # Errors
    ///
    /// Returns typed cancellation, transfer, filesystem, or resume mismatch
    /// errors. Existing staging is reused only after complete validation.
    pub async fn transfer(
        &self,
        plan: RepresentationTransferPlan,
        staging_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<TransferResult, Error> {
        self.transfer_with_refresh(plan, staging_root, cancellation, &NoRefresh)
            .await
    }

    pub(crate) async fn transfer_init(
        &self,
        plan: &RepresentationTransferPlan,
        staging_root: &Path,
        cancellation: &CancellationToken,
    ) -> Result<PathBuf, Error> {
        let layout = StagingLayout::new(
            staging_root,
            &plan.media_id,
            &plan.representation_fingerprint,
        );
        layout.create().await?;
        let mut journal = load_or_create_journal(&layout, plan).await?;
        validate_completed_files(&layout, &mut journal).await?;
        self.transfer_initialization(plan, &layout, &mut journal, cancellation)
            .await
            .map_err(|error| match error {
                AttemptError::Expired => {
                    Error::Transfer("initialization URL expired before licensing".to_string())
                }
                AttemptError::Fatal(error) => error,
            })?;
        Ok(layout.init)
    }

    /// Download or resume with stable representation refresh after URL expiry.
    ///
    /// # Errors
    ///
    /// In addition to [`TransferEngine::transfer`] errors, returns
    /// [`Error::ResumeMismatch`] when refreshed playback no longer contains the
    /// exact stable representation.
    pub async fn transfer_with_refresh<R: RepresentationRefresher>(
        &self,
        mut plan: RepresentationTransferPlan,
        staging_root: &Path,
        cancellation: &CancellationToken,
        refresher: &R,
    ) -> Result<TransferResult, Error> {
        let layout = StagingLayout::new(
            staging_root,
            &plan.media_id,
            &plan.representation_fingerprint,
        );
        layout.create().await?;
        let mut journal = load_or_create_journal(&layout, &plan).await?;
        validate_completed_files(&layout, &mut journal).await?;

        loop {
            cancellation.check()?;
            match self
                .transfer_current(&plan, &layout, &mut journal, cancellation)
                .await
            {
                Ok(()) => break,
                Err(AttemptError::Expired) => {
                    let refreshed = refresher.refresh(&plan).await?;
                    validate_refresh(&plan, &refreshed)?;
                    plan = refreshed;
                }
                Err(AttemptError::Fatal(error)) => return Err(error),
            }
        }

        Ok(TransferResult {
            init: layout.init.clone(),
            segments: (0..plan.segments.len())
                .map(|index| layout.segment(index))
                .collect(),
            journal: layout.journal.clone(),
        })
    }

    async fn transfer_current(
        &self,
        plan: &RepresentationTransferPlan,
        layout: &StagingLayout,
        journal: &mut ResumeJournal,
        cancellation: &CancellationToken,
    ) -> Result<(), AttemptError> {
        self.transfer_initialization(plan, layout, journal, cancellation)
            .await?;

        let completed = journal
            .completed
            .iter()
            .map(|segment| segment.index)
            .collect::<BTreeSet<_>>();
        let pending = (0..plan.segments.len())
            .filter(|index| !completed.contains(index))
            .collect::<Vec<_>>();
        for batch in pending.chunks(self.options.concurrency) {
            cancellation.check().map_err(AttemptError::Fatal)?;
            let mut tasks = JoinSet::new();
            for &index in batch {
                let engine = self.clone();
                let event_context = TransferEventContext::from(plan);
                let request = plan.segments[index].clone();
                let path = layout.segment(index);
                let cancellation = cancellation.clone();
                tasks.spawn(async move {
                    let bytes = engine
                        .fetch(&event_context, &request, Some(index), &cancellation)
                        .await?;
                    cancellation.check().map_err(FetchError::Fatal)?;
                    atomic_write(&path, &bytes)
                        .await
                        .map_err(FetchError::Fatal)?;
                    Ok::<_, FetchError>((index, bytes.len() as u64))
                });
            }

            let mut successes = BTreeMap::new();
            let mut first_error = None;
            while let Some(result) = tasks.join_next().await {
                match result {
                    Ok(Ok((index, bytes))) => {
                        successes.insert(index, bytes);
                    }
                    Ok(Err(error)) if first_error.is_none() => first_error = Some(error),
                    Err(error) if first_error.is_none() => {
                        first_error = Some(FetchError::Fatal(Error::Transfer(format!(
                            "segment task failed: {error}"
                        ))));
                    }
                    _ => {}
                }
            }
            for (index, bytes) in successes {
                journal.completed.push(CompletedSegment { index, bytes });
                journal.completed.sort_by_key(|segment| segment.index);
                journal
                    .save(&layout.journal)
                    .await
                    .map_err(AttemptError::Fatal)?;
                self.events.emit(DownloadEvent::SegmentCompleted {
                    media_id: plan.media_id.clone(),
                    version_id: plan.version_id.clone(),
                    representation_fingerprint: plan.representation_fingerprint.clone(),
                    track: plan.track.clone(),
                    index,
                    completed: journal.completed.len(),
                    total: plan.segments.len(),
                    bytes,
                    completed_bytes: journal.init_bytes.unwrap_or_default().saturating_add(
                        journal.completed.iter().map(|segment| segment.bytes).sum(),
                    ),
                    total_bytes: representation_bytes(plan),
                });
            }
            if let Some(error) = first_error {
                return Err(error.into_attempt_error());
            }
        }
        Ok(())
    }

    async fn transfer_initialization(
        &self,
        plan: &RepresentationTransferPlan,
        layout: &StagingLayout,
        journal: &mut ResumeJournal,
        cancellation: &CancellationToken,
    ) -> Result<(), AttemptError> {
        if journal.init_bytes.is_none() {
            let event_context = TransferEventContext::from(plan);
            let bytes = self
                .fetch(&event_context, &plan.init, None, cancellation)
                .await
                .map_err(|error| error.into_attempt_error())?;
            cancellation.check().map_err(AttemptError::Fatal)?;
            atomic_write(&layout.init, &bytes)
                .await
                .map_err(AttemptError::Fatal)?;
            journal.init_bytes = Some(bytes.len() as u64);
            journal
                .save(&layout.journal)
                .await
                .map_err(AttemptError::Fatal)?;
        }
        Ok(())
    }
}
