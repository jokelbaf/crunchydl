//! Retry, resume-journal, and transfer-diagnostic helpers.

use std::time::Duration;

use crate::Error;
use crate::staging::{ResumeJournal, StagingLayout};

use super::RepresentationTransferPlan;

pub(super) fn request_error_kind(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_body() || error.is_decode() {
        "response body was interrupted"
    } else {
        "network request failed"
    }
}

pub(super) fn segment_label(index: Option<usize>) -> String {
    index.map_or_else(
        || "initialization segment".to_string(),
        |index| format!("media segment {}", index + 1),
    )
}

pub(super) fn representation_bytes(plan: &RepresentationTransferPlan) -> Option<u64> {
    std::iter::once(plan.init.expected_bytes)
        .chain(plan.segments.iter().map(|segment| segment.expected_bytes))
        .try_fold(0_u64, |total, bytes| {
            bytes.and_then(|bytes| total.checked_add(bytes))
        })
}

#[derive(Clone)]
pub(super) struct TransferEventContext {
    pub(super) media_id: String,
    pub(super) version_id: String,
    pub(super) representation_fingerprint: String,
    pub(super) track: Option<crate::TransferTrack>,
}

impl From<&RepresentationTransferPlan> for TransferEventContext {
    fn from(plan: &RepresentationTransferPlan) -> Self {
        Self {
            media_id: plan.media_id.clone(),
            version_id: plan.version_id.clone(),
            representation_fingerprint: plan.representation_fingerprint.clone(),
            track: plan.track.clone(),
        }
    }
}

pub(super) enum FetchError {
    Expired,
    Fatal(Error),
}

impl FetchError {
    pub(super) fn into_attempt_error(self) -> AttemptError {
        match self {
            Self::Expired => AttemptError::Expired,
            Self::Fatal(error) => AttemptError::Fatal(error),
        }
    }
}

pub(super) enum AttemptError {
    Expired,
    Fatal(Error),
}

pub(super) fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

pub(super) async fn load_or_create_journal(
    layout: &StagingLayout,
    plan: &RepresentationTransferPlan,
) -> Result<ResumeJournal, Error> {
    if let Some(journal) = ResumeJournal::load(&layout.journal).await? {
        validate_journal(&journal, plan)?;
        return Ok(journal);
    }
    let mut journal = ResumeJournal::new(
        plan.media_id.clone(),
        plan.version_id.clone(),
        plan.plan_fingerprint.clone(),
        plan.representation_fingerprint.clone(),
        plan.init.identity.clone(),
        plan.segments
            .iter()
            .map(|segment| segment.identity.clone())
            .collect(),
    );
    journal.save(&layout.journal).await?;
    Ok(journal)
}

fn validate_journal(
    journal: &ResumeJournal,
    plan: &RepresentationTransferPlan,
) -> Result<(), Error> {
    let identities = plan
        .segments
        .iter()
        .map(|segment| segment.identity.as_str())
        .collect::<Vec<_>>();
    if journal.media_id != plan.media_id
        || journal.version_id != plan.version_id
        || journal.plan_fingerprint != plan.plan_fingerprint
        || journal.representation_fingerprint != plan.representation_fingerprint
        || journal.init_identity != plan.init.identity
        || journal.segment_identities.len() != identities.len()
        || journal
            .segment_identities
            .iter()
            .map(String::as_str)
            .ne(identities)
    {
        return Err(Error::ResumeMismatch(
            "staging descriptors do not match the current immutable plan".to_string(),
        ));
    }
    Ok(())
}

pub(super) async fn validate_completed_files(
    layout: &StagingLayout,
    journal: &mut ResumeJournal,
) -> Result<(), Error> {
    if let Some(expected) = journal.init_bytes {
        let observed = tokio::fs::metadata(&layout.init)
            .await
            .map_err(|_| Error::ResumeMismatch("completed init file is missing".to_string()))?
            .len();
        if observed != expected {
            return Err(Error::ResumeMismatch(format!(
                "completed init length changed: expected {expected}, got {observed}"
            )));
        }
    }
    for completed in &journal.completed {
        if completed.index >= journal.segment_identities.len() {
            return Err(Error::ResumeMismatch(
                "journal contains an out-of-range segment index".to_string(),
            ));
        }
        let observed = tokio::fs::metadata(layout.segment(completed.index))
            .await
            .map_err(|_| {
                Error::ResumeMismatch(format!("completed segment {} is missing", completed.index))
            })?
            .len();
        if observed != completed.bytes {
            return Err(Error::ResumeMismatch(format!(
                "completed segment {} length changed: expected {}, got {observed}",
                completed.index, completed.bytes
            )));
        }
    }
    Ok(())
}

pub(super) fn validate_refresh(
    expired: &RepresentationTransferPlan,
    refreshed: &RepresentationTransferPlan,
) -> Result<(), Error> {
    if expired.media_id != refreshed.media_id
        || expired.version_id != refreshed.version_id
        || expired.plan_fingerprint != refreshed.plan_fingerprint
        || expired.representation_fingerprint != refreshed.representation_fingerprint
        || expired.init.identity != refreshed.init.identity
        || expired.segments.len() != refreshed.segments.len()
        || expired
            .segments
            .iter()
            .zip(&refreshed.segments)
            .any(|(left, right)| left.identity != right.identity || left.range != right.range)
    {
        return Err(Error::ResumeMismatch(
            "refreshed playback did not contain the exact stable representation".to_string(),
        ));
    }
    Ok(())
}
