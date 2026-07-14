use std::io::{Read, Seek};

use crate::error::{Error, Result};
use crate::{Codec, Edit, TimeBase, Track, TrackKind};

use super::box_reader::{
    BoxHeader, file_end, full_box, next_box, read_array, read_i32, read_i64, read_payload,
    read_u16, read_u32, read_u64, seek_to,
};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TrackDefaults {
    pub(crate) track_id: u32,
    pub(crate) duration: u32,
    pub(crate) size: u32,
    pub(crate) flags: u32,
}

pub(crate) struct ParsedInit {
    pub(crate) tracks: Vec<Track>,
    pub(crate) defaults: Vec<TrackDefaults>,
    pub(crate) first_fragment: u64,
    pub(crate) file_end: u64,
}

#[derive(Default)]
struct TrackBuilder {
    id: Option<u32>,
    kind: Option<TrackKind>,
    timescale: Option<u32>,
    duration: u64,
    codec: Option<Codec>,
    dimensions: Option<(u32, u32)>,
    sample_rate: Option<u32>,
    channels: Option<u16>,
    raw_edits: Vec<RawEdit>,
}

#[derive(Clone, Copy)]
struct RawEdit {
    movie_duration: u64,
    media_time: i64,
}

pub(crate) fn parse<R: Read + Seek>(reader: &mut R) -> Result<ParsedInit> {
    seek_to(reader, 0)?;
    let end = file_end(reader)?;
    let mut saw_ftyp = false;
    let mut movie_timescale = None;
    let mut tracks = Vec::new();
    let mut defaults = Vec::new();
    let mut first_fragment = None;
    while let Some(header) = next_box(reader, end)? {
        match &header.kind {
            b"ftyp" => saw_ftyp = true,
            b"moov" => {
                let parsed = parse_moov(reader, header)?;
                movie_timescale = Some(parsed.0);
                tracks = parsed.1;
                defaults = parsed.2;
            }
            b"moof" => {
                first_fragment.get_or_insert(header.start);
            }
            _ => {}
        }
        seek_to(reader, header.end)?;
    }
    if !saw_ftyp {
        return Err(Error::Invalid("missing ftyp box"));
    }
    let _movie_timescale = movie_timescale.ok_or(Error::Invalid("missing moov or mvhd box"))?;
    if tracks.is_empty() {
        return Err(Error::Invalid("movie contains no supported tracks"));
    }
    Ok(ParsedInit {
        tracks,
        defaults,
        first_fragment: first_fragment.unwrap_or(end),
        file_end: end,
    })
}

fn parse_moov<R: Read + Seek>(
    reader: &mut R,
    moov: BoxHeader,
) -> Result<(u32, Vec<Track>, Vec<TrackDefaults>)> {
    seek_to(reader, moov.content_start)?;
    let mut movie_timescale = None;
    let mut builders = Vec::new();
    let mut defaults = Vec::new();
    while let Some(child) = next_box(reader, moov.end)? {
        match &child.kind {
            b"mvhd" => movie_timescale = Some(parse_mvhd(reader, child)?),
            b"trak" => builders.push(parse_trak(reader, child)?),
            b"mvex" => parse_mvex(reader, child, &mut defaults)?,
            _ => {}
        }
        seek_to(reader, child.end)?;
    }
    let movie_timescale = movie_timescale.ok_or(Error::Invalid("missing mvhd box"))?;
    let tracks = builders
        .into_iter()
        .map(|builder| finish_track(builder, movie_timescale))
        .collect::<Result<Vec<_>>>()?;
    Ok((movie_timescale, tracks, defaults))
}

fn parse_mvhd<R: Read + Seek>(reader: &mut R, header: BoxHeader) -> Result<u32> {
    seek_to(reader, header.content_start)?;
    let (version, _) = full_box(reader)?;
    match version {
        0 => {
            seek_to(reader, header.content_start + 12)?;
            read_u32(reader)
        }
        1 => {
            seek_to(reader, header.content_start + 20)?;
            read_u32(reader)
        }
        _ => Err(Error::Unsupported("mvhd version")),
    }
}

