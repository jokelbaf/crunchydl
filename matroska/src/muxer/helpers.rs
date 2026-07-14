//! Matroska segment layout and patching helpers.

use super::*;

pub(super) fn validate_tracks(tracks: &[Track]) -> Result<()> {
    if tracks.is_empty() {
        return Err(Error::Invalid("no tracks"));
    }
    let mut numbers = Vec::new();
    let mut video_defaults = 0;
    for (index, track) in tracks.iter().enumerate() {
        track.validate()?;
        let number = effective_number(index, track);
        if number == 0 || numbers.contains(&number) {
            return Err(Error::Invalid("track numbers must be unique and nonzero"));
        }
        numbers.push(number);
        if track.track_type == TrackType::Video && track.default {
            video_defaults += 1;
        }
    }
    if video_defaults > 1 {
        return Err(Error::Invalid("multiple default video tracks"));
    }
    Ok(())
}

pub(super) fn effective_number(index: usize, track: &Track) -> u64 {
    if track.number == 0 {
        index as u64 + 1
    } else {
        track.number
    }
}

pub(super) fn ebml_header() -> Result<Vec<u8>> {
    master(
        0x1a45_dfa3,
        [
            uint(0x4286, 1)?,
            uint(0x42f7, 1)?,
            uint(0x42f2, 4)?,
            uint(0x42f3, 8)?,
            string(0x4282, "matroska")?,
            uint(0x4287, 4)?,
            uint(0x4285, 2)?,
        ],
    )
}

pub(super) fn write_info<W: Write + Seek>(output: &mut W, options: &MuxOptions) -> Result<u64> {
    let mut children = vec![
        uint(0x002a_d7b1, 1_000_000)?,
        string(0x4d80, &options.muxing_application)?,
        string(0x5741, &options.writing_application)?,
    ];
    if let Some(title) = &options.title {
        children.push(string(0x7ba9, title)?);
    }
    let info_start = output.stream_position()?;
    let duration = float64(0x4489, 0.0)?;
    children.push(duration);
    let encoded = master(0x1549_a966, children)?;
    let marker = id_bytes(0x4489)?;
    let offset = encoded
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or(Error::Invalid("duration marker missing"))?;
    let payload_position = info_start + offset as u64 + marker.len() as u64 + 1;
    output.write_all(&encoded)?;
    Ok(payload_position)
}

pub(super) fn write_cluster<W: Write>(output: &mut W, payload: &[u8]) -> Result<()> {
    output.write_all(&crate::element::raw(0x1f43_b675, payload)?)?;
    Ok(())
}
pub(super) fn relative(position: u64, start: u64) -> Result<u64> {
    position
        .checked_sub(start)
        .ok_or(Error::Overflow("relative file offset"))
}
pub(super) fn write_void<W: Write>(output: &mut W, total: usize) -> Result<()> {
    let id = id_bytes(0xec)?;
    for width in 1..=8 {
        if total > id.len() + width {
            let payload = total - id.len() - width;
            if let Ok(size) = size_bytes(payload as u64, Some(width)) {
                output.write_all(&id)?;
                output.write_all(&size)?;
                output.write_all(&vec![0; payload])?;
                return Ok(());
            }
        }
    }
    Err(Error::Invalid("void reserve cannot be encoded"))
}

pub(super) fn patch_duration<W: Write + Seek>(
    output: &mut W,
    position: u64,
    duration: Duration,
) -> Result<()> {
    output.seek(SeekFrom::Start(position))?;
    output.write_all(&(duration.as_secs_f64() * 1000.0).to_be_bytes())?;
    Ok(())
}

pub(super) fn patch_seek_head<W: Write + Seek>(
    output: &mut W,
    position: u64,
    base: &[Entry],
    chapters: Option<u64>,
    attachments: Option<u64>,
) -> Result<()> {
    let mut entries = base
        .iter()
        .map(|entry| Entry {
            id: entry.id,
            position: entry.position,
        })
        .collect::<Vec<_>>();
    if let Some(position) = chapters {
        entries.push(Entry {
            id: 0x1043_a770,
            position,
        });
    }
    if let Some(position) = attachments {
        entries.push(Entry {
            id: 0x1941_a469,
            position,
        });
    }
    let encoded = seek::encode(&entries)?;
    if encoded.len() + 2 > SEEK_RESERVE {
        return Err(Error::Overflow("seek head reserve"));
    }
    output.seek(SeekFrom::Start(position))?;
    output.write_all(&encoded)?;
    write_void(output, SEEK_RESERVE - encoded.len())
}

pub(super) fn patch_segment_size<W: Write + Seek>(
    output: &mut W,
    position: u64,
    size: u64,
) -> Result<()> {
    output.seek(SeekFrom::Start(position))?;
    output.write_all(&size_bytes(size, Some(8))?)?;
    Ok(())
}
