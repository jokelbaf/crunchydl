//! License response parsing and independent per-track key acquisition.

use drm::{
    BoxFuture, ContentKey, ContentType, DrmProvider, DrmRequest, Error, KeyId, KeySet,
    LicenseRequest, LicenseResponse, LicenseTransport, acquire_track_keys,
};

struct EchoTransport;

impl LicenseTransport for EchoTransport {
    fn send<'a>(
        &'a self,
        request: LicenseRequest,
    ) -> BoxFuture<'a, Result<LicenseResponse, Error>> {
        Box::pin(async move { Ok(LicenseResponse::binary(request.challenge)) })
    }
}

struct FixtureProvider;

impl DrmProvider for FixtureProvider {
    fn acquire_keys<'a>(
        &'a self,
        request: DrmRequest,
        transport: &'a dyn LicenseTransport,
    ) -> BoxFuture<'a, Result<KeySet, Error>> {
        Box::pin(async move {
            let response = transport
                .send(LicenseRequest {
                    endpoint: request.endpoint,
                    content_id: request.content_id,
                    playback_token: request.playback_token,
                    content_type: request.content_type,
                    challenge: request.pssh,
                })
                .await?;
            let marker = *response.as_bytes().first().ok_or(Error::License)?;
            let mut keys = KeySet::new();
            keys.insert(
                KeyId::new([marker; 16]),
                ContentKey::try_from_slice(&[marker.wrapping_add(1); 16])?,
            );
            Ok(keys)
        })
    }
}

#[test]
fn unwraps_json_license_response() {
    let response = LicenseResponse::from_http(
        Some("application/json; charset=utf-8"),
        br#"{"license":"AQIDBA=="}"#.to_vec(),
    )
    .unwrap();
    assert_eq!(response.as_bytes(), &[1, 2, 3, 4]);
}

#[test]
fn preserves_binary_license_response() {
    let response =
        LicenseResponse::from_http(Some("application/octet-stream"), vec![0, 255, 17]).unwrap();
    assert_eq!(response.as_bytes(), &[0, 255, 17]);
}

#[tokio::test]
async fn independently_licenses_audio_and_video() {
    let request = |marker, content_type| DrmRequest {
        endpoint: "https://license.invalid".to_string(),
        content_id: "content".to_string(),
        playback_token: format!("token-{marker}"),
        content_type,
        pssh: vec![marker],
    };
    let keys = acquire_track_keys(
        &FixtureProvider,
        &EchoTransport,
        vec![
            (KeyId::new([3; 16]), request(3, ContentType::Video)),
            (KeyId::new([7; 16]), request(7, ContentType::Audio)),
        ],
    )
    .await
    .unwrap();

    assert_eq!(keys[0].expose(), &[4; 16]);
    assert_eq!(keys[1].expose(), &[8; 16]);
}
