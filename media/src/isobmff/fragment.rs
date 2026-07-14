use std::collections::VecDeque;
use std::io::{Read, Seek};

use crate::error::{Error, Result};
use crate::{Packet, Timestamp, Track};

use super::box_reader::{
    BoxHeader, MAX_SAMPLES_PER_FRAGMENT, full_box, next_box, read_i32, read_u32, read_u64, seek_to,
};
use super::init::TrackDefaults;

#[derive(Clone, Copy)]
struct FragmentDefaults {
    track_id: u32,
    base_offset: u64,
    duration: u32,
    size: u32,
    flags: u32,
}

struct Sample {
    track_id: u32,
    offset: u64,
    size: u32,
    dts: i64,
    pts: i64,
    duration: u32,
    flags: u32,
}

/// Streaming packet iterator returned by [`super::FragmentedMp4::packets`].
///
/// Only one fragment's compact sample table and one sample payload are held in
/// memory at a time.
pub struct PacketIter<R> {
    reader: R,
    tracks: Vec<Track>,
    defaults: Vec<TrackDefaults>,
    next_dts: Vec<(u32, u64)>,
    cursor: u64,
    file_end: u64,
    pending: VecDeque<Sample>,
    failed: bool,
}

impl<R: Read + Seek> PacketIter<R> {
    pub(crate) fn new(
        reader: R,
        tracks: Vec<Track>,
        defaults: Vec<TrackDefaults>,
        first_fragment: u64,
        file_end: u64,
    ) -> Self {
        Self {
            reader,
            tracks,
            defaults,
            next_dts: Vec::new(),
            cursor: first_fragment,
            file_end,
            pending: VecDeque::new(),
            failed: false,
        }
    }

    fn read_next(&mut self) -> Result<Option<Packet>> {
        while self.pending.is_empty() {
            if !self.load_fragment()? {
                return Ok(None);
            }
        }
        let sample = self.pending.pop_front().expect("pending was checked");
        let track = self
            .tracks
            .iter()
            .find(|track| track.id == sample.track_id)
            .ok_or(Error::Invalid("fragment references unknown track"))?;
        let end = sample
            .offset
            .checked_add(u64::from(sample.size))
            .ok_or(Error::Overflow("sample end"))?;
        if end > self.file_end {
            return Err(Error::Truncated {
                context: "sample payload",
            });
        }
        seek_to(&mut self.reader, sample.offset)?;
        let len = usize::try_from(sample.size).map_err(|_| Error::Overflow("sample size"))?;
        let mut data = vec![0; len];
        self.reader.read_exact(&mut data).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                Error::Truncated {
                    context: "sample payload",
                }
            } else {
                Error::Io(error)
            }
        })?;
        Ok(Some(Packet {
            track_id: sample.track_id,
            dts: Timestamp::new(sample.dts, track.time_base),
            pts: Timestamp::new(sample.pts, track.time_base),
            duration: sample.duration,
            is_keyframe: sample.flags & 0x0001_0000 == 0,
            data,
        }))
    }

    fn load_fragment(&mut self) -> Result<bool> {
        seek_to(&mut self.reader, self.cursor)?;
        let moof = loop {
            let Some(header) = next_box(&mut self.reader, self.file_end)? else {
                self.cursor = self.file_end;
                return Ok(false);
            };
            self.cursor = header.end;
            if &header.kind == b"moof" {
                break header;
            }
            seek_to(&mut self.reader, header.end)?;
        };
        let mut samples = Vec::new();
        seek_to(&mut self.reader, moof.content_start)?;
        while let Some(child) = next_box(&mut self.reader, moof.end)? {
            if &child.kind == b"traf" {
                self.parse_traf(moof, child, &mut samples)?;
            }
            seek_to(&mut self.reader, child.end)?;
        }
        samples.sort_by_key(|sample| sample.offset);
        self.pending = samples.into();
        self.cursor = moof.end;
        if self.pending.is_empty() {
            return Err(Error::Invalid("moof contains no media samples"));
        }
        Ok(true)
    }
}

mod parsing;

impl<R: Read + Seek> Iterator for PacketIter<R> {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        match self.read_next() {
            Ok(Some(packet)) => Some(Ok(packet)),
            Ok(None) => None,
            Err(error) => {
                self.failed = true;
                Some(Err(error))
            }
        }
    }
}

fn parse_tfdt<R: Read + Seek>(reader: &mut R, header: BoxHeader) -> Result<u64> {
    seek_to(reader, header.content_start)?;
    let (version, _) = full_box(reader)?;
    match version {
        0 => Ok(u64::from(read_u32(reader)?)),
        1 => read_u64(reader),
        _ => Err(Error::Unsupported("tfdt version")),
    }
}

fn edit_offset(track: &Track) -> Result<i64> {
    let mut empty_duration = 0_u64;
    let mut media_time = None;
    for edit in &track.edits {
        if edit.media_time == -1 && media_time.is_none() {
            empty_duration = empty_duration
                .checked_add(edit.duration)
                .ok_or(Error::Overflow("empty edit duration"))?;
        } else if edit.media_time >= 0 && media_time.is_none() {
            media_time = Some(edit.media_time);
        } else {
            return Err(Error::Unsupported("complex edit list"));
        }
    }
    let empty = i64::try_from(empty_duration).map_err(|_| Error::Overflow("edit offset"))?;
    empty
        .checked_sub(media_time.unwrap_or(0))
        .ok_or(Error::Overflow("edit offset"))
}

fn add_signed(base: u64, relative: i64, context: &'static str) -> Result<u64> {
    if relative >= 0 {
        base.checked_add(relative.unsigned_abs())
            .ok_or(Error::Overflow(context))
    } else {
        base.checked_sub(relative.unsigned_abs())
            .ok_or(Error::Invalid("negative data offset before file start"))
    }
}
