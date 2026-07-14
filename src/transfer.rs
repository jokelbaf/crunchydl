//! Bounded, ordered, resumable HTTP segment transfer.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::StatusCode;
use tokio::task::JoinSet;

use crate::staging::{CompletedSegment, ResumeJournal, StagingLayout, atomic_write};
use crate::{CancellationToken, DownloadEvent, Error, EventSink, NoopSink, TransferTrack};

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
    url: String,
    range: Option<(u64, u64)>,
    identity: String,
    expected_bytes: Option<u64>,
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
    client: reqwest::Client,
    options: TransferOptions,
    events: Arc<dyn EventSink>,
}

impl fmt::Debug for TransferEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransferEngine")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

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

    async fn fetch(
        &self,
        event_context: &TransferEventContext,
        request: &SegmentRequest,
        index: Option<usize>,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u8>, FetchError> {
        for attempt in 1..=self.options.max_attempts {
            cancellation.check().map_err(FetchError::Fatal)?;
            let mut builder = self.client.get(&request.url);
            if let Some((start, end)) = request.range {
                builder = builder.header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
            }
            let response = match builder.send().await {
                Ok(response) => response,
                Err(error) => {
                    if attempt == self.options.max_attempts {
                        return Err(FetchError::Fatal(Error::Transfer(format!(
                            "{} transport failed after {attempt} attempts: {}",
                            segment_label(index),
                            request_error_kind(&error)
                        ))));
                    }
                    self.retry_delay(event_context, index, attempt + 1, None, cancellation)
                        .await?;
                    continue;
                }
            };
            cancellation.check().map_err(FetchError::Fatal)?;
            let status = response.status();
            if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
                return Err(FetchError::Expired);
            }
            if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                if attempt == self.options.max_attempts {
                    return Err(FetchError::Fatal(Error::Transfer(format!(
                        "{} returned HTTP {status} after {attempt} attempts",
                        segment_label(index)
                    ))));
                }
                let retry_after = parse_retry_after(response.headers());
                self.retry_delay(event_context, index, attempt + 1, retry_after, cancellation)
                    .await?;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Fatal(Error::Transfer(format!(
                    "{} returned non-retryable HTTP {status}",
                    segment_label(index)
                ))));
            }
            let bytes = match response.bytes().await {
                Ok(bytes) => bytes,
                Err(error) => {
                    if attempt == self.options.max_attempts {
                        return Err(FetchError::Fatal(Error::Transfer(format!(
                            "{} response body failed after {attempt} attempts: {}",
                            segment_label(index),
                            request_error_kind(&error)
                        ))));
                    }
                    self.retry_delay(event_context, index, attempt + 1, None, cancellation)
                        .await?;
                    continue;
                }
            };
            cancellation.check().map_err(FetchError::Fatal)?;
            if let Some(expected) = request.expected_bytes
                && bytes.len() as u64 != expected
            {
                return Err(FetchError::Fatal(Error::Transfer(format!(
                    "{} length mismatch: expected {expected}, got {}",
                    segment_label(index),
                    bytes.len()
                ))));
            }
            return Ok(bytes.to_vec());
        }
        unreachable!("max_attempts is validated as nonzero")
    }

    async fn retry_delay(
        &self,
        context: &TransferEventContext,
        index: Option<usize>,
        next_attempt: u32,
        retry_after: Option<Duration>,
        cancellation: &CancellationToken,
    ) -> Result<(), FetchError> {
        let exponent = next_attempt.saturating_sub(2).min(31);
        let factor = 1_u32 << exponent;
        let base = self
            .options
            .base_retry_delay
            .saturating_mul(factor)
            .min(self.options.max_retry_delay);
        let jitter = Duration::from_millis(
            ((index.unwrap_or(0) as u64 * 31 + u64::from(next_attempt) * 17) % 101)
                .min(self.options.max_retry_delay.as_millis() as u64),
        );
        let delay = retry_after.unwrap_or_else(|| {
            base.saturating_add(jitter)
                .min(self.options.max_retry_delay)
        });
        self.events.emit(DownloadEvent::TransferRetry {
            media_id: context.media_id.clone(),
            version_id: context.version_id.clone(),
            representation_fingerprint: context.representation_fingerprint.clone(),
            track: context.track.clone(),
            index,
            attempt: next_attempt,
            delay,
        });
        let started = tokio::time::Instant::now();
        while started.elapsed() < delay {
            cancellation.check().map_err(FetchError::Fatal)?;
            tokio::time::sleep(
                Duration::from_millis(10).min(delay.saturating_sub(started.elapsed())),
            )
            .await;
        }
        Ok(())
    }
}

fn request_error_kind(error: &reqwest::Error) -> &'static str {
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

fn segment_label(index: Option<usize>) -> String {
    index.map_or_else(
        || "initialization segment".to_string(),
        |index| format!("media segment {}", index + 1),
    )
}

