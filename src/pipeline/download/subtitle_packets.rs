//! ASS subtitle packet conversion.

use super::*;

pub(crate) fn ass_packets(
    ass: &str,
    track_number: u64,
) -> Result<(String, Vec<mkv::Packet>), Error> {
    let normalized = ass.replace("\r\n", "\n").replace('\r', "\n");
    let mut header = Vec::new();
    let mut packets = Vec::new();
    let mut read_order = 0_u64;
    for line in normalized.lines() {
        let Some(dialogue) = line.strip_prefix("Dialogue:") else {
            header.push(line);
            continue;
        };
        let fields = dialogue.trim_start().splitn(10, ',').collect::<Vec<_>>();
        if fields.len() != 10 {
            return Err(Error::Subtitle("malformed ASS dialogue".into()));
        }
        let start = ass_time(fields[1])?;
        let end = ass_time(fields[2])?;
        if end < start {
            return Err(Error::Subtitle("ASS dialogue ends before it starts".into()));
        }
        let payload = format!(
            "{read_order},{},{},{},{},{},{},{},{}",
            fields[0], fields[3], fields[4], fields[5], fields[6], fields[7], fields[8], fields[9]
        );
        packets.push(mkv::Packet {
            track_number,
            decode_time_ms: i64::try_from(start.as_millis())
                .map_err(|_| Error::Subtitle("subtitle timestamp overflow".into()))?,
            presentation_time_ms: i64::try_from(start.as_millis())
                .map_err(|_| Error::Subtitle("subtitle timestamp overflow".into()))?,
            duration: end - start,
            keyframe: true,
            data: payload.into_bytes(),
        });
        read_order += 1;
    }
    packets.sort_by_key(|packet| packet.decode_time_ms);
    Ok((header.join("\r\n") + "\r\n", packets))
}

fn ass_time(value: &str) -> Result<Duration, Error> {
    let (hours, rest) = value
        .trim()
        .split_once(':')
        .ok_or_else(|| Error::Subtitle("invalid ASS timestamp".into()))?;
    let (minutes, seconds) = rest
        .split_once(':')
        .ok_or_else(|| Error::Subtitle("invalid ASS timestamp".into()))?;
    let hours: u64 = hours
        .parse()
        .map_err(|_| Error::Subtitle("invalid ASS timestamp".into()))?;
    let minutes: u64 = minutes
        .parse()
        .map_err(|_| Error::Subtitle("invalid ASS timestamp".into()))?;
    let seconds: f64 = seconds
        .parse()
        .map_err(|_| Error::Subtitle("invalid ASS timestamp".into()))?;
    if minutes >= 60 || !seconds.is_finite() || !(0.0..60.0).contains(&seconds) {
        return Err(Error::Subtitle("invalid ASS timestamp".into()));
    }
    Ok(Duration::from_millis(
        hours
            .saturating_mul(3_600_000)
            .saturating_add(minutes * 60_000)
            .saturating_add((seconds * 1000.0).round() as u64),
    ))
}
