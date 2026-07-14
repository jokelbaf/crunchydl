//! Catalog item conversion, artwork selection, and rating normalization.

use crunchyroll_rs::media::EpisodeRating;
use crunchyroll_rs::{Episode, Series};

use crunchydl::{CatalogImageKind, CatalogItem, CatalogRating, MediaTarget};

#[test]
fn episode_catalog_item_keeps_artwork_and_download_target() {
    let mut episode: Episode = serde_json::from_str(
        r#"{
                "id":"EP1","title":"One","description":"Description",
                "duration_ms":120000,"audio_locale":"ja-JP",
                "subtitle_locales":["en-US"],"is_subbed":true,
                "images":{"thumbnail":[[{"source":"https://img.test/one.jpg","width":640,"height":360}]]}
            }"#,
    )
    .unwrap();
    episode.versions.clear();
    let item = CatalogItem::from_episode(&episode);
    assert_eq!(item.target, Some(MediaTarget::Episode("EP1".into())));
    assert_eq!(item.duration_millis, Some(120_000));
    assert_eq!(item.images[0].kind, CatalogImageKind::Thumbnail);
}

#[test]
fn series_catalog_item_keeps_hierarchy_and_poster_shapes() {
    let series: Series = serde_json::from_str(
        r#"{
                "id":"S1","title":"Series","description":"Short",
                "extended_description":"Long","series_launch_year":2024,
                "episode_count":12,"season_count":1,"is_subbed":true,
                "audio_locales":["ja-JP"],"subtitle_locales":["en-US"],
                "images":{"poster_tall":[{"source":"https://img.test/tall.jpg","width":400,"height":600}],"poster_wide":[{"source":"https://img.test/wide.jpg","width":800,"height":450}]}
            }"#,
    )
    .unwrap();
    let item = CatalogItem::from_series(&series);
    assert_eq!(item.season_count, Some(1));
    assert_eq!(item.episode_count, Some(12));
    assert_eq!(item.images[0].kind, CatalogImageKind::PosterTall);
    assert_eq!(item.images[1].kind, CatalogImageKind::PosterWide);
}

#[test]
fn episode_vote_rating_becomes_an_approval_percentage() {
    let rating: EpisodeRating = serde_json::from_str(
        r#"{"up":{"displayed":"1.2","unit":"K"},"down":{"displayed":"50","unit":""},"total":1250}"#,
    )
    .unwrap();
    assert_eq!(
        CatalogRating::from_episode_rating(&rating),
        Some(CatalogRating::Approval {
            percentage: 96.0,
            total: Some(1250),
        })
    );
}
