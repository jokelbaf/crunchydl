use crate::element::{binary, master, string, uint};
use crate::error::{Error, Result};

/// A Matroska track role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackType {
    /// Video.
    Video,
    /// Audio.
    Audio,
    /// Text subtitle.
    Subtitle,
}

/// Codec data supported by the focused writer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrackCodec {
    /// AVC/H.264 with an `avcC` decoder configuration record.
    Avc(Vec<u8>),
    /// AAC with an MPEG-4 AudioSpecificConfig.
    Aac(Vec<u8>),
    /// ASS with the script header as codec private data.
    Ass(String),
}

/// Both legacy and modern container language tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    /// ISO 639-2/B three-letter tag.
    pub legacy: String,
    /// BCP 47 language tag.
    pub ietf: String,
}

/// Video display settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSettings {
    /// Display width in pixels.
    pub width: u32,
    /// Display height in pixels.
    pub height: u32,
}

/// Audio playback settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSettings {
    /// Sampling frequency in hertz.
    pub sampling_frequency: f64,
    /// Channel count.
    pub channels: u16,
}

/// Type-specific track settings.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum TrackSettings {
    /// Video dimensions.
    Video(VideoSettings),
    /// Audio frequency and channels.
    Audio(AudioSettings),
    /// No additional settings for an ASS track.
    Subtitle,
}

/// Metadata for one Matroska track.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    /// Stable one-based track number, or zero to assign by input order.
    pub number: u64,
    /// Stable nonzero UID, or zero to derive deterministically.
    pub uid: u64,
    /// Video, audio, or subtitle role.
    pub track_type: TrackType,
    /// Supported codec and private configuration.
    pub codec: TrackCodec,
    /// Type-specific settings.
    pub settings: TrackSettings,
    /// Human-readable track name.
    pub name: Option<String>,
    /// Legacy and BCP 47 language identifiers.
    pub language: Option<Language>,
    /// Whether this track should be selected by default.
    pub default: bool,
    /// Whether this track must be shown or played.
    pub forced: bool,
    /// Hearing-impaired accessibility flag.
    pub hearing_impaired: bool,
    /// Visual-impaired accessibility flag.
    pub visual_impaired: bool,
    /// Native-language flag.
    pub original: bool,
    /// Commentary flag.
    pub commentary: bool,
}

impl Track {
    pub(crate) fn validate(&self) -> Result<()> {
        match (&self.track_type, &self.codec, &self.settings) {
            (TrackType::Video, TrackCodec::Avc(data), TrackSettings::Video(video))
                if !data.is_empty() && video.width > 0 && video.height > 0 => {}
            (TrackType::Audio, TrackCodec::Aac(data), TrackSettings::Audio(audio))
                if !data.is_empty()
                    && audio.sampling_frequency.is_finite()
                    && audio.sampling_frequency > 0.0
                    && audio.channels > 0 => {}
            (TrackType::Subtitle, TrackCodec::Ass(header), TrackSettings::Subtitle)
                if !header.is_empty() => {}
            _ => {
                return Err(Error::Invalid(
                    "track type, codec, and settings do not match",
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn encode(tracks: &[Track]) -> Result<Vec<u8>> {
    let mut entries = Vec::with_capacity(tracks.len());
    for (index, track) in tracks.iter().enumerate() {
        track.validate()?;
        let number = if track.number == 0 {
            index as u64 + 1
        } else {
            track.number
        };
        let uid = if track.uid == 0 {
            derived_uid(number, track)
        } else {
            track.uid
        };
        let type_number = match track.track_type {
            TrackType::Video => 1,
            TrackType::Audio => 2,
            TrackType::Subtitle => 17,
        };
        let (codec_id, private) = match &track.codec {
            TrackCodec::Avc(data) => ("V_MPEG4/ISO/AVC", data.as_slice()),
            TrackCodec::Aac(data) => ("A_AAC", data.as_slice()),
            TrackCodec::Ass(header) => ("S_TEXT/ASS", header.as_bytes()),
        };
        let mut children = vec![
            uint(0xd7, number)?,
            uint(0x73c5, uid)?,
            uint(0x83, type_number)?,
            uint(0xb9, 1)?,
            uint(0x88, u64::from(track.default))?,
            uint(0x55aa, u64::from(track.forced))?,
            uint(0x9c, 0)?,
            uint(0x55ab, u64::from(track.hearing_impaired))?,
            uint(0x55ac, u64::from(track.visual_impaired))?,
            uint(0x55ae, u64::from(track.original))?,
            uint(0x55af, u64::from(track.commentary))?,
            string(0x86, codec_id)?,
            binary(0x63a2, private)?,
        ];
        if let Some(name) = &track.name {
            children.push(string(0x536e, name)?);
        }
        if let Some(language) = &track.language {
            children.push(string(0x22b59c, &language.legacy)?);
            children.push(string(0x22b59d, &language.ietf)?);
        }
        match track.settings {
            TrackSettings::Video(video) => children.push(master(
                0xe0,
                [
                    uint(0xb0, u64::from(video.width))?,
                    uint(0xba, u64::from(video.height))?,
                ],
            )?),
            TrackSettings::Audio(audio) => children.push(master(
                0xe1,
                [
                    crate::element::float64(0xb5, audio.sampling_frequency)?,
                    uint(0x9f, u64::from(audio.channels))?,
                ],
            )?),
            TrackSettings::Subtitle => {}
        }
        entries.push(master(0xae, children)?);
    }
    master(0x1654_ae6b, entries)
}

fn derived_uid(number: u64, track: &Track) -> u64 {
    let role = match track.track_type {
        TrackType::Video => 1,
        TrackType::Audio => 2,
        TrackType::Subtitle => 3,
    };
    0x4352_4e43_4800_0000_u64 | (role << 8) | number
}
