//! Resumable transfer engine: retries, resume, refresh, and staging integrity.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crunchydl::{
    CancellationToken, Error, RepresentationRefresher, RepresentationTransferPlan, SegmentRequest,
    TransferEngine, TransferOptions,
};

mod support;
use support::{FixtureServer, request, server_plan, staging_directory};

#[test]
fn debug_never_contains_signed_url() {
    let debug = format!("{:?}", request("segment"));
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("example.invalid"));
}

#[tokio::test]
async fn retries_rate_limit_and_server_error_then_preserves_order() {
    let server = FixtureServer::start();
    let staging = staging_directory();
    let engine = TransferEngine::new(TransferOptions {
        base_retry_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(5),
        ..TransferOptions::default()
    })
    .expect("engine");
    let result = engine
        .transfer(
            server_plan(&server, &["/rate", "/unstable"]),
            &staging,
            &CancellationToken::new(),
        )
        .await
        .expect("transfer succeeds");
    assert_eq!(std::fs::read(&result.init).expect("init"), b"init");
    assert_eq!(std::fs::read(&result.segments[0]).expect("rate"), b"rate");
    assert_eq!(
        std::fs::read(&result.segments[1]).expect("unstable"),
        b"unstable"
    );
    assert_eq!(server.count("/rate"), 2);
    assert_eq!(server.count("/unstable"), 2);
    std::fs::remove_dir_all(staging).expect("cleanup");
}

#[tokio::test]
async fn interrupted_transfer_resumes_without_redownloading_completed_segments() {
    let server = FixtureServer::start();
    let staging = staging_directory();
    let first = TransferEngine::new(TransferOptions {
        concurrency: 1,
        max_attempts: 1,
        ..TransferOptions::default()
    })
    .expect("engine");
    let plan = server_plan(&server, &["/fresh", "/interrupt"]);
    assert!(
        first
            .transfer(plan.clone(), &staging, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(server.count("/fresh"), 1);

    let second = TransferEngine::new(TransferOptions {
        base_retry_delay: Duration::from_millis(1),
        max_retry_delay: Duration::from_millis(5),
        ..TransferOptions::default()
    })
    .expect("engine");
    let result = second
        .transfer(plan, &staging, &CancellationToken::new())
        .await
        .expect("resume succeeds");
    assert_eq!(server.count("/fresh"), 1);
    assert_eq!(server.count("/interrupt"), 2);
    assert_eq!(result.segments.len(), 2);
    std::fs::remove_dir_all(staging).expect("cleanup");
}

struct RefreshToFresh {
    fresh: RepresentationTransferPlan,
}

impl RepresentationRefresher for RefreshToFresh {
    fn refresh<'a>(
        &'a self,
        _expired: &'a RepresentationTransferPlan,
    ) -> Pin<Box<dyn Future<Output = Result<RepresentationTransferPlan, Error>> + Send + 'a>> {
        Box::pin(async { Ok(self.fresh.clone()) })
    }
}

#[tokio::test]
async fn expired_url_refreshes_only_an_exact_representation_match() {
    let server = FixtureServer::start();
    let staging = staging_directory();
    let expired = server_plan(&server, &["/expired"]);
    let mut fresh = expired.clone();

    fresh.segments = vec![SegmentRequest::new(
        format!("{}/fresh?token=two", server.base),
        None,
        "expired",
        None,
    )];

    let engine = TransferEngine::new(TransferOptions::default()).expect("engine");
    let result = engine
        .transfer_with_refresh(
            expired,
            &staging,
            &CancellationToken::new(),
            &RefreshToFresh { fresh },
        )
        .await
        .expect("refresh succeeds");
    assert_eq!(std::fs::read(&result.segments[0]).expect("fresh"), b"fresh");
    assert_eq!(server.count("/expired"), 1);
    assert_eq!(server.count("/fresh"), 1);
    std::fs::remove_dir_all(staging).expect("cleanup");
}

#[tokio::test]
async fn mismatched_staging_is_rejected_without_network_reuse() {
    let server = FixtureServer::start();
    let staging = staging_directory();
    let engine = TransferEngine::new(TransferOptions::default()).expect("engine");
    let original = server_plan(&server, &["/rate"]);
    engine
        .transfer(original.clone(), &staging, &CancellationToken::new())
        .await
        .expect("initial transfer");
    let before = server.count("/rate");
    let mut changed = original;
    changed.plan_fingerprint = "different-plan".to_string();
    assert!(matches!(
        engine
            .transfer(changed, &staging, &CancellationToken::new())
            .await,
        Err(Error::ResumeMismatch(_))
    ));
    assert_eq!(server.count("/rate"), before);
    std::fs::remove_dir_all(staging).expect("cleanup");
}
