use std::time::Duration;

use crunchyroll_rs::media::{SkipEvents, SkipEventsEvent};

/// A service-neutral chapter point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    /// Chapter start relative to the media timeline.
    pub start: Duration,
    /// Stable, human-readable chapter name.
    pub title: String,
}

pub(crate) fn from_skip_events(events: Option<SkipEvents>, duration: Duration) -> Vec<Chapter> {
    let Some(events) = events else {
        return Vec::new();
    };
    let mut ranges = Vec::new();
    add_event(&mut ranges, "Recap", events.recap, duration, 0);
    add_event(&mut ranges, "Intro", events.intro, duration, 1);
    add_event(&mut ranges, "Credits", events.credits, duration, 2);
    add_event(&mut ranges, "Preview", events.preview, duration, 3);
    ranges.sort_by_key(|range| (range.start, range.priority));
    let mut points = Vec::new();
    let mut cursor = Duration::ZERO;
    for range in ranges {
        if range.start < cursor {
            continue;
        }
        if range.start > cursor {
            points.push(Chapter {
                start: cursor,
                title: "Episode".to_string(),
            });
        }
        points.push(Chapter {
            start: range.start,
            title: range.title.to_string(),
        });
        cursor = range.end.max(range.start);
    }
    if cursor < duration {
        points.push(Chapter {
            start: cursor,
            title: "Episode".to_string(),
        });
    }
    points.dedup_by(|left, right| left.start == right.start);
    points
}

struct ChapterRange {
    start: Duration,
    end: Duration,
    title: &'static str,
    priority: u8,
}

fn add_event(
    ranges: &mut Vec<ChapterRange>,
    title: &'static str,
    event: Option<SkipEventsEvent>,
    duration: Duration,
    priority: u8,
) {
    if let Some(event) = event {
        let start = Duration::from_secs_f32(event.start.max(0.0)).min(duration);
        let end = Duration::from_secs_f32(event.end.max(0.0)).min(duration);
        ranges.push(ChapterRange {
            start,
            end,
            title,
            priority,
        });
    }
}
