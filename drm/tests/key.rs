//! Content-key set lookup by key identifier.

use drm::{ContentKey, Error, KeyId, KeySet};

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
