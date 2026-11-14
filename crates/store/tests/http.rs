//! Validate the wire contract without an installed service or real credentials.
use serde_json::{Value, json};
use store::Store;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};
fn snapshot() -> Value {
    json!({"version":"ticketz-snapshot/1","issues":[],"boards":[]})
}
fn envelope(value: Value) -> Value {
    json!({"exit_code":0,"stdout":value.to_string(),"stderr":"","truncated":false})
}
async fn server(value: Value) -> MockServer {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(&s)
        .await;
    s
}
#[tokio::test]
async fn snapshots_have_no_credential_and_unknown_stays_unknown() {
    let mut v = snapshot();
    v["boards"] = json!([{"version":"ticketz-status/1","repo":"example-org/board","issues":0,"last_sync":"2026-01-01T00:00:00Z","actors":{"example-user":{"permission_unknown":"GitHub answered 404","observed_at":"2026-01-01T00:00:00Z"}}}]);
    let s = server(envelope(v)).await;
    let value = Store::new(&s.uri()).unwrap().snapshot().await.unwrap();
    assert!(value.issues.is_empty());
    assert!(value.boards[0].files.is_none());
    assert!(matches!(
        value.boards[0].access[0].access,
        types::Access::Unknown(_)
    ));
    let requests = s.received_requests().await.unwrap();
    assert_eq!(requests[0].url.path(), "/ticketz/snapshot");
    assert!(!requests[0].headers.contains_key("authorization"));
}
#[tokio::test]
async fn incomplete_or_failed_responses_never_become_empty_snapshots() {
    let mut truncated = envelope(snapshot());
    truncated["truncated"] = json!(true);
    let mut failed = envelope(snapshot());
    failed["exit_code"] = json!(3);
    failed["stderr"] = json!("invented refusal");
    let mut killed = envelope(snapshot());
    killed["exit_code"] = Value::Null;
    let bad_version = json!({"version":"ticketz-snapshot/99","issues":[],"boards":[]});
    let count = json!({"version":"ticketz-snapshot/1","issues":[],"boards":[{"version":"ticketz-status/1","repo":"example-org/board","issues":1,"last_sync":"now"}]});
    for value in [
        truncated,
        failed,
        killed,
        envelope(bad_version),
        envelope(count),
        json!({}),
        envelope(json!({"version":"ticketz-snapshot/1"})),
    ] {
        let s = server(value).await;
        assert!(Store::new(&s.uri()).unwrap().snapshot().await.is_err());
    }
}
#[tokio::test]
async fn sync_and_reset_send_only_their_contract_arguments() {
    let s = MockServer::start().await;
    Mock::given(method("POST")).and(path("/ticketz/sync")).and(body_json(json!({"repo":"example-org/board","full":true,"actor-permissions":true,"json":true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(envelope(json!({"version":"ticketz-sync/1","repo":"example-org/board","full":true,"fetched":2,"kept":2,"removed":1,"board_files_unknown":"not readable"})))).expect(1).mount(&s).await;
    Mock::given(path("/ticketz/reset"))
        .and(body_json(json!({"json":true})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(envelope(json!({"version":"ticketz-reset/1","removed":2}))),
        )
        .expect(1)
        .mount(&s)
        .await;
    let store = Store::new(&s.uri()).unwrap();
    assert_eq!(
        store
            .sync(&"example-org/board".parse().unwrap(), true, true)
            .await
            .unwrap()
            .removed,
        Some(1)
    );
    assert_eq!(store.clear().await.unwrap(), 2);
}
#[tokio::test]
async fn redirects_are_not_followed_and_http_refusals_are_errors() {
    let target = MockServer::start().await;
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", target.uri()))
        .mount(&s)
        .await;
    assert!(Store::new(&s.uri()).unwrap().snapshot().await.is_err());
    assert!(target.received_requests().await.unwrap().is_empty());
}
#[test]
fn invalid_endpoints_are_rejected_before_network() {
    for url in [
        "some.sqlite",
        "http://example.com",
        "https://user:password@127.0.0.1",
        "http://localhost:4287?api=other",
    ] {
        assert!(Store::new(url).is_err());
    }
}
