//! CENC/CENS/CBC1/CBCS decryption conformance against AES test vectors.

use drm::{CencDecrypter, ContentKey, Error};

const KEY: [u8; 16] = [
    0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f, 0x3c,
];
const IV: [u8; 16] = [
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];
const CBC_IV: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];
const PLAIN: [u8; 16] = [
    0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17, 0x2a,
];
const CTR_CIPHER: [u8; 16] = [
    0x87, 0x4d, 0x61, 0x91, 0xb6, 0x20, 0xe3, 0x26, 0x1b, 0xef, 0x68, 0x64, 0x99, 0x0d, 0xb6, 0xce,
];
const CBC_CIPHER: [u8; 16] = [
    0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9, 0x19, 0x7d,
];

#[test]
fn decrypts_all_four_schemes_and_neutralizes_signaling() {
    for (scheme, cipher, iv) in [
        (*b"cenc", CTR_CIPHER, IV),
        (*b"cens", CTR_CIPHER, IV),
        (*b"cbc1", CBC_CIPHER, CBC_IV),
        (*b"cbcs", CBC_CIPHER, CBC_IV),
    ] {
        let init = init(scheme, false);
        let decrypter =
            CencDecrypter::new(&init, &ContentKey::try_from_slice(&KEY).unwrap()).unwrap();
        let output = decrypter
            .decrypt_fragment(fragment(&cipher, false, iv))
            .unwrap();
        assert!(output.ends_with(&PLAIN), "scheme {scheme:?}");
        assert!(!output.windows(4).any(|value| value == b"senc"));

        let mut assembled = Vec::new();
        decrypter
            .assemble(&init, [(0, fragment(&cipher, false, iv))], &mut assembled)
            .unwrap();
        assert!(assembled.windows(4).any(|value| value == b"avc1"));
        assert!(!assembled.windows(4).any(|value| value == b"encv"));
    }
}

#[test]
fn decrypts_ctr_subsample_layout() {
    let init = init(*b"cenc", false);
    let decrypter = CencDecrypter::new(&init, &ContentKey::try_from_slice(&KEY).unwrap()).unwrap();
    let mut cipher = vec![1, 2, 3, 4, 5];
    cipher.extend_from_slice(&CTR_CIPHER);
    let output = decrypter
        .decrypt_fragment(fragment(&cipher, true, IV))
        .unwrap();
    let mut expected = vec![1, 2, 3, 4, 5];
    expected.extend_from_slice(&PLAIN);
    assert!(output.ends_with(&expected));
}

#[test]
fn rejects_unsupported_or_multi_track_layouts() {
    let bad_init = init(*b"xxxx", false);
    assert!(matches!(
        CencDecrypter::new(&bad_init, &ContentKey::try_from_slice(&KEY).unwrap()),
        Err(Error::UnsupportedLayout(_))
    ));

    let init = init(*b"cenc", false);
    let decrypter = CencDecrypter::new(&init, &ContentKey::try_from_slice(&KEY).unwrap()).unwrap();
    let mut duplicated = fragment(&CTR_CIPHER, false, IV);
    let second = mp4_box(*b"moof", mp4_box(*b"traf", Vec::new()));
    duplicated.splice(0..0, second);
    assert!(matches!(
        decrypter.decrypt_fragment(duplicated),
        Err(Error::UnsupportedLayout(_))
    ));
}

fn init(scheme: [u8; 4], audio: bool) -> Vec<u8> {
    let mut schm = vec![0, 0, 0, 0];
    schm.extend_from_slice(&scheme);
    schm.extend_from_slice(&0x0001_0000_u32.to_be_bytes());
    let schm = mp4_box(*b"schm", schm);

    let version = u8::from(matches!(&scheme, b"cens" | b"cbcs"));
    let mut tenc = vec![version, 0, 0, 0, 0];
    tenc.push(0);
    tenc.push(1);
    tenc.push(16);
    tenc.extend_from_slice(&[3; 16]);
    let tenc = mp4_box(*b"tenc", tenc);
    let sinf = mp4_box(*b"sinf", [schm, mp4_box(*b"schi", tenc)].concat());
    let (kind, prefix) = if audio {
        (*b"enca", vec![0; 28])
    } else {
        (*b"encv", vec![0; 78])
    };
    let entry = mp4_box(kind, [prefix, sinf].concat());
    let mut stsd = vec![0, 0, 0, 0];
    stsd.extend_from_slice(&1_u32.to_be_bytes());
    stsd.extend_from_slice(&entry);
    mp4_box(
        *b"moov",
        mp4_box(
            *b"trak",
            mp4_box(
                *b"mdia",
                mp4_box(*b"minf", mp4_box(*b"stbl", mp4_box(*b"stsd", stsd))),
            ),
        ),
    )
}

fn fragment(ciphertext: &[u8], subsample: bool, iv: [u8; 16]) -> Vec<u8> {
    let mut senc = vec![0, 0, 0, if subsample { 2 } else { 0 }];
    senc.extend_from_slice(&1_u32.to_be_bytes());
    senc.extend_from_slice(&iv);
    if subsample {
        senc.extend_from_slice(&1_u16.to_be_bytes());
        senc.extend_from_slice(&5_u16.to_be_bytes());
        senc.extend_from_slice(&16_u32.to_be_bytes());
    }
    let senc = mp4_box(*b"senc", senc);

    let mut trun = vec![0, 0, 2, 1];
    trun.extend_from_slice(&1_u32.to_be_bytes());
    let data_offset_position = trun.len();
    trun.extend_from_slice(&0_u32.to_be_bytes());
    trun.extend_from_slice(&(ciphertext.len() as u32).to_be_bytes());
    let trun = mp4_box(*b"trun", trun);
    let traf = mp4_box(*b"traf", [senc, trun].concat());
    let mut moof = mp4_box(*b"moof", traf);
    let data_offset = u32::try_from(moof.len() + 8).unwrap();
    let trun_type = moof.windows(4).position(|value| value == b"trun").unwrap();
    let field = trun_type + 4 + data_offset_position;
    moof[field..field + 4].copy_from_slice(&data_offset.to_be_bytes());
    [moof, mp4_box(*b"mdat", ciphertext.to_vec())].concat()
}

fn mp4_box(name: [u8; 4], payload: Vec<u8>) -> Vec<u8> {
    let size = u32::try_from(payload.len() + 8).unwrap();
    [size.to_be_bytes().as_slice(), &name, &payload].concat()
}
