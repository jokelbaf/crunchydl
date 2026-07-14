use crate::error::{Error, Result};

/// Encode an EBML element id as its canonical big-endian bytes.
///
/// # Errors
///
/// Returns [`Error::Invalid`] when `id` is not a valid EBML element id.
pub fn id_bytes(id: u32) -> Result<Vec<u8>> {
    let width = match id {
        0x80..=0xff => 1,
        0x4000..=0x7fff => 2,
        0x20_0000..=0x3f_ffff => 3,
        0x1000_0000..=0x1fff_ffff => 4,
        _ => return Err(Error::Invalid("invalid EBML element id")),
    };
    Ok(id.to_be_bytes()[4 - width..].to_vec())
}

/// Encode an EBML size using the minimal width when `width` is `None`.
///
/// # Errors
///
/// Returns [`Error::Invalid`] for an out-of-range width and [`Error::Overflow`]
/// when `value` cannot be represented.
pub fn size_bytes(value: u64, width: Option<usize>) -> Result<Vec<u8>> {
    let width = match width {
        Some(width @ 1..=8) => width,
        Some(_) => return Err(Error::Invalid("invalid EBML size width")),
        None => (1..=8)
            .find(|width| value < (1_u64 << (7 * width)) - 1)
            .ok_or(Error::Overflow("EBML size"))?,
    };
    let maximum = (1_u64 << (7 * width)) - 1;
    if value >= maximum {
        return Err(Error::Overflow("EBML size"));
    }
    let encoded = value | (1_u64 << (7 * width));
    Ok(encoded.to_be_bytes()[8 - width..].to_vec())
}

pub(crate) fn vint(value: u64) -> Result<Vec<u8>> {
    size_bytes(value, None)
}

pub(crate) fn unsigned(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|byte| *byte != 0).unwrap_or(7);
    bytes[first..].to_vec()
}
