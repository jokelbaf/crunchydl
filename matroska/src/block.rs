use std::time::Duration;

use crate::ebml::vint;
use crate::error::{Error, Result};

pub(crate) fn simple(track: u64, relative_ms: i16, keyframe: bool, data: &[u8]) -> Result<Vec<u8>> {
    crate::element::binary(
        0xa3,
        &payload(track, relative_ms, u8::from(keyframe) << 7, data)?,
    )
}

pub(crate) fn group(
    track: u64,
    relative_ms: i16,
    duration: Duration,
    data: &[u8],
) -> Result<Vec<u8>> {
    let duration = u64::try_from(duration.as_millis())
        .map_err(|_| Error::Overflow("block duration milliseconds"))?;
    crate::element::master(
        0xa0,
        [
            crate::element::binary(0xa1, &payload(track, relative_ms, 0, data)?)?,
            crate::element::uint(0x9b, duration)?,
        ],
    )
}

fn payload(track: u64, relative_ms: i16, flags: u8, data: &[u8]) -> Result<Vec<u8>> {
    if track == 0 {
        return Err(Error::Invalid("block track number is zero"));
    }
    let mut payload = vint(track)?;
    payload.extend(relative_ms.to_be_bytes());
    payload.push(flags);
    payload.extend(data);
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitle_block_group_carries_its_display_duration() {
        assert_eq!(
            group(1, 0, Duration::from_millis(2_500), b"hi").unwrap(),
            [
                0xa0, 0x8c, 0xa1, 0x86, 0x81, 0x00, 0x00, 0x00, b'h', b'i', 0x9b, 0x82, 0x09, 0xc4,
            ]
        );
    }
}
