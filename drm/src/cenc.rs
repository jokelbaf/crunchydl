//! Bounded CENC/CENS/CBC1/CBCS fragment decryption.

use std::io::Write;

use vsd_mp4::boxes::TencBox;
use vsd_mp4::decrypt::CencDecrypter as VsdDecrypter;

use crate::{ContentKey, Error, KeyId, key::hex};

/// A supported ISO Common Encryption scheme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncryptionScheme {
    /// AES-CTR full-sample encryption.
    Cenc,
    /// AES-CTR pattern encryption.
    Cens,
    /// AES-CBC full-sample encryption.
    Cbc1,
    /// AES-CBC pattern encryption.
    Cbcs,
}

/// Encryption defaults parsed from an initialization segment before licensing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncryptionInfo {
    /// Default content-key identifier declared by `tenc`.
    pub default_kid: KeyId,
    /// Common Encryption scheme declared by `schm`.
    pub scheme: EncryptionScheme,
}

/// Inspect an encrypted initialization segment without requiring a content key.
///
/// # Errors
///
/// Returns a typed layout error when required `tenc` or `schm` metadata is
/// absent, malformed, or unsupported.
pub fn inspect_encryption(init: &[u8]) -> Result<EncryptionInfo, Error> {
    validate_boxes(init, 0)?;
    let tenc = TencBox::from_init(init)
        .map_err(|_| Error::UnsupportedLayout("malformed tenc box".to_string()))?
        .ok_or_else(|| Error::UnsupportedLayout("missing tenc box".to_string()))?;
    if !tenc.is_protected {
        return Err(Error::UnsupportedLayout(
            "tenc does not mark the track protected".to_string(),
        ));
    }
    let scheme_bytes = find_box_payload(init, b"schm", 0)?
        .ok_or_else(|| Error::UnsupportedLayout("missing schm box".to_string()))?;
    if scheme_bytes.len() < 12 {
        return Err(Error::UnsupportedLayout("truncated schm box".to_string()));
    }
    Ok(EncryptionInfo {
        default_kid: KeyId::new(tenc.default_kid),
        scheme: EncryptionScheme::parse(&scheme_bytes[4..8])?,
    })
}

impl EncryptionScheme {
    fn parse(value: &[u8]) -> Result<Self, Error> {
        match value {
            b"cenc" => Ok(Self::Cenc),
            b"cens" => Ok(Self::Cens),
            b"cbc1" => Ok(Self::Cbc1),
            b"cbcs" => Ok(Self::Cbcs),
            _ => Err(Error::UnsupportedLayout(format!(
                "unknown protection scheme {}",
                String::from_utf8_lossy(value)
            ))),
        }
    }
}

/// A local replaceable wrapper around `vsd-mp4` decryption.
pub struct CencDecrypter {
    inner: VsdDecrypter,
    default_kid: KeyId,
    scheme: EncryptionScheme,
}

impl CencDecrypter {
    /// Parse encryption defaults from one initialization segment.
    ///
    /// # Errors
    ///
    /// Returns a typed layout error when required `tenc` or `schm` metadata is
    /// absent, malformed, or unsupported.
    pub fn new(init: &[u8], key: &ContentKey) -> Result<Self, Error> {
        let info = inspect_encryption(init)?;
        let inner = VsdDecrypter::with_init(&hex(key.expose()), init)
            .map_err(|_| Error::UnsupportedLayout("unsupported init box layout".to_string()))?;
        Ok(Self {
            inner,
            default_kid: info.default_kid,
            scheme: info.scheme,
        })
    }

    /// Default content-key identifier declared by the init segment.
    #[must_use]
    pub const fn default_kid(&self) -> KeyId {
        self.default_kid
    }

    /// Encryption scheme declared by the init segment.
    #[must_use]
    pub const fn scheme(&self) -> EncryptionScheme {
        self.scheme
    }

