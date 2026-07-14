//! Shared terminal presentation helpers for headless and interactive frontends.

use std::io::{IsTerminal, stderr};
use std::sync::Mutex;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::queue::{QueueItem, QueueState};

mod text;

pub(crate) use text::{
    AccountSummary, account_summary, catalog_rating_label, ellipsize, ellipsize_middle,
    human_bytes, kind_label, locale_label_from_code, locale_name, paint, print_account,
    print_error, print_success, print_warning, safe_failure, selection_label, target_kind_label,
    yes_no,
};

pub(crate) struct CliProgress {
    bar: ProgressBar,
    target: Mutex<String>,
}

impl CliProgress {
    pub(crate) fn new() -> Self {
        let bar = if stderr().is_terminal() {
            ProgressBar::new_spinner()
        } else {
            ProgressBar::hidden()
        };
        bar.set_draw_target(ProgressDrawTarget::stderr_with_hz(12));
        bar.enable_steady_tick(Duration::from_millis(80));
        Self {
            bar,
            target: Mutex::new(String::new()),
        }
    }

    pub(crate) fn start(&self, item: &QueueItem, index: usize, total: usize) {
        let target = format!("{} {}", target_kind_label(&item.target), item.target.id());
        *self.target.lock().expect("progress target lock") = target.clone();
        self.spinner_style();
        self.bar.reset();
        self.bar.set_prefix(format!("{index}/{total}"));
        self.bar.set_message(format!("Preparing {target}"));
    }

    pub(crate) fn success(&self, output: &std::path::Path) {
        self.bar.finish_and_clear();
        print_success(&format!("Saved {}", output.display()));
    }

    pub(crate) fn failure(&self, message: &str) {
        self.bar.finish_and_clear();
        print_error(message);
    }

    pub(crate) fn finish(&self) {
        self.bar.finish_and_clear();
    }

    fn spinner_style(&self) {
        self.bar.set_style(
            ProgressStyle::with_template("{spinner:.cyan} {prefix:.dim} {msg}")
                .expect("valid progress template")
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
        );
    }

    fn count_style(&self) {
        self.bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} {prefix:.dim} {msg}\n  [{wide_bar:.cyan/blue}] {pos}/{len}",
            )
            .expect("valid count progress template")
            .progress_chars("━━╸"),
        );
    }

    fn bytes_style(&self) {
        self.bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.cyan} {prefix:.dim} {msg}\n  [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} • {eta}",
            )
            .expect("valid byte progress template")
            .progress_chars("━━╸"),
        );
    }
}

impl crunchydl::EventSink for CliProgress {
    fn emit(&self, event: crunchydl::DownloadEvent) {
        match event {
            crunchydl::DownloadEvent::StateChanged(state) => {
                self.spinner_style();
                self.bar.set_message(job_state_label(state));
            }
            crunchydl::DownloadEvent::SegmentCompleted {
                completed,
                total,
                completed_bytes,
                total_bytes,
                track,
                ..
            } => {
                let role = track.map_or("media".to_string(), |track| {
                    format!("{:?} {}", track.kind, locale_name(&track.locale)).to_lowercase()
                });
                if let Some(total_bytes) = total_bytes {
                    self.bytes_style();
                    self.bar.set_length(total_bytes);
                    self.bar.set_position(completed_bytes.min(total_bytes));
                } else {
                    self.count_style();
                    self.bar.set_length(total as u64);
                    self.bar.set_position(completed as u64);
                }
                self.bar
                    .set_message(format!("Downloading {role} • {completed}/{total} segments"));
            }
            crunchydl::DownloadEvent::StageProgress {
                state,
                completed,
                total,
            } => {
                self.count_style();
                self.bar.set_length(total as u64);
                self.bar.set_position(completed as u64);
                self.bar.set_message(job_state_label(state));
            }
            crunchydl::DownloadEvent::TransferRetry { attempt, delay, .. } => {
                self.bar.println(format!(
                    "{} Transfer interrupted; retry {attempt} in {:.1}s",
                    paint("!", "33", stderr().is_terminal()),
                    delay.as_secs_f64()
                ));
            }
            crunchydl::DownloadEvent::Warning(warning) => {
                self.bar.println(format!(
                    "{} {warning}",
                    paint("!", "33", stderr().is_terminal())
                ));
            }
            crunchydl::DownloadEvent::OutputCommitted { .. } => {}
            _ => {}
        }
    }
}

fn job_state_label(state: crunchydl::JobState) -> &'static str {
    match state {
        crunchydl::JobState::Created => "Starting download",
        crunchydl::JobState::ResolvingMedia => "Resolving media",
        crunchydl::JobState::OpeningPlaybackSessions => "Opening playback session",
        crunchydl::JobState::PlanningTracks => "Selecting tracks",
        crunchydl::JobState::AcquiringLicenses => "Acquiring DRM licenses",
        crunchydl::JobState::Downloading => "Downloading media",
        crunchydl::JobState::Decrypting => "Decrypting media",
        crunchydl::JobState::ProcessingSubtitles => "Processing subtitles",
        crunchydl::JobState::Muxing => "Building output file",
        crunchydl::JobState::Verifying => "Verifying output",
        crunchydl::JobState::Committing => "Saving output",
        crunchydl::JobState::Completed => "Download complete",
        crunchydl::JobState::Cancelled => "Download cancelled",
        crunchydl::JobState::Failed => "Download failed",
        _ => "Working",
    }
}

pub(crate) fn queue_state_style(state: QueueState, enabled: bool) -> String {
    let (icon, code) = match state {
        QueueState::Pending => ("○ pending", "37"),
        QueueState::Running => ("● downloading", "36"),
        QueueState::Completed => ("✓ completed", "32"),
        QueueState::Failed => ("× failed", "31"),
    };
    paint(icon, code, enabled)
}
