//! Edit-list parsing.

use super::*;

pub(super) fn parse_edts<R: Read + Seek>(
    reader: &mut R,
    edts: BoxHeader,
    edits: &mut Vec<RawEdit>,
) -> Result<()> {
    seek_to(reader, edts.content_start)?;
    while let Some(child) = next_box(reader, edts.end)? {
        if &child.kind == b"elst" {
            parse_elst(reader, child, edits)?;
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn parse_elst<R: Read + Seek>(
    reader: &mut R,
    header: BoxHeader,
    edits: &mut Vec<RawEdit>,
) -> Result<()> {
    seek_to(reader, header.content_start)?;
    let (version, _) = full_box(reader)?;
    let count = read_u32(reader)?;
    if count > 1024 {
        return Err(Error::Unsupported("excessive edit-list entries"));
    }
    for _ in 0..count {
        let (movie_duration, media_time) = match version {
            0 => (u64::from(read_u32(reader)?), i64::from(read_i32(reader)?)),
            1 => (read_u64(reader)?, read_i64(reader)?),
            _ => return Err(Error::Unsupported("elst version")),
        };
        let rate_integer = read_u16(reader)?;
        let rate_fraction = read_u16(reader)?;
        if rate_integer != 1 || rate_fraction != 0 {
            return Err(Error::Unsupported("edit-list media rate other than 1.0"));
        }
        edits.push(RawEdit {
            movie_duration,
            media_time,
        });
    }
    Ok(())
}

pub(super) fn parse_mvex<R: Read + Seek>(
    reader: &mut R,
    mvex: BoxHeader,
    defaults: &mut Vec<TrackDefaults>,
) -> Result<()> {
    seek_to(reader, mvex.content_start)?;
    while let Some(child) = next_box(reader, mvex.end)? {
        if &child.kind == b"trex" {
            seek_to(reader, child.content_start)?;
            let (_, _) = full_box(reader)?;
            let track_id = read_u32(reader)?;
            let _description_index = read_u32(reader)?;
            defaults.push(TrackDefaults {
                track_id,
                duration: read_u32(reader)?,
                size: read_u32(reader)?,
                flags: read_u32(reader)?,
            });
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}