    /// Decrypt one single-track `moof`/`mdat` fragment and neutralize auxiliary
    /// encryption signaling.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnsupportedLayout`] for malformed, multi-track, or
    /// incomplete fragments and [`Error::Decrypt`] for cryptographic failure.
    pub fn decrypt_fragment(&self, fragment: Vec<u8>) -> Result<Vec<u8>, Error> {
        validate_fragment(&fragment)?;
        let mut output = self
            .inner
            .decrypt_fragment(fragment, None)
            .map_err(|_| Error::Decrypt)?;
        neutralize_fragment(&mut output)?;
        Ok(output)
    }

    /// Write a clear init segment followed by ordered decrypted fragments.
    ///
    /// Fragment indices must be contiguous and start at zero.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an ordering, MP4 layout, decryption, or output
    /// failure.
    pub fn assemble<W, I>(&self, init: &[u8], fragments: I, writer: &mut W) -> Result<(), Error>
    where
        W: Write,
        I: IntoIterator<Item = (usize, Vec<u8>)>,
    {
        let mut clear_init = init.to_vec();
        neutralize_init(&mut clear_init)?;
        writer.write_all(&clear_init)?;
        for (expected, (index, fragment)) in fragments.into_iter().enumerate() {
            if index != expected {
                return Err(Error::UnsupportedLayout(format!(
                    "fragment index {index} appeared where {expected} was required"
                )));
            }
            writer.write_all(&self.decrypt_fragment(fragment)?)?;
        }
        writer.flush()?;
        Ok(())
    }
}

fn validate_fragment(data: &[u8]) -> Result<(), Error> {
    validate_boxes(data, 0)?;
    let moof = count_box(data, b"moof", 0)?;
    let mdat = count_box(data, b"mdat", 0)?;
    let traf = count_box(data, b"traf", 0)?;
    let trun = count_box(data, b"trun", 0)?;
    let senc = count_box(data, b"senc", 0)?;
    if (moof, mdat, traf, trun, senc) != (1, 1, 1, 1, 1) {
        return Err(Error::UnsupportedLayout(format!(
            "expected one moof/mdat/traf/trun/senc, found {moof}/{mdat}/{traf}/{trun}/{senc}"
        )));
    }
    Ok(())
}

fn neutralize_init(data: &mut [u8]) -> Result<(), Error> {
    walk_boxes_mut(data, 0, &mut |name| match &*name {
        b"encv" => *name = *b"avc1",
        b"enca" => *name = *b"mp4a",
        _ => {}
    })
}

fn neutralize_fragment(data: &mut [u8]) -> Result<(), Error> {
    walk_boxes_mut(data, 0, &mut |name| {
        if matches!(&*name, b"senc" | b"saiz" | b"saio") {
            *name = *b"free";
        }
    })
}

fn validate_boxes(data: &[u8], child_offset: usize) -> Result<(), Error> {
    walk_boxes(data, child_offset, &mut |_, _| Ok(()))
}

fn count_box(data: &[u8], target: &[u8; 4], child_offset: usize) -> Result<usize, Error> {
    let mut count = 0;
    walk_boxes(data, child_offset, &mut |name, _| {
        if name == target {
            count += 1;
        }
        Ok(())
    })?;
    Ok(count)
}

fn find_box_payload<'a>(
    data: &'a [u8],
    target: &[u8; 4],
    child_offset: usize,
) -> Result<Option<&'a [u8]>, Error> {
    let mut found = None;
    walk_boxes(data, child_offset, &mut |name, payload| {
        if name == target && found.is_none() {
            found = Some(payload);
        }
        Ok(())
    })?;
    Ok(found)
}

fn walk_boxes<'a>(
    data: &'a [u8],
    child_offset: usize,
    visit: &mut impl FnMut(&[u8; 4], &'a [u8]) -> Result<(), Error>,
) -> Result<(), Error> {
    if child_offset > data.len() {
        return Err(Error::UnsupportedLayout("invalid child offset".to_string()));
    }
    let mut position = child_offset;
    while position < data.len() {
        let (size, header) = box_size(data, position)?;
        let end = position
            .checked_add(size)
            .ok_or_else(|| Error::UnsupportedLayout("box size overflow".to_string()))?;
        if end > data.len() {
            return Err(Error::UnsupportedLayout(
                "box exceeds parent bounds".to_string(),
            ));
        }
        let name: &[u8; 4] = data[position + 4..position + 8]
            .try_into()
            .expect("box type is four bytes");
        let payload = &data[position + header..end];
        visit(name, payload)?;
        if let Some(offset) = children_offset(name) {
            walk_boxes(payload, offset, visit)?;
        }
        position = end;
    }
    Ok(())
}

