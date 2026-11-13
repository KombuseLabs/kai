//! Fetching a board from a mock server that stands in for GitHub, with
//! invented answers: pages, the cursor, the token, and what GitHub refuses.

use std::path::Path;

use github::{Client, Error};
use secrecy::SecretString;
use serde_json::{Value, json};
use types::{Form, IssueState, LabelEvent, RepoName};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "invented-token-0123456789";

/// Answers a GraphQL request whose variables include `variables` with
/// the fixture `answer`, exactly once, and only with the token. The server
/// checks the "exactly once" when it is dropped.
async fn graphql(server: &MockServer, variables: Value, answer: &str) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(body_partial_json(json!({ "variables": variables })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(fixture(answer), "application/json"))
        .expect(1)
        .mount(server)
        .await;
}

fn client(server: &MockServer) -> Client {
    Client::new(SecretString::from(TOKEN), &server.uri()).unwrap()
}

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn board() -> RepoName {
    "example-org/example-board".parse().unwrap()
}

#[tokio::test]
async fn every_page_is_fetched_and_read_into_tickets() {
    let github = MockServer::start().await;
    let first =
        json!({ "owner": "example-org", "name": "example-board", "since": null, "after": null });
    graphql(&github, first, "board-page-1.json").await;
    graphql(
        &github,
        json!({ "after": "Y3Vyc29yOmludmVudGVk" }),
        "board-page-2.json",
    )
    .await;

    let fetched = client(&github).fetch_board(&board(), None).await.unwrap();

    let numbers: Vec<u64> = fetched.tickets.iter().map(|t| t.issue.number).collect();
    assert_eq!(numbers, [1, 2, 3]);
    assert_eq!(fetched.since.as_deref(), Some("2026-01-04T10:00:00Z"));

    let [question, count, sort] = &fetched.tickets[..] else {
        unreachable!()
    };
    assert_eq!(question.form, None);
    assert!(question.labels.is_empty());

    assert_eq!(count.issue.to_string(), "example-org/example-board#2");
    assert_eq!(count.state, IssueState::Open);
    assert_eq!(count.labels, ["kai", "bug"]);
    assert_eq!(
        count.label_events,
        [
            LabelEvent {
                label: "kai".to_owned(),
                actor: Some("example-maintainer".to_owned()),
                at: "2026-01-03T09:00:00Z".to_owned(),
            },
            LabelEvent {
                label: "bug".to_owned(),
                actor: None,
                at: "2026-01-03T09:30:00Z".to_owned(),
            },
        ]
    );
    assert_eq!(
        count.form,
        Some(Form {
            instruction: Some(
                "Make `widgets --count` print how many widgets there are.".to_owned()
            ),
            repo: Some("example-org/example-code".to_owned()),
            base_ref: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
            check: None,
            hidden_tests: Some("tests/hidden/count.rs".to_owned()),
        })
    );
    assert_eq!(sort.state, IssueState::Closed);
}

#[tokio::test]
async fn the_cursor_is_sent_and_kept_when_nothing_changed() {
    let github = MockServer::start().await;
    let since = "2026-01-05T00:00:00Z";
    graphql(&github, json!({ "since": since }), "board-empty.json").await;

    let fetched = client(&github)
        .fetch_board(&board(), Some(since))
        .await
        .unwrap();

    assert!(fetched.tickets.is_empty());
    assert_eq!(fetched.since.as_deref(), Some(since));
}

#[tokio::test]
async fn a_board_github_does_not_know_is_an_error_with_githubs_words() {
    let github = MockServer::start().await;
    graphql(
        &github,
        json!({ "name": "missing-board" }),
        "board-not-found.json",
    )
    .await;
    let missing: RepoName = "example-org/missing-board".parse().unwrap();

    match client(&github).fetch_board(&missing, None).await {
        Err(Error::Query(messages)) => {
            assert!(messages[0].contains("Could not resolve"), "{messages:?}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_bad_token_is_an_error_with_the_status_and_without_the_token() {
    let github = MockServer::start().await;
    let mock = Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_raw(fixture("bad-credentials.json"), "application/json"),
        );
    mock.mount(&github).await;
    let client = client(&github);

    let err = client.fetch_board(&board(), None).await.unwrap_err();

    assert!(matches!(err, Error::Status { status: 401, .. }), "{err:?}");
    for shown in [err.to_string(), format!("{err:?}"), format!("{client:?}")] {
        assert!(!shown.contains(TOKEN), "{shown}");
    }
}

#[test]
fn plain_http_is_refused_unless_it_stays_on_this_machine() {
    let token = || SecretString::from(TOKEN);
    for api in [
        "http://api.example.com",
        // A user part that makes another host look like this machine.
        "http://127.0.0.1:8080@192.0.2.1",
        "http://localhost.example.com",
        "ftp://127.0.0.1",
    ] {
        assert!(
            matches!(Client::new(token(), api), Err(Error::Api(_))),
            "{api}"
        );
    }
    for api in [
        "https://api.example.com",
        "http://127.0.0.1:8080",
        "http://localhost",
        "http://[::1]:8080/",
    ] {
        assert!(Client::new(token(), api).is_ok(), "{api}");
    }
}
