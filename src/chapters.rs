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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_are_sorted_clamped_and_deduplicated() {
        let events: SkipEvents = serde_json::from_str(
            r#"{"intro":{"start":10,"end":20},"recap":{"start":10,"end":12},"credits":{"start":120,"end":130}}"#,
        )
        .expect("valid fixture");
        assert_eq!(
            from_skip_events(Some(events), Duration::from_secs(100)),
            vec![
                Chapter {
                    start: Duration::ZERO,
                    title: "Episode".to_string()
                },
                Chapter {
                    start: Duration::from_secs(10),
                    title: "Recap".to_string()
                },
                Chapter {
                    start: Duration::from_secs(12),
                    title: "Episode".to_string()
                },
                Chapter {
                    start: Duration::from_secs(100),
                    title: "Credits".to_string()
                }
            ]
        );
    }

    #[test]
    fn event_ends_restore_episode_and_adjacent_ranges_do_not_duplicate() {
        let events: SkipEvents = serde_json::from_str(
            r#"{"recap":{"start":0,"end":5},"intro":{"start":5,"end":10},"credits":{"start":90,"end":100}}"#,
        )
        .expect("valid fixture");
        assert_eq!(
            from_skip_events(Some(events), Duration::from_secs(100)),
            vec![
                Chapter {
                    start: Duration::ZERO,
                    title: "Recap".to_string()
                },
                Chapter {
                    start: Duration::from_secs(5),
                    title: "Intro".to_string()
                },
                Chapter {
                    start: Duration::from_secs(10),
                    title: "Episode".to_string()
                },
                Chapter {
                    start: Duration::from_secs(90),
                    title: "Credits".to_string()
                },
            ]
        );
    }
}