fn walk_boxes_mut(
    data: &mut [u8],
    child_offset: usize,
    visit: &mut impl FnMut(&mut [u8; 4]),
) -> Result<(), Error> {
    if child_offset > data.len() {
        return Err(Error::UnsupportedLayout("invalid child offset".to_string()));
    }
    let mut position = child_offset;
    while position < data.len() {
        let (size, header) = box_size(data, position)?;
        let end = position
            .checked_add(size)
            .ok_or_else(|| Error::UnsupportedLayout("box size overflow".to_string()))?;
        if end > data.len() {
            return Err(Error::UnsupportedLayout(
                "box exceeds parent bounds".to_string(),
            ));
        }
        let original: [u8; 4] = data[position + 4..position + 8]
            .try_into()
            .expect("box type is four bytes");
        let offset = children_offset(&original);
        let name: &mut [u8; 4] = (&mut data[position + 4..position + 8])
            .try_into()
            .expect("box type is four bytes");
        visit(name);
        if let Some(offset) = offset {
            walk_boxes_mut(&mut data[position + header..end], offset, visit)?;
        }
        position = end;
    }
    Ok(())
}

fn box_size(data: &[u8], position: usize) -> Result<(usize, usize), Error> {
    if data.len().saturating_sub(position) < 8 {
        return Err(Error::UnsupportedLayout("truncated box header".to_string()));
    }
    let short = u32::from_be_bytes(
        data[position..position + 4]
            .try_into()
            .expect("box size is four bytes"),
    );
    match short {
        0 => Ok((data.len() - position, 8)),
        1 => {
            if data.len().saturating_sub(position) < 16 {
                return Err(Error::UnsupportedLayout(
                    "truncated extended box header".to_string(),
                ));
            }
            let size = u64::from_be_bytes(
                data[position + 8..position + 16]
                    .try_into()
                    .expect("extended box size is eight bytes"),
            );
            let size = usize::try_from(size)
                .map_err(|_| Error::UnsupportedLayout("box size overflow".to_string()))?;
            if size < 16 {
                return Err(Error::UnsupportedLayout(
                    "invalid extended box size".to_string(),
                ));
            }
            Ok((size, 16))
        }
        size if size < 8 => Err(Error::UnsupportedLayout("invalid box size".to_string())),
        size => Ok((size as usize, 8)),
    }
}

fn children_offset(name: &[u8; 4]) -> Option<usize> {
    match name {
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"sinf" | b"schi" | b"moof" | b"traf" => {
            Some(0)
        }
        b"stsd" => Some(8),
        b"encv" => Some(78),
        b"enca" => Some(28),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 16] = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c,
    ];
    const IV: [u8; 16] = [
        0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe,
        0xff,
    ];
    const CBC_IV: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];
    const PLAIN: [u8; 16] = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a,
    ];
    const CTR_CIPHER: [u8; 16] = [
        0x87, 0x4d, 0x61, 0x91, 0xb6, 0x20, 0xe3, 0x26, 0x1b, 0xef, 0x68, 0x64, 0x99, 0x0d, 0xb6,
        0xce,
    ];
    const CBC_CIPHER: [u8; 16] = [
        0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9, 0x19,
        0x7d,
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
        let decrypter =
            CencDecrypter::new(&init, &ContentKey::try_from_slice(&KEY).unwrap()).unwrap();
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
        let decrypter =
            CencDecrypter::new(&init, &ContentKey::try_from_slice(&KEY).unwrap()).unwrap();
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
}
