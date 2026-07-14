mod box_reader;
mod fragment;
mod init;

pub use fragment::PacketIter;

use std::io::{Read, Seek};

use crate::{Probe, Track, error::Result};

/// A bounded-memory reader for concatenated fragmented MP4 media.
pub struct FragmentedMp4<R> {
    reader: R,
    tracks: Vec<Track>,
    defaults: Vec<init::TrackDefaults>,
    first_fragment: u64,
    file_end: u64,
}

impl<R: Read + Seek> FragmentedMp4<R> {
    /// Parse initialization metadata without loading media fragments.
    ///
    /// # Errors
    ///
    /// Returns a typed error for malformed or unsupported layouts and I/O
    /// failures from the source.
    pub fn open(mut reader: R) -> Result<Self> {
        let parsed = init::parse(&mut reader)?;
        Ok(Self {
            reader,
            tracks: parsed.tracks,
            defaults: parsed.defaults,
            first_fragment: parsed.first_fragment,
            file_end: parsed.file_end,
        })
    }

    /// Return initialization metadata for every track.
    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Consume the reader and stream packets one fragment at a time.
    #[must_use]
    pub fn packets(self) -> PacketIter<R> {
        PacketIter::new(
            self.reader,
            self.tracks,
            self.defaults,
            self.first_fragment,
            self.file_end,
        )
    }

    /// Iterate every packet and return aggregate probe data.
    ///
    /// # Errors
    ///
    /// Returns the first packet parsing or input error.
    pub fn probe(self) -> Result<Probe> {
        let tracks = self.tracks.clone();
        let mut packet_count = 0_u64;
        let mut byte_count = 0_u64;
        let mut duration = std::time::Duration::ZERO;
        for packet in self.packets() {
            let packet = packet?;
            packet_count = packet_count
                .checked_add(1)
                .ok_or(crate::Error::Overflow("packet count"))?;
            byte_count = byte_count
                .checked_add(packet.data.len() as u64)
                .ok_or(crate::Error::Overflow("packet byte count"))?;
            let end = packet
                .pts
                .ticks()
                .saturating_add(i64::from(packet.duration));
            if let Ok(end) = u64::try_from(end) {
                duration = duration.max(packet.pts.time_base().duration(end)?);
            }
        }
        Ok(Probe {
            tracks,
            duration,
            packet_count,
            byte_count,
        })
    }
}
