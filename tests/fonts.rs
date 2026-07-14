//! Font resolution: only referenced families, magic validation, missing policy.

use std::fs;
use std::sync::Mutex;

use crunchyroll_rs::Locale;

use crunchydl::{
    DirectoryFontResolver, DownloadWarning, Error, FontPolicy, FontResolver, ResolvedFont,
    SubtitleMetadata, SubtitleTrack, resolve_referenced_fonts,
};

struct RecordingResolver {
    requested: Mutex<Vec<String>>,
}

impl FontResolver for RecordingResolver {
    fn resolve(&self, family: &str) -> Result<Option<ResolvedFont>, Error> {
        self.requested
            .lock()
            .expect("lock")
            .push(family.to_string());
        if family.eq_ignore_ascii_case("Arial") {
            Ok(Some(ResolvedFont {
                family: family.to_string(),
                filename: "arial.ttf".to_string(),
                mime_type: "application/x-truetype-font".to_string(),
                data: b"\0\x01\0\0fixture".to_vec(),
            }))
        } else {
            Ok(None)
        }
    }
}

fn track(fonts: &[&str]) -> SubtitleTrack {
    SubtitleTrack {
        metadata: SubtitleMetadata {
            locale: Locale::en_US,
            title: "English".to_string(),
            is_caption: false,
            is_signs: false,
            default: false,
            forced: false,
        },
        ass: String::new(),
        referenced_fonts: fonts.iter().map(|font| (*font).to_string()).collect(),
    }
}

#[test]
fn resolves_only_referenced_fonts_and_applies_missing_policy() {
    let resolver = RecordingResolver {
        requested: Mutex::new(Vec::new()),
    };
    let (fonts, warnings) = resolve_referenced_fonts(
        &[track(&["Arial", "ARIAL", "Missing Font"])],
        &resolver,
        FontPolicy::Warn,
    )
    .expect("resolve");
    assert_eq!(fonts.len(), 1);
    assert_eq!(
        warnings,
        [DownloadWarning::MissingFont {
            family: "Missing Font".to_string()
        }]
    );
    assert_eq!(
        resolver.requested.lock().expect("lock").as_slice(),
        ["Arial", "Missing Font"]
    );

    let error = resolve_referenced_fonts(&[track(&["Missing Font"])], &resolver, FontPolicy::Error)
        .expect_err("missing is fatal");
    assert!(matches!(error, Error::Font(_)));
}

#[test]
fn directory_resolver_validates_magic_and_aliases() {
    let root = std::env::temp_dir().join(format!("crunchydl-font-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create fixture directory");
    fs::write(root.join("trebuc.ttf"), b"\0\x01\0\0fixture-font").expect("write fixture");
    let resolver =
        DirectoryFontResolver::new([root.clone()]).with_alias("Trebuchet MS", "trebuc.ttf");
    let font = resolver
        .resolve("Trebuchet MS")
        .expect("valid")
        .expect("found");
    assert_eq!(font.filename, "trebuc.ttf");
    assert_eq!(font.mime_type, "application/x-truetype-font");
    fs::write(root.join("bad.otf"), b"not-a-font!!").expect("write invalid fixture");
    let resolver = DirectoryFontResolver::new([root.clone()]).with_alias("Bad", "bad.otf");
    assert!(matches!(resolver.resolve("Bad"), Err(Error::Font(_))));
    fs::remove_dir_all(root).expect("remove fixture directory");
}
