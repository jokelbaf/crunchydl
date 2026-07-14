//! Matroska track construction and packet interleaving.

use super::*;

pub(crate) fn mux_inputs(
    paths: &[(PathBuf, crate::PlannedTrack)],
    subtitles: &[crate::SubtitleTrack],
    synchronization: &SynchronizationOptions,
    cancellation: &CancellationToken,
) -> Result<(Vec<mkv::Track>, PacketStreams), Error> {
    let mut tracks = Vec::new();
    let mut streams = Vec::new();
    let mut audio_default = true;
    let mut video_default = true;
    let mut reference_audio_duration: Option<Duration> = None;
    for (path, diagnostic) in paths {
        let parsed = FragmentedMp4::open(BufReader::new(
            File::open(path).map_err(|error| path_error(path, error))?,
        ))?;
        let media_track = parsed.tracks()[0].clone();
        let number = tracks.len() as u64 + 1;
        let offset = synchronization
            .offsets
            .iter()
            .find_map(|(locale, offset)| (locale == &diagnostic.locale).then_some(*offset))
            .unwrap_or(0);
        let (track_type, codec, settings, default) = match (&media_track.kind, &media_track.codec) {
            (TrackKind::Video, Codec::Avc { configuration }) => (
                mkv::TrackType::Video,
                mkv::TrackCodec::Avc(configuration.clone()),
                mkv::TrackSettings::Video(mkv::VideoSettings {
                    width: media_track
                        .dimensions
                        .ok_or_else(|| Error::Mux("video dimensions missing".into()))?
                        .0,
                    height: media_track
                        .dimensions
                        .ok_or_else(|| Error::Mux("video dimensions missing".into()))?
                        .1,
                }),
                {
                    let default = video_default;
                    video_default = false;
                    default
                },
            ),
            (
                TrackKind::Audio,
                Codec::Aac {
                    audio_specific_config,
                },
            ) => {
                let actual_duration = FragmentedMp4::open(BufReader::new(
                    File::open(path).map_err(|error| path_error(path, error))?,
                ))?
                .probe()?
                .duration;
                if let Some(reference) = reference_audio_duration {
                    let difference = reference.abs_diff(actual_duration);
                    if synchronization.strict && difference > synchronization.tolerance {
                        return Err(Error::IncompatibleTimelines {
                            locale: diagnostic.locale.to_string(),
                        });
                    }
                } else {
                    reference_audio_duration = Some(actual_duration);
                }
                let value = (
                    mkv::TrackType::Audio,
                    mkv::TrackCodec::Aac(audio_specific_config.clone()),
                    mkv::TrackSettings::Audio(mkv::AudioSettings {
                        sampling_frequency: f64::from(
                            media_track
                                .sample_rate
                                .ok_or_else(|| Error::Mux("audio sampling rate missing".into()))?,
                        ),
                        channels: media_track
                            .channels
                            .ok_or_else(|| Error::Mux("audio channels missing".into()))?,
                    }),
                    audio_default,
                );
                audio_default = false;
                value
            }
            _ => return Err(Error::Mux("unsupported media codec".into())),
        };
        tracks.push(mkv::Track {
            number,
            uid: 0,
            track_type,
            codec,
            settings,
            name: (track_type == mkv::TrackType::Audio)
                .then(|| crate::locale_display_name(&diagnostic.locale)),
            language: (track_type == mkv::TrackType::Audio).then(|| language(&diagnostic.locale)),
            default,
            forced: false,
            hearing_impaired: false,
            visual_impaired: false,
            original: default,
            commentary: false,
        });
        let packets = parsed.packets().map(move |packet| {
            let packet = packet.map_err(|error| mkv::Error::PacketSource(error.to_string()))?;
            let decode_time_ms = timestamp_millis(packet.dts)
                .map_err(|error| mkv::Error::PacketSource(error.to_string()))?;
            let presentation_time_ms = timestamp_millis(packet.pts)
                .map_err(|error| mkv::Error::PacketSource(error.to_string()))?;
            Ok(mkv::Packet {
                track_number: number,
                decode_time_ms: decode_time_ms
                    .checked_add(offset)
                    .ok_or(mkv::Error::Overflow("explicit track decode offset"))?,
                presentation_time_ms: presentation_time_ms
                    .checked_add(offset)
                    .ok_or(mkv::Error::Overflow("explicit track presentation offset"))?,
                duration: packet
                    .pts
                    .time_base()
                    .duration(u64::from(packet.duration))
                    .map_err(|error| mkv::Error::PacketSource(error.to_string()))?,
                keyframe: packet.is_keyframe,
                data: packet.data,
            })
        });
        streams.push(PacketStream::new(Box::new(packets)));
    }
    for subtitle in subtitles {
        let number = tracks.len() as u64 + 1;
        let (header, packets) = ass_packets(&subtitle.ass, number)?;
        tracks.push(mkv::Track {
            number,
            uid: 0,
            track_type: mkv::TrackType::Subtitle,
            codec: mkv::TrackCodec::Ass(header),
            settings: mkv::TrackSettings::Subtitle,
            name: Some(subtitle.metadata.title.clone()),
            language: Some(language(&subtitle.metadata.locale)),
            default: subtitle.metadata.default,
            forced: subtitle.metadata.forced,
            hearing_impaired: subtitle.metadata.is_caption,
            visual_impaired: false,
            original: false,
            commentary: false,
        });
        streams.push(PacketStream::new(Box::new(packets.into_iter().map(Ok))));
    }
    Ok((
        tracks,
        PacketStreams {
            streams,
            cancellation: cancellation.clone(),
            cancellation_reported: false,
        },
    ))
}

