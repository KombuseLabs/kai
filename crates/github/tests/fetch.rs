//! Fetching a board from a mock server that stands in for GitHub, with
//! invented answers: pages, the cursor, the token, an account's access,
//! and what GitHub refuses.

use std::path::Path;

use github::{Client, Error};
use secrecy::SecretString;
use serde_json::{Value, json};
use types::{Access, IssueState, LabelEvent, RepoName};
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
async fn every_page_is_fetched_and_read_into_issues_as_github_said_them() {
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

    let numbers: Vec<u64> = fetched.issues.iter().map(|i| i.id.number).collect();
    assert_eq!(numbers, [1, 2, 3]);
    assert_eq!(fetched.since.as_deref(), Some("2026-01-04T10:00:00Z"));

    let [question, count, sort] = &fetched.issues[..] else {
        unreachable!()
    };
    assert_eq!(question.body, "How are the widgets counted?");
    assert_eq!(question.author.as_deref(), Some("example-asker"));
    assert!(question.labels.is_empty());

    assert_eq!(count.id.to_string(), "example-org/example-board#2");
    assert_eq!(count.state, IssueState::Open);
    assert_eq!(count.created_at, "2026-01-01T02:00:00Z");
    assert_eq!(count.labels, ["kai", "bug"]);
    assert!(count.labels_complete && count.label_events_complete);
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
    assert!(
        count
            .body
            .starts_with("### Instruction\n\nMake `widgets --count`")
    );

    assert_eq!(sort.state, IssueState::Closed);
    assert_eq!(sort.author, None);
    assert!(sort.labels_complete);
    assert!(!sort.label_events_complete);
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

    assert!(fetched.issues.is_empty());
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

/// Answers the permission of `login` on the board with `status` and the
/// fixture `answer`, exactly once, and only with the token.
async fn permission(server: &MockServer, login: &str, status: u16, answer: &str) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/example-org/example-board/collaborators/{login}/permission"
        )))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(
            ResponseTemplate::new(status).set_body_raw(fixture(answer), "application/json"),
        )
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn an_accounts_access_is_read_with_its_role() {
    let github = MockServer::start().await;
    permission(
        &github,
        "example-maintainer",
        200,
        "permission-maintain.json",
    )
    .await;

    let access = client(&github)
        .access(&board(), "example-maintainer")
        .await
        .unwrap();

    assert_eq!(
        access,
        Access::Known {
            permission: "write".to_owned(),
            role: Some("maintain".to_owned()),
        }
    );
}

#[tokio::test]
async fn an_access_github_does_not_tell_is_unknown_with_its_words() {
    let github = MockServer::start().await;
    permission(
        &github,
        "example-stranger",
        404,
        "permission-not-found.json",
    )
    .await;

    let access = client(&github)
        .access(&board(), "example-stranger")
        .await
        .unwrap();

    assert_eq!(
        access,
        Access::Unknown("GitHub answered 404: example-stranger is not a user".to_owned())
    );
}

#[tokio::test]
async fn a_login_stays_one_part_of_the_path() {
    let github = MockServer::start().await;
    // Nothing is mounted: a request to any other path answers 404.
    Mock::given(method("GET"))
        .and(path(
            "/repos/example-org/example-board/collaborators/..%2F..%2Fx/permission",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(fixture("permission-maintain.json"), "application/json"),
        )
        .expect(1)
        .mount(&github)
        .await;

    let access = client(&github).access(&board(), "../../x").await.unwrap();

    assert!(matches!(access, Access::Known { .. }), "{access:?}");
}
