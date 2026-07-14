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

mod movie;
use movie::parse_mvhd;

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

mod codecs;
mod edits;

use codecs::{parse_avc1, parse_mp4a};
use edits::{parse_edts, parse_mvex};

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
