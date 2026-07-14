//! Zeroizing content-key storage and lookup.

use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::Error;

/// A 16-byte Common Encryption key identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Zeroize)]
pub struct KeyId([u8; 16]);

impl KeyId {
    /// Construct a key identifier from its bytes.
    #[must_use]
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Borrow the key identifier bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    /// Return a lowercase hexadecimal identifier safe for diagnostics.
    #[must_use]
    pub fn to_hex(self) -> String {
        hex(&self.0)
    }
}

/// A zeroizing 16-byte content key.
///
/// This type deliberately implements neither `Debug` nor `Display`.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct ContentKey([u8; 16]);

impl ContentKey {
    /// Copy a key from a byte slice, rejecting non-128-bit keys.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidKeyLength`] unless `bytes` contains 16 bytes.
    pub fn try_from_slice(bytes: &[u8]) -> Result<Self, Error> {
        Ok(Self(
            bytes
                .try_into()
                .map_err(|_| Error::InvalidKeyLength(bytes.len()))?,
        ))
    }

    /// Borrow the key bytes for immediate cryptographic use.
    #[must_use]
    pub const fn expose(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Content keys indexed by key identifier.
///
/// The collection and every contained key are zeroized when dropped. It does
/// not implement `Debug`, `Display`, or serialization traits.
#[derive(Default, Zeroize, ZeroizeOnDrop)]
pub struct KeySet(Vec<(KeyId, ContentKey)>);

impl KeySet {
    /// Create an empty key set.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Insert a content key, replacing a previous value for the same KID.
    pub fn insert(&mut self, kid: KeyId, key: ContentKey) {
        if let Some((_, existing)) = self.0.iter_mut().find(|(candidate, _)| *candidate == kid) {
            *existing = key;
        } else {
            self.0.push((kid, key));
        }
    }

    /// Return the content key matching `kid`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingKey`] when the license omitted the requested KID.
    pub fn require(&self, kid: KeyId) -> Result<&ContentKey, Error> {
        self.0
            .iter()
            .find_map(|(candidate, key)| (*candidate == kid).then_some(key))
            .ok_or_else(|| Error::MissingKey(kid.to_hex()))
    }

    /// Number of content keys in the set.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no content keys were returned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_keys_by_kid() {
        let kid = KeyId::new([7; 16]);
        let mut keys = KeySet::new();
        keys.insert(kid, ContentKey::try_from_slice(&[9; 16]).unwrap());

        assert_eq!(keys.require(kid).unwrap().expose(), &[9; 16]);
        assert!(matches!(
            keys.require(KeyId::new([8; 16])),
            Err(Error::MissingKey(_))
        ));
    }
}
