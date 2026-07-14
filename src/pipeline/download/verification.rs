//! Output verification, atomic commit, and staging-retention policy.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::archive::{Archive, ArchiveEntry, ArchiveKey};
use crate::{Error, OverwritePolicy, ResolvedMedia};

use super::{OutputTrack, RetentionPolicy};

pub(super) fn verify(
    path: &Path,
    track_count: usize,
    attachment_count: usize,
    chapter_count: usize,
) -> Result<Vec<OutputTrack>, Error> {
    let parsed =
        matroska_reader::open(path).map_err(|error| Error::Verification(error.to_string()))?;
    if parsed.tracks.len() != track_count
        || parsed.attachments.len() != attachment_count
        || parsed
            .chapters
            .iter()
            .map(|edition| edition.chapters.len())
            .sum::<usize>()
            != chapter_count
        || parsed
            .info
            .duration
            .is_none_or(|duration| duration.is_zero())
    {
        return Err(Error::Verification(
            "Matroska structural verification failed".into(),
        ));
    }
    let markers = scan_markers(path)?;
    if !markers.cluster || !markers.block {
        return Err(Error::Verification(
            "Matroska contains no media blocks".into(),
        ));
    }
    Ok(parsed
        .tracks
        .into_iter()
        .map(|track| OutputTrack {
            codec: track.codec_id,
            language: track.language.map(|language| match language {
                matroska_reader::Language::ISO639(value)
                | matroska_reader::Language::IETF(value) => value,
            }),
            name: track.name,
            default: track.default,
            forced: track.forced,
        })
        .collect())
}

pub(super) struct StructuralMarkers {
    pub(super) cluster: bool,
    pub(super) block: bool,
}

pub(super) fn scan_markers(path: &Path) -> Result<StructuralMarkers, Error> {
    let mut reader = BufReader::new(File::open(path).map_err(|error| path_error(path, error))?);
    let mut carry = Vec::new();
    let mut markers = StructuralMarkers {
        cluster: false,
        block: false,
    };
    loop {
        let mut chunk = vec![0; 64 * 1024];
        let read = reader
            .read(&mut chunk)
            .map_err(|error| path_error(path, error))?;
        if read == 0 {
            break;
        }
        chunk.truncate(read);
        carry.extend(chunk);
        markers.cluster |= carry
            .windows(4)
            .any(|window| window == [0x1f, 0x43, 0xb6, 0x75]);
        markers.block |= carry.contains(&0xa3);
        if carry.len() > 7 {
            carry.drain(..carry.len() - 7);
        }
    }
    Ok(markers)
}

pub(super) fn commit(
    temporary: &Path,
    destination: &Path,
    overwrite: OverwritePolicy,
) -> Result<(), Error> {
    if destination.exists() && overwrite != OverwritePolicy::Replace {
        return Err(Error::Filesystem(format!(
            "output already exists: {}",
            destination.display()
        )));
    }
    match std::fs::rename(temporary, destination) {
        Ok(()) => {}
        Err(error)
            if overwrite == OverwritePolicy::Replace
                && error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            std::fs::remove_file(destination).map_err(|error| path_error(destination, error))?;
            std::fs::rename(temporary, destination)
                .map_err(|error| path_error(destination, error))?;
        }
        Err(error) => return Err(path_error(destination, error)),
    }
    if let Some(parent) = destination.parent()
        && let Ok(directory) = File::open(parent)
    {
        let _ = directory.sync_all();
    }
    Ok(())
}

pub(super) fn commit_output(
    temporary: &Path,
    destination: &Path,
    overwrite: OverwritePolicy,
    archive: Option<&dyn Archive>,
    archive_key: &ArchiveKey,
    tracks: &[OutputTrack],
) -> Result<(), Error> {
    commit(temporary, destination, overwrite)?;
    if let Some(archive) = archive {
        archive.record(&ArchiveEntry {
            key: archive_key.clone(),
            output: destination.to_path_buf(),
            tracks: tracks.to_vec(),
        })?;
    }
    Ok(())
}

pub(super) fn cleanup_staging(path: &Path, retention: RetentionPolicy, succeeded: bool) {
    let retain = matches!(
        (retention, succeeded),
        (RetentionPolicy::KeepAlways, _) | (RetentionPolicy::KeepOnFailure, false)
    );
    if !retain {
        let _ = std::fs::remove_dir_all(path);
    }
}

pub(super) fn validate_media(media: &ResolvedMedia) -> Result<(), Error> {
    if media.content_id.is_empty()
        || media
            .availability_status
            .eq_ignore_ascii_case("unavailable")
    {
        return Err(Error::Unavailable(media.content_id.clone()));
    }
    Ok(())
}

pub(super) fn safe_component(value: &str) -> String {
    let value = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    if value.is_empty() {
        "media".into()
    } else {
        value
    }
}

pub(super) fn path_error(path: &Path, error: std::io::Error) -> Error {
    Error::Filesystem(format!("{}: {error}", path.display()))
}