type FalliblePackets = Box<dyn Iterator<Item = Result<mkv::Packet, mkv::Error>>>;

struct PacketStream {
    packets: FalliblePackets,
    head: Option<Result<mkv::Packet, mkv::Error>>,
}

impl PacketStream {
    fn new(mut packets: FalliblePackets) -> Self {
        let head = packets.next();
        Self { packets, head }
    }
}

pub(crate) struct PacketStreams {
    streams: Vec<PacketStream>,
    cancellation: CancellationToken,
    cancellation_reported: bool,
}

impl Iterator for PacketStreams {
    type Item = Result<mkv::Packet, mkv::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cancellation.is_cancelled() && !self.cancellation_reported {
            self.cancellation_reported = true;
            return Some(Err(mkv::Error::Cancelled));
        }
        let index = self
            .streams
            .iter()
            .position(|stream| stream.head.as_ref().is_some_and(Result::is_err))
            .or_else(|| {
                self.streams
                    .iter()
                    .enumerate()
                    .filter_map(|(index, stream)| {
                        stream
                            .head
                            .as_ref()
                            .and_then(|packet| packet.as_ref().ok())
                            .map(|packet| (index, packet.decode_time_ms, packet.track_number))
                    })
                    .min_by_key(|(_, decode_time_ms, track_number)| {
                        (*decode_time_ms, *track_number)
                    })
                    .map(|(index, _, _)| index)
            })?;
        let result = self.streams[index].head.take();
        self.streams[index].head = self.streams[index].packets.next();
        result
    }
}

fn timestamp_millis(timestamp: media::Timestamp) -> Result<i64, media::Error> {
    let numerator = i128::from(timestamp.ticks())
        .checked_mul(1_000)
        .ok_or(media::Error::Overflow("timestamp milliseconds"))?;
    i64::try_from(numerator / i128::from(timestamp.time_base().ticks_per_second()))
        .map_err(|_| media::Error::Overflow("timestamp milliseconds"))
}

pub(crate) fn language(locale: &Locale) -> mkv::Language {
    let ietf = locale.to_string();
    let primary = ietf.split('-').next().unwrap_or("und");
    let legacy = match primary {
        "ar" => "ara",
        "ca" => "cat",
        "de" => "ger",
        "en" => "eng",
        "es" => "spa",
        "fr" => "fre",
        "hi" => "hin",
        "id" => "ind",
        "it" => "ita",
        "ja" => "jpn",
        "ko" => "kor",
        "ms" => "may",
        "pl" => "pol",
        "pt" => "por",
        "ru" => "rus",
        "ta" => "tam",
        "te" => "tel",
        "th" => "tha",
        "tr" => "tur",
        "vi" => "vie",
        "zh" => "chi",
        _ => "und",
    };
    mkv::Language {
        legacy: legacy.into(),
        ietf,
    }
}
