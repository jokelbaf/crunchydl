use crate::error::{Error, Result};

pub(crate) fn id_bytes(id: u32) -> Result<Vec<u8>> {
    let width = match id {
        0x80..=0xff => 1,
        0x4000..=0x7fff => 2,
        0x20_0000..=0x3f_ffff => 3,
        0x1000_0000..=0x1fff_ffff => 4,
        _ => return Err(Error::Invalid("invalid EBML element id")),
    };
    Ok(id.to_be_bytes()[4 - width..].to_vec())
}

pub(crate) fn size_bytes(value: u64, width: Option<usize>) -> Result<Vec<u8>> {
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

#[cfg(test)]
pub(crate) fn signed(value: i64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let mut first = 0;
    while first < 7 {
        let redundant_zero = bytes[first] == 0 && bytes[first + 1] & 0x80 == 0;
        let redundant_one = bytes[first] == 0xff && bytes[first + 1] & 0x80 != 0;
        if !(redundant_zero || redundant_one) {
            break;
        }
        first += 1;
    }
    bytes[first..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_boundaries_use_minimal_width() {
        assert_eq!(size_bytes(0, None).unwrap(), [0x80]);
        assert_eq!(size_bytes(126, None).unwrap(), [0xfe]);
        assert_eq!(size_bytes(127, None).unwrap(), [0x40, 0x7f]);
        assert_eq!(size_bytes(16_382, None).unwrap(), [0x7f, 0xfe]);
        assert_eq!(size_bytes(16_383, None).unwrap(), [0x20, 0x3f, 0xff]);
        assert!(size_bytes((1_u64 << 56) - 1, None).is_err());
    }

    #[test]
    fn ids_and_signed_values_are_canonical() {
        assert_eq!(id_bytes(0x1a45_dfa3).unwrap(), [0x1a, 0x45, 0xdf, 0xa3]);
        assert_eq!(signed(0), [0]);
        assert_eq!(signed(127), [0x7f]);
        assert_eq!(signed(128), [0, 0x80]);
        assert_eq!(signed(-129), [0xff, 0x7f]);
    }
}
