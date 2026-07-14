//! Download archive ignores stale entries whose output no longer exists.

use crunchydl::{Archive, ArchiveEntry, ArchiveKey, JsonArchive};

#[test]
fn stale_entries_do_not_suppress_downloads() {
    let root = std::env::temp_dir().join(format!("crunchydl-archive-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let archive = JsonArchive::new(root.join("archive.json"));
    let key = ArchiveKey {
        media_id: "M1".into(),
        selection_fingerprint: "abc".into(),
    };
    archive
        .record(&ArchiveEntry {
            key: key.clone(),
            output: root.join("missing.mkv"),
            tracks: Vec::new(),
        })
        .unwrap();
    assert!(archive.find(&key).unwrap().is_none());
    std::fs::remove_dir_all(root).unwrap();
}
