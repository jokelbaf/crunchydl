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

mod boxes;
use boxes::{count_box, find_box_payload, validate_boxes, walk_boxes_mut};
