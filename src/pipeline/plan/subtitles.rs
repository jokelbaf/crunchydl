//! Subtitle resource deduplication and selection.

use super::types::PreparedSubtitle;
use super::*;

pub(super) fn plan_subtitles(
    subtitles: Vec<ApiSubtitle>,
    audio_locales: &[Locale],
    options: &PlanningOptions,
    warnings: &mut Vec<DownloadWarning>,
) -> (Vec<PlannedSubtitle>, Vec<PreparedSubtitle>) {
    let mut unique = Vec::<ApiSubtitle>::new();
    for subtitle in subtitles {
        let identity = url_identity(&subtitle.url);
        if !unique.iter().any(|current| {
            current.locale == subtitle.locale
                && current.format == subtitle.format
                && current.is_caption == subtitle.is_caption
                && url_identity(&current.url) == identity
        }) {
            unique.push(subtitle);
        }
    }
    let infos = unique
        .iter()
        .map(|subtitle| SubtitleTrackInfo {
            locale: subtitle.locale.clone(),
            is_caption: subtitle.is_caption,
            format: subtitle.format.clone(),
        })
        .collect::<Vec<_>>();
    let selected = select_subtitles(&infos, &options.subtitles);
    warnings.extend(selected.warnings);
    let mut prepared = selected
        .tracks
        .into_iter()
        .filter_map(|index| unique.get(index))
        .filter(|subtitle| {
            options.subtitles.include_signs
                || subtitle.is_caption
                || !audio_locales.contains(&subtitle.locale)
        })
        .map(|subtitle| {
            let is_signs = !subtitle.is_caption && audio_locales.contains(&subtitle.locale);
            let language = crate::locale_display_name(&subtitle.locale);
            let title = if subtitle.is_caption {
                format!("{language} (CC)")
            } else if is_signs {
                format!("{language} (Signs)")
            } else {
                language
            };
            let diagnostic = PlannedSubtitle {
                locale: subtitle.locale.clone(),
                format: subtitle.format.clone(),
                is_caption: subtitle.is_caption,
                is_signs,
                resource_identity: url_identity(&subtitle.url),
                title,
                default: false,
                forced: is_signs,
            };
            PreparedSubtitle {
                diagnostic,
                url: subtitle.url.clone(),
            }
        })
        .collect::<Vec<_>>();

    if matches!(options.subtitles.locales, SubtitleLocales::Explicit(_))
        && let Some(subtitle) = prepared
            .iter_mut()
            .find(|subtitle| !subtitle.diagnostic.forced)
    {
        subtitle.diagnostic.default = true;
    }
    let diagnostics = prepared
        .iter()
        .map(|subtitle| subtitle.diagnostic.clone())
        .collect();
    (diagnostics, prepared)
}
