//! Block-group encoding, including subtitle display duration.

use std::time::Duration;

use matroska::block::group;

#[test]
fn subtitle_block_group_carries_its_display_duration() {
    assert_eq!(
        group(1, 0, Duration::from_millis(2_500), b"hi").unwrap(),
        [
            0xa0, 0x8c, 0xa1, 0x86, 0x81, 0x00, 0x00, 0x00, b'h', b'i', 0x9b, 0x82, 0x09, 0xc4,
        ]
    );
}
