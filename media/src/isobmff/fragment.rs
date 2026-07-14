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

    fn parse_traf(
        &mut self,
        moof: BoxHeader,
        traf: BoxHeader,
        output: &mut Vec<Sample>,
    ) -> Result<()> {
        let mut defaults = None;
        let mut decode_time = None;
        let mut truns = Vec::new();
        seek_to(&mut self.reader, traf.content_start)?;
        while let Some(child) = next_box(&mut self.reader, traf.end)? {
            match &child.kind {
                b"tfhd" => defaults = Some(self.parse_tfhd(moof, child)?),
                b"tfdt" => decode_time = Some(parse_tfdt(&mut self.reader, child)?),
                b"trun" => {
                    if truns.len() >= 1024 {
                        return Err(Error::Unsupported("excessive trun boxes in one fragment"));
                    }
                    truns.push(child);
                }
                _ => {}
            }
            seek_to(&mut self.reader, child.end)?;
        }
        let defaults = defaults.ok_or(Error::Invalid("traf has no tfhd"))?;
        let mut dts = decode_time.unwrap_or_else(|| self.running_dts(defaults.track_id));
        let edit_offset = edit_offset(
            self.tracks
                .iter()
                .find(|track| track.id == defaults.track_id)
                .ok_or(Error::Invalid("tfhd references unknown track"))?,
        )?;
        let mut next_data_offset = None;
        for trun in truns {
            next_data_offset = Some(self.parse_trun(
                trun,
                defaults,
                &mut dts,
                edit_offset,
                next_data_offset,
                output,
            )?);
        }
        self.set_running_dts(defaults.track_id, dts);
        Ok(())
    }

    fn parse_tfhd(&mut self, moof: BoxHeader, header: BoxHeader) -> Result<FragmentDefaults> {
        seek_to(&mut self.reader, header.content_start)?;
        let (_, flags) = full_box(&mut self.reader)?;
        let track_id = read_u32(&mut self.reader)?;
        let trex = self
            .defaults
            .iter()
            .find(|defaults| defaults.track_id == track_id)
            .copied()
            .unwrap_or_default();
        let base_offset = if flags & 0x000001 != 0 {
            read_u64(&mut self.reader)?
        } else if flags & 0x020000 != 0 {
            moof.start
        } else {
            return Err(Error::Unsupported(
                "tfhd without an explicit or moof-relative data base",
            ));
        };
        if flags & 0x000002 != 0 {
            let _sample_description_index = read_u32(&mut self.reader)?;
        }
        let duration = if flags & 0x000008 != 0 {
            read_u32(&mut self.reader)?
        } else {
            trex.duration
        };
        let size = if flags & 0x000010 != 0 {
            read_u32(&mut self.reader)?
        } else {
            trex.size
        };
        let sample_flags = if flags & 0x000020 != 0 {
            read_u32(&mut self.reader)?
        } else {
            trex.flags
        };
        Ok(FragmentDefaults {
            track_id,
            base_offset,
            duration,
            size,
            flags: sample_flags,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_trun(
        &mut self,
        header: BoxHeader,
        defaults: FragmentDefaults,
        dts: &mut u64,
        edit_offset: i64,
        prior_end: Option<u64>,
        output: &mut Vec<Sample>,
    ) -> Result<u64> {
        seek_to(&mut self.reader, header.content_start)?;
        let (version, flags) = full_box(&mut self.reader)?;
        if version > 1 {
            return Err(Error::Unsupported("trun version"));
        }
        let count = read_u32(&mut self.reader)?;
        if count > MAX_SAMPLES_PER_FRAGMENT
            || output.len().saturating_add(count as usize) > MAX_SAMPLES_PER_FRAGMENT as usize
        {
            return Err(Error::Unsupported("excessive samples in one fragment"));
        }
        let data_offset = if flags & 0x000001 != 0 {
            let relative = i64::from(read_i32(&mut self.reader)?);
            add_signed(defaults.base_offset, relative, "trun data offset")?
        } else {
            prior_end.ok_or(Error::Unsupported("first trun has no data offset"))?
        };
        let first_sample_flags = if flags & 0x000004 != 0 {
            Some(read_u32(&mut self.reader)?)
        } else {
            None
        };
        let mut offset = data_offset;
        for index in 0..count {
            let duration = if flags & 0x000100 != 0 {
                read_u32(&mut self.reader)?
            } else {
                defaults.duration
            };
            let size = if flags & 0x000200 != 0 {
                read_u32(&mut self.reader)?
            } else {
                defaults.size
            };
            let sample_flags = if flags & 0x000400 != 0 {
                read_u32(&mut self.reader)?
            } else if index == 0 {
                first_sample_flags.unwrap_or(defaults.flags)
            } else {
                defaults.flags
            };
            let composition_offset = if flags & 0x000800 != 0 {
                if version == 0 {
                    i64::from(read_u32(&mut self.reader)?)
                } else {
                    i64::from(read_i32(&mut self.reader)?)
                }
            } else {
                0
            };
            if duration == 0 || size == 0 {
                return Err(Error::Invalid("sample duration or size is zero"));
            }
            let raw_dts = i64::try_from(*dts).map_err(|_| Error::Overflow("sample dts"))?;
            let edited_dts = raw_dts
                .checked_add(edit_offset)
                .ok_or(Error::Overflow("edited dts"))?;
            let pts = edited_dts
                .checked_add(composition_offset)
                .ok_or(Error::Overflow("sample pts"))?;
            output.push(Sample {
                track_id: defaults.track_id,
                offset,
                size,
                dts: edited_dts,
                pts,
                duration,
                flags: sample_flags,
            });
            offset = offset
                .checked_add(u64::from(size))
                .ok_or(Error::Overflow("sample data offset"))?;
            *dts = dts
                .checked_add(u64::from(duration))
                .ok_or(Error::Overflow("decode timestamp"))?;
        }
        Ok(offset)
    }

    fn running_dts(&self, track_id: u32) -> u64 {
        self.next_dts
            .iter()
            .find_map(|(id, dts)| (*id == track_id).then_some(*dts))
            .unwrap_or(0)
    }

    fn set_running_dts(&mut self, track_id: u32, dts: u64) {
        if let Some((_, value)) = self.next_dts.iter_mut().find(|(id, _)| *id == track_id) {
            *value = dts;
        } else {
            self.next_dts.push((track_id, dts));
        }
    }
}

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
