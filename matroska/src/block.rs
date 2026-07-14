use std::time::Duration;

use crate::ebml::vint;
use crate::error::{Error, Result};

pub(crate) fn simple(track: u64, relative_ms: i16, keyframe: bool, data: &[u8]) -> Result<Vec<u8>> {
    crate::element::binary(
        0xa3,
        &payload(track, relative_ms, u8::from(keyframe) << 7, data)?,
    )
}

/// Encode a `BlockGroup` carrying a block and its display duration.
///
/// # Errors
///
/// Returns [`Error::Overflow`] when the duration exceeds the container limit and
/// [`Error::Invalid`] when the track number is zero.
pub fn group(track: u64, relative_ms: i16, duration: Duration, data: &[u8]) -> Result<Vec<u8>> {
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