fn parse_trak<R: Read + Seek>(reader: &mut R, trak: BoxHeader) -> Result<TrackBuilder> {
    let mut builder = TrackBuilder::default();
    seek_to(reader, trak.content_start)?;
    while let Some(child) = next_box(reader, trak.end)? {
        match &child.kind {
            b"tkhd" => builder.id = Some(parse_tkhd(reader, child)?),
            b"mdia" => parse_mdia(reader, child, &mut builder)?,
            b"edts" => parse_edts(reader, child, &mut builder.raw_edits)?,
            _ => {}
        }
        seek_to(reader, child.end)?;
    }
    Ok(builder)
}

fn parse_tkhd<R: Read + Seek>(reader: &mut R, header: BoxHeader) -> Result<u32> {
    seek_to(reader, header.content_start)?;
    let (version, _) = full_box(reader)?;
    match version {
        0 => seek_to(reader, header.content_start + 12)?,
        1 => seek_to(reader, header.content_start + 20)?,
        _ => return Err(Error::Unsupported("tkhd version")),
    }
    read_u32(reader)
}

fn parse_mdia<R: Read + Seek>(
    reader: &mut R,
    mdia: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    seek_to(reader, mdia.content_start)?;
    while let Some(child) = next_box(reader, mdia.end)? {
        match &child.kind {
            b"mdhd" => parse_mdhd(reader, child, builder)?,
            b"hdlr" => builder.kind = Some(parse_hdlr(reader, child)?),
            b"minf" => parse_minf(reader, child, builder)?,
            _ => {}
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn parse_mdhd<R: Read + Seek>(
    reader: &mut R,
    header: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    seek_to(reader, header.content_start)?;
    let (version, _) = full_box(reader)?;
    match version {
        0 => {
            seek_to(reader, header.content_start + 12)?;
            builder.timescale = Some(read_u32(reader)?);
            builder.duration = u64::from(read_u32(reader)?);
        }
        1 => {
            seek_to(reader, header.content_start + 20)?;
            builder.timescale = Some(read_u32(reader)?);
            builder.duration = read_u64(reader)?;
        }
        _ => return Err(Error::Unsupported("mdhd version")),
    }
    Ok(())
}

fn parse_hdlr<R: Read + Seek>(reader: &mut R, header: BoxHeader) -> Result<TrackKind> {
    seek_to(reader, header.content_start + 8)?;
    match &read_array::<4, _>(reader)? {
        b"vide" => Ok(TrackKind::Video),
        b"soun" => Ok(TrackKind::Audio),
        _ => Err(Error::Unsupported("non-audio/video handler")),
    }
}

fn parse_minf<R: Read + Seek>(
    reader: &mut R,
    minf: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    seek_to(reader, minf.content_start)?;
    while let Some(child) = next_box(reader, minf.end)? {
        if &child.kind == b"stbl" {
            parse_stbl(reader, child, builder)?;
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn parse_stbl<R: Read + Seek>(
    reader: &mut R,
    stbl: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    seek_to(reader, stbl.content_start)?;
    while let Some(child) = next_box(reader, stbl.end)? {
        if &child.kind == b"stsd" {
            parse_stsd(reader, child, builder)?;
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn parse_stsd<R: Read + Seek>(
    reader: &mut R,
    stsd: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    seek_to(reader, stsd.content_start)?;
    let (_, _) = full_box(reader)?;
    let count = read_u32(reader)?;
    if count != 1 {
        return Err(Error::Unsupported(
            "sample description count other than one",
        ));
    }
    let entry = next_box(reader, stsd.end)?.ok_or(Error::Truncated {
        context: "sample entry",
    })?;
    match &entry.kind {
        b"avc1" => parse_avc1(reader, entry, builder),
        b"mp4a" => parse_mp4a(reader, entry, builder),
        b"encv" | b"enca" => Err(Error::Unsupported(
            "encrypted sample entry must be decrypted first",
        )),
        _ => Err(Error::Unsupported("sample entry codec")),
    }
}

fn parse_avc1<R: Read + Seek>(
    reader: &mut R,
    entry: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    if entry.payload_len() < 78 {
        return Err(Error::Truncated {
            context: "avc1 sample entry",
        });
    }
    seek_to(reader, entry.content_start + 24)?;
    builder.dimensions = Some((u32::from(read_u16(reader)?), u32::from(read_u16(reader)?)));
    seek_to(reader, entry.content_start + 78)?;
    while let Some(child) = next_box(reader, entry.end)? {
        if &child.kind == b"avcC" {
            builder.codec = Some(Codec::Avc {
                configuration: read_payload(reader, child)?,
            });
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn parse_mp4a<R: Read + Seek>(
    reader: &mut R,
    entry: BoxHeader,
    builder: &mut TrackBuilder,
) -> Result<()> {
    if entry.payload_len() < 28 {
        return Err(Error::Truncated {
            context: "mp4a sample entry",
        });
    }
    seek_to(reader, entry.content_start + 16)?;
    builder.channels = Some(read_u16(reader)?);
    seek_to(reader, entry.content_start + 24)?;
    builder.sample_rate = Some(read_u32(reader)? >> 16);
    seek_to(reader, entry.content_start + 28)?;
    while let Some(child) = next_box(reader, entry.end)? {
        if &child.kind == b"esds" {
            let payload = read_payload(reader, child)?;
            builder.codec = Some(Codec::Aac {
                audio_specific_config: decoder_specific_info(&payload)?,
            });
        }
        seek_to(reader, child.end)?;
    }
    Ok(())
}

fn decoder_specific_info(esds: &[u8]) -> Result<Vec<u8>> {
    if esds.len() < 6 {
        return Err(Error::Truncated { context: "esds" });
    }
    // DecoderSpecificInfo is nested inside ES/DecoderConfig descriptors. A
    // bounded scan is used because descriptor children are not box-aligned.
    for index in 4..esds.len() {
        if esds[index] != 5 {
            continue;
        }
        if let Ok((length, consumed)) = descriptor_length(&esds[index + 1..]) {
            let start = index + 1 + consumed;
            if let Some(end) = start.checked_add(length)
                && end <= esds.len()
                && length > 0
            {
                return Ok(esds[start..end].to_vec());
            }
        }
    }
    Err(Error::Invalid("esds has no DecoderSpecificInfo"))
}

fn descriptor_length(bytes: &[u8]) -> Result<(usize, usize)> {
    let mut value = 0_usize;
    for (index, byte) in bytes.iter().copied().take(4).enumerate() {
        value = value
            .checked_shl(7)
            .ok_or(Error::Overflow("descriptor length"))?
            | usize::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err(Error::Invalid("unterminated descriptor length"))
}

fn parse_edts<R: Read + Seek>(
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

fn parse_mvex<R: Read + Seek>(
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

fn finish_track(builder: TrackBuilder, movie_timescale: u32) -> Result<Track> {
    let id = builder.id.ok_or(Error::Invalid("track has no id"))?;
    let kind = builder.kind.ok_or(Error::Invalid("track has no handler"))?;
    let time_base = TimeBase::new(
        builder
            .timescale
            .ok_or(Error::Invalid("track has no mdhd"))?,
    )?;
    let duration = time_base.duration(builder.duration)?;
    let edits = builder
        .raw_edits
        .into_iter()
        .map(|edit| {
            let duration = u128::from(edit.movie_duration)
                .checked_mul(u128::from(time_base.ticks_per_second()))
                .ok_or(Error::Overflow("edit duration"))?
                / u128::from(movie_timescale);
            Ok(Edit {
                duration: u64::try_from(duration).map_err(|_| Error::Overflow("edit duration"))?,
                media_time: edit.media_time,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Track {
        id,
        kind,
        time_base,
        codec: builder
            .codec
            .ok_or(Error::Invalid("track has no supported codec config"))?,
        dimensions: builder.dimensions,
        sample_rate: builder.sample_rate,
        channels: builder.channels,
        duration,
        edits,
    })
}
