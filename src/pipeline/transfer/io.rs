//! HTTP request and retry mechanics for transfer workers.

use std::time::Duration;

use reqwest::StatusCode;

use super::*;

impl TransferEngine {
    pub(super) async fn fetch(
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

    pub(super) async fn retry_delay(
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