fn representation_bytes(plan: &RepresentationTransferPlan) -> Option<u64> {
    std::iter::once(plan.init.expected_bytes)
        .chain(plan.segments.iter().map(|segment| segment.expected_bytes))
        .try_fold(0_u64, |total, bytes| {
            bytes.and_then(|bytes| total.checked_add(bytes))
        })
}

#[derive(Clone)]
struct TransferEventContext {
    media_id: String,
    version_id: String,
    representation_fingerprint: String,
    track: Option<TransferTrack>,
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

enum FetchError {
    Expired,
    Fatal(Error),
}

impl FetchError {
    fn into_attempt_error(self) -> AttemptError {
        match self {
            Self::Expired => AttemptError::Expired,
            Self::Fatal(error) => AttemptError::Fatal(error),
        }
    }
}

enum AttemptError {
    Expired,
    Fatal(Error),
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

async fn load_or_create_journal(
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

async fn validate_completed_files(
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

fn validate_refresh(
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::thread;

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct FixtureServer {
        base: String,
        requests: Arc<Mutex<HashMap<String, usize>>>,
        stop: Arc<AtomicBool>,
        thread: Option<thread::JoinHandle<()>>,
    }

    impl FixtureServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("fixture server binds");
            listener
                .set_nonblocking(true)
                .expect("listener is nonblocking");
            let address = listener.local_addr().expect("listener address");
            let requests = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let thread_requests = requests.clone();
            let thread_stop = stop.clone();
            let thread = thread::spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(connection) => connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(_) => break,
                    };
                    let mut request = [0_u8; 2048];
                    let read = stream.read(&mut request).unwrap_or(0);
                    let line = String::from_utf8_lossy(&request[..read]);
                    let path = line
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .split('?')
                        .next()
                        .unwrap_or("/")
                        .to_string();
                    let count = {
                        let mut requests = thread_requests.lock().expect("request lock");
                        let count = requests.entry(path.clone()).or_default();
                        *count += 1;
                        *count
                    };
                    let (status, headers, body): (&str, &str, &[u8]) = match (path.as_str(), count)
                    {
                        ("/rate", 1) => ("429 Too Many Requests", "Retry-After: 0\r\n", b""),
                        ("/unstable", 1) | ("/interrupt", 1) => {
                            ("500 Internal Server Error", "", b"")
                        }
                        ("/expired", _) => ("403 Forbidden", "", b""),
                        ("/init", _) => ("200 OK", "", b"init"),
                        ("/rate", _) => ("200 OK", "", b"rate"),
                        ("/unstable", _) => ("200 OK", "", b"unstable"),
                        ("/interrupt", _) => ("200 OK", "", b"interrupt"),
                        ("/fresh", _) => ("200 OK", "", b"fresh"),
                        _ => ("404 Not Found", "", b""),
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(body);
                }
            });
            Self {
                base: format!("http://{address}"),
                requests,
                stop,
                thread: Some(thread),
            }
        }

