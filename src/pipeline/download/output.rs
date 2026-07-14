//! Container contracts and redacted representation transfer plans.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn mux_matroska(
    temporary: &Path,
    assembled: &[(PathBuf, crate::PlannedTrack)],
    subtitles: &[crate::SubtitleTrack],
    fonts: &[crate::ResolvedFont],
    source_chapters: &[crate::Chapter],
    media: &ResolvedMedia,
    synchronization: &SynchronizationOptions,
    cancellation: &CancellationToken,
) -> Result<Vec<OutputTrack>, Error> {
    let (tracks, packets) = mux_inputs(assembled, subtitles, synchronization, cancellation)?;
    let chapters = source_chapters
        .iter()
        .map(|chapter| mkv::Chapter {
            start: chapter.start,
            title: chapter.title.clone(),
            language: language(&Locale::en_US),
        })
        .collect::<Vec<_>>();
    let attachments = fonts
        .iter()
        .enumerate()
        .map(|(index, font)| mkv::Attachment {
            filename: font.filename.clone(),
            mime_type: font.mime_type.clone(),
            uid: index as u64 + 1,
            data: font.data.clone(),
        })
        .collect::<Vec<_>>();
    let mut output =
        BufWriter::new(File::create(temporary).map_err(|error| path_error(temporary, error))?);
    mkv::Muxer::write_fallible(
        &mut output,
        &tracks,
        packets,
        &chapters,
        &attachments,
        &mkv::MuxOptions {
            title: Some(media.title.clone()),
            ..mkv::MuxOptions::default()
        },
    )
    .map_err(|error| match error {
        mkv::Error::Cancelled => Error::Cancelled,
        error => Error::Mux(error.to_string()),
    })?;
    output
        .get_ref()
        .sync_all()
        .map_err(|error| path_error(temporary, error))?;
    drop(output);
    verify(temporary, tracks.len(), attachments.len(), chapters.len())
}

pub(crate) fn validate_container_contract(
    request: &DownloadRequest,
    prepared: &PreparedPlan,
) -> Result<(), Error> {
    if request.output.container != crate::Container::Mp4 {
        return Ok(());
    }
    if !prepared.subtitles.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not silently discard subtitles; select no subtitles or use Matroska"
                .into(),
        ));
    }
    if !prepared.public.chapters.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not silently discard chapters; disable chapters or use Matroska"
                .into(),
        ));
    }
    if !request.synchronization.offsets.is_empty() {
        return Err(Error::Mux(
            "MP4 output does not support explicit track offsets".into(),
        ));
    }
    let video = prepared
        .public
        .tracks
        .iter()
        .filter(|track| track.kind == PlannedTrackKind::Video)
        .count();
    let audio = prepared
        .public
        .tracks
        .iter()
        .filter(|track| track.kind == PlannedTrackKind::Audio)
        .count();
    if video != 1 || audio == 0 {
        return Err(Error::Mux(
            "MP4 output requires exactly one AVC video and at least one AAC audio track".into(),
        ));
    }
    Ok(())
}

pub(crate) fn source_transfer_plan(
    source: &PlannedSource,
    fingerprint: &str,
    media_id: &str,
) -> crate::RepresentationTransferPlan {
    let request = |segment: &crate::plan::SourceSegment| {
        SegmentRequest::new(
            segment.url.clone(),
            segment.range,
            url_identity(&segment.url),
            segment
                .range
                .map(|(start, end)| end.saturating_sub(start).saturating_add(1)),
        )
    };
    crate::RepresentationTransferPlan {
        media_id: media_id.to_string(),
        version_id: source.diagnostic.version_id.clone(),
        plan_fingerprint: fingerprint.to_string(),
        representation_fingerprint: source.diagnostic.representation_fingerprint.clone(),
        track: Some(crate::TransferTrack {
            kind: source.diagnostic.kind,
            locale: source.diagnostic.locale.clone(),
        }),
        init: request(&source.init),
        segments: source.media.iter().map(request).collect(),
    }
}
