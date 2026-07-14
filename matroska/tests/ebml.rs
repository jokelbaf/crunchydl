//! EBML variable-integer, element-id, and signed-integer encoding boundaries.

use matroska::ebml::{id_bytes, size_bytes};

/// Minimal-width signed EBML integer, mirroring the writer's block timecodes.
fn signed(value: i64) -> Vec<u8> {
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