        fn count(&self, path: &str) -> usize {
            *self
                .requests
                .lock()
                .expect("request lock")
                .get(path)
                .unwrap_or(&0)
        }
    }

    impl Drop for FixtureServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                thread.join().expect("fixture server joins");
            }
        }
    }

    fn staging_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "crunchydl-transfer-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn request(id: &str) -> SegmentRequest {
        SegmentRequest::new(
            format!("https://example.invalid/{id}?secret"),
            None,
            id,
            None,
        )
    }

    fn plan() -> RepresentationTransferPlan {
        RepresentationTransferPlan {
            media_id: "EP1".to_string(),
            version_id: "V1".to_string(),
            plan_fingerprint: "plan".to_string(),
            representation_fingerprint: "representation".to_string(),
            track: None,
            init: request("init"),
            segments: vec![request("0"), request("1")],
        }
    }

    fn server_plan(server: &FixtureServer, paths: &[&str]) -> RepresentationTransferPlan {
        RepresentationTransferPlan {
            media_id: "EP1".to_string(),
            version_id: "V1".to_string(),
            plan_fingerprint: "plan".to_string(),
            representation_fingerprint: "0123456789abcdef".to_string(),
            track: None,
            init: SegmentRequest::new(
                format!("{}/init?token=one", server.base),
                None,
                "init",
                Some(4),
            ),
            segments: paths
                .iter()
                .map(|path| {
                    SegmentRequest::new(
                        format!("{}{path}?token=one", server.base),
                        None,
                        path.trim_start_matches('/'),
                        None,
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn debug_never_contains_signed_url() {
        let debug = format!("{:?}", request("segment"));
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("example.invalid"));
    }

    #[test]
    fn refresh_requires_stable_identity() {
        let original = plan();
        let mut changed = original.clone();
        changed.segments[0].identity = "different".to_string();
        assert!(matches!(
            validate_refresh(&original, &changed),
            Err(Error::ResumeMismatch(_))
        ));
    }

    #[tokio::test]
    async fn retries_rate_limit_and_server_error_then_preserves_order() {
        let server = FixtureServer::start();
        let staging = staging_directory();
        let engine = TransferEngine::new(TransferOptions {
            base_retry_delay: Duration::from_millis(1),
            max_retry_delay: Duration::from_millis(5),
            ..TransferOptions::default()
        })
        .expect("engine");
        let result = engine
            .transfer(
                server_plan(&server, &["/rate", "/unstable"]),
                &staging,
                &CancellationToken::new(),
            )
            .await
            .expect("transfer succeeds");
        assert_eq!(std::fs::read(&result.init).expect("init"), b"init");
        assert_eq!(std::fs::read(&result.segments[0]).expect("rate"), b"rate");
        assert_eq!(
            std::fs::read(&result.segments[1]).expect("unstable"),
            b"unstable"
        );
        assert_eq!(server.count("/rate"), 2);
        assert_eq!(server.count("/unstable"), 2);
        std::fs::remove_dir_all(staging).expect("cleanup");
    }

    #[tokio::test]
    async fn initialization_can_precede_full_transfer() {
        let server = FixtureServer::start();
        let staging = staging_directory();
        let engine = TransferEngine::new(TransferOptions::default()).expect("engine");
        let plan = server_plan(&server, &["/fresh"]);
        let init = engine
            .transfer_init(&plan, &staging, &CancellationToken::new())
            .await
            .expect("initialization succeeds");
        assert_eq!(std::fs::read(init).expect("init"), b"init");
        assert_eq!(server.count("/init"), 1);
        assert_eq!(server.count("/fresh"), 0);

        let result = engine
            .transfer(plan, &staging, &CancellationToken::new())
            .await
            .expect("full transfer resumes from initialization");
        assert_eq!(server.count("/init"), 1);
        assert_eq!(server.count("/fresh"), 1);
        assert_eq!(std::fs::read(&result.segments[0]).expect("fresh"), b"fresh");
        std::fs::remove_dir_all(staging).expect("cleanup");
    }

    #[tokio::test]
    async fn interrupted_transfer_resumes_without_redownloading_completed_segments() {
        let server = FixtureServer::start();
        let staging = staging_directory();
        let first = TransferEngine::new(TransferOptions {
            concurrency: 2,
            max_attempts: 1,
            ..TransferOptions::default()
        })
        .expect("engine");
        let plan = server_plan(&server, &["/fresh", "/interrupt"]);
        assert!(
            first
                .transfer(plan.clone(), &staging, &CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(server.count("/fresh"), 1);

        let second = TransferEngine::new(TransferOptions {
            base_retry_delay: Duration::from_millis(1),
            max_retry_delay: Duration::from_millis(5),
            ..TransferOptions::default()
        })
        .expect("engine");
        let result = second
            .transfer(plan, &staging, &CancellationToken::new())
            .await
            .expect("resume succeeds");
        assert_eq!(server.count("/fresh"), 1);
        assert_eq!(server.count("/interrupt"), 2);
        assert_eq!(result.segments.len(), 2);
        std::fs::remove_dir_all(staging).expect("cleanup");
    }

    struct RefreshToFresh {
        fresh: RepresentationTransferPlan,
    }

    impl RepresentationRefresher for RefreshToFresh {
        fn refresh<'a>(
            &'a self,
            _expired: &'a RepresentationTransferPlan,
        ) -> Pin<Box<dyn Future<Output = Result<RepresentationTransferPlan, Error>> + Send + 'a>>
        {
            Box::pin(async { Ok(self.fresh.clone()) })
        }
    }

    #[tokio::test]
    async fn expired_url_refreshes_only_an_exact_representation_match() {
        let server = FixtureServer::start();
        let staging = staging_directory();
        let expired = server_plan(&server, &["/expired"]);
        let mut fresh = expired.clone();
        fresh.segments[0].url = format!("{}/fresh?token=two", server.base);
        let engine = TransferEngine::new(TransferOptions::default()).expect("engine");
        let result = engine
            .transfer_with_refresh(
                expired,
                &staging,
                &CancellationToken::new(),
                &RefreshToFresh { fresh },
            )
            .await
            .expect("refresh succeeds");
        assert_eq!(std::fs::read(&result.segments[0]).expect("fresh"), b"fresh");
        assert_eq!(server.count("/expired"), 1);
        assert_eq!(server.count("/fresh"), 1);
        std::fs::remove_dir_all(staging).expect("cleanup");
    }

    #[tokio::test]
    async fn mismatched_staging_is_rejected_without_network_reuse() {
        let server = FixtureServer::start();
        let staging = staging_directory();
        let engine = TransferEngine::new(TransferOptions::default()).expect("engine");
        let original = server_plan(&server, &["/rate"]);
        engine
            .transfer(original.clone(), &staging, &CancellationToken::new())
            .await
            .expect("initial transfer");
        let before = server.count("/rate");
        let mut changed = original;
        changed.plan_fingerprint = "different-plan".to_string();
        assert!(matches!(
            engine
                .transfer(changed, &staging, &CancellationToken::new())
                .await,
            Err(Error::ResumeMismatch(_))
        ));
        assert_eq!(server.count("/rate"), before);
        std::fs::remove_dir_all(staging).expect("cleanup");
    }
}
