//! Shared terminal presentation helpers for headless and interactive frontends.

use std::io::{IsTerminal, stderr, stdout};
use std::sync::Mutex;
use std::time::Duration;

use crunchyroll_rs::Locale;
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::queue::{QueueItem, QueueState};

#[derive(Clone, Debug)]
pub(crate) struct AccountSummary {
    pub(crate) name: String,
    pub(crate) email: Option<String>,
    pub(crate) premium: bool,
}

pub(crate) async fn account_summary(client: &crunchyroll_rs::Crunchyroll) -> AccountSummary {
    let (account, profiles) = tokio::join!(client.account(), client.profiles());
    let email = account
        .ok()
        .map(|account| account.email)
        .filter(|email| !email.trim().is_empty());
    let profile_name = profiles.ok().and_then(|profiles| {
        let mut profiles = profiles.profiles;
        let fallback = profiles.first().cloned();
        profiles
            .drain(..)
            .find(|profile| profile.is_selected)
            .or(fallback)
            .map(|profile| profile.profile_name)
            .filter(|name| !name.trim().is_empty())
    });
    AccountSummary {
        name: profile_name
            .or_else(|| email.clone())
            .unwrap_or_else(|| "Crunchyroll user".to_string()),
        email,
        premium: client.premium().await,
    }
}

pub(crate) fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

pub(crate) fn kind_label(kind: crunchydl::CatalogKind) -> &'static str {
    match kind {
        crunchydl::CatalogKind::Series => "Series",
        crunchydl::CatalogKind::Season => "Season",
        crunchydl::CatalogKind::Episode => "Episode",
        crunchydl::CatalogKind::MovieListing => "Movies",
        crunchydl::CatalogKind::Movie => "Movie",
        crunchydl::CatalogKind::MusicVideo => "Music video",
        crunchydl::CatalogKind::Concert => "Concert",
        _ => "Media",
    }
}

pub(crate) fn catalog_rating_label(rating: Option<&crunchydl::CatalogRating>) -> String {
    match rating {
        Some(crunchydl::CatalogRating::Stars { average, total }) => total.map_or_else(
            || format!("{average:.1}/5"),
            |total| format!("{average:.1}/5 ({total})"),
        ),
        Some(crunchydl::CatalogRating::Approval { percentage, total }) => total.map_or_else(
            || format!("{percentage:.0}% positive"),
            |total| format!("{percentage:.0}% ({total})"),
        ),
        _ => "not rated".to_string(),
    }
}

pub(crate) fn target_kind_label(target: &crunchydl::MediaTarget) -> &'static str {
    match target {
        crunchydl::MediaTarget::Episode(_) => "Episode",
        crunchydl::MediaTarget::Movie(_) => "Movie",
        crunchydl::MediaTarget::MusicVideo(_) => "Music video",
        _ => "Media",
    }
}

pub(crate) fn locale_name(locale: &Locale) -> String {
    locale_name_from_code(&locale.to_string())
}

pub(crate) fn locale_name_from_code(code: &str) -> String {
    crunchydl::locale_display_name(&Locale::from(code.to_string()))
}

pub(crate) fn locale_label_from_code(code: &str) -> String {
    let name = locale_name_from_code(code);
    if name.eq_ignore_ascii_case(code) {
        code.to_string()
    } else {
        format!("{name} [{code}]")
    }
}

pub(crate) fn selection_label(item: &QueueItem) -> String {
    let audio = if item.selection.all_audio {
        "all dubs".to_string()
    } else if item.selection.audio_locales.is_empty() {
        "original audio".to_string()
    } else {
        item.selection
            .audio_locales
            .iter()
            .map(|locale| locale_label_from_code(locale))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let subtitles = if item.selection.no_subtitles {
        "no subtitles".to_string()
    } else if item.selection.subtitle_locales.is_empty() {
        "all subtitles".to_string()
    } else {
        format!(
            "{} subtitles",
            item.selection
                .subtitle_locales
                .iter()
                .map(|locale| locale_label_from_code(locale))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let quality = item.selection.max_height.map_or_else(
        || "best quality".to_string(),
        |height| format!("up to {height}p"),
    );
    format!(
        "{audio} • {subtitles} • {quality} • {}",
        item.selection.format
    )
}

pub(crate) fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn ellipsize(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", value.chars().take(keep).collect::<String>())
}

/// Hide signed request URLs retained by queue documents written by older
/// versions. New transfer errors are already redacted at their source.
pub(crate) fn safe_failure(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    [" for url (http", " for url http"]
        .into_iter()
        .filter_map(|marker| lower.find(marker))
        .min()
        .map_or_else(
            || value.to_string(),
            |index| format!("{} (request URL redacted)", value[..index].trim_end()),
        )
}

pub(crate) fn print_account(summary: &AccountSummary) {
    println!(
        "{} {}",
        paint("✓", "32", stdout().is_terminal()),
        paint(
            &format!("Logged in as {}", summary.name),
            "1;36",
            stdout().is_terminal()
        )
    );
    if summary
        .email
        .as_deref()
        .is_some_and(|email| email != summary.name)
    {
        println!(
            "  Email    {}",
            summary.email.as_deref().unwrap_or_default()
        );
    }
    println!("  Premium  {}", yes_no(summary.premium));
}

pub(crate) fn print_success(message: &str) {
    println!("{} {message}", paint("✓", "32", stdout().is_terminal()));
}

pub(crate) fn print_warning(message: &str) {
    eprintln!("{} {message}", paint("!", "33", stderr().is_terminal()));
}

pub(crate) fn print_error(message: &str) {
    eprintln!(
        "{} {message}",
        paint("error", "1;31", stderr().is_terminal())
    );
}

pub(crate) fn paint(value: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{code}m{value}\x1b[0m")
    } else {
        value.to_string()
    }
}

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
    paint(&format!("{icon:<15}"), code, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_helpers_are_human_readable() {
        assert_eq!(yes_no(true), "yes");
        assert_eq!(human_bytes(1_572_864), "1.5 MiB");
        assert_eq!(ellipsize("abcdefgh", 5), "abcd…");
        assert_eq!(locale_name_from_code("es-419"), "Español (América Latina)");
        assert_eq!(locale_label_from_code("ja-JP"), "Japanese [ja-JP]");
        assert_eq!(locale_label_from_code("x-custom"), "x-custom");
        assert_eq!(
            safe_failure("body failed for url (https://example.test/?token=secret)"),
            "body failed (request URL redacted)"
        );
    }
}
