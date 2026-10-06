//! Each adapter against a local server that plays the provider: what it
//! answers on acceptance, on a rate limit, on a rejection, and when it
//! does not answer at all.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use secrecy::SecretString;
use serde_json::json;

use super::*;
use crate::domain::alerts::delivery::{Message, Provider as _, Receipt, SendError};

#[derive(Clone)]
struct Play {
    status: StatusCode,
    body: serde_json::Value,
    delay: Duration,
    hits: Arc<AtomicUsize>,
}

async fn play(State(play): State<Play>, body: String) -> impl IntoResponse {
    play.hits.fetch_add(1, Ordering::SeqCst);
    tokio::time::sleep(play.delay).await;
    assert!(
        body.contains("Insight"),
        "the request carries the message: {body}"
    );

    (play.status, axum::Json(play.body))
}

async fn provider_at(
    path: &str,
    status: StatusCode,
    body: serde_json::Value,
    delay: Duration,
) -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let router = Router::new().route(path, post(play)).with_state(Play {
        status,
        body,
        delay,
        hits: hits.clone(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("the provider must bind: {error}"));
    let address = listener
        .local_addr()
        .unwrap_or_else(|error| panic!("the provider must have an address: {error}"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    (format!("http://{address}"), hits)
}

fn http() -> reqwest::Client {
    client(Duration::from_millis(300)).unwrap_or_else(|error| panic!("a client builds: {error}"))
}

fn message() -> Message {
    Message {
        text: "Insight alert `a`: m.total = 1 (> 0) at 2026-09-29T00:00:00Z".to_owned(),
    }
}

fn is_unconfirmed(result: &Result<Receipt, SendError>) -> bool {
    matches!(result, Err(SendError::Unconfirmed(_)))
}

fn is_rejected(result: &Result<Receipt, SendError>) -> bool {
    matches!(result, Err(SendError::Rejected(_)))
}

#[tokio::test]
async fn discord_reads_the_message_id_and_classifies_every_other_answer() {
    let cases = [
        (
            "accepted",
            StatusCode::OK,
            json!({"id": "1234"}),
            Ok(Receipt("1234".into())),
        ),
        (
            "rate limited",
            StatusCode::TOO_MANY_REQUESTS,
            json!({"retry_after": 1.5}),
            Err(SendError::Unconfirmed(String::new())),
        ),
        (
            "server error",
            StatusCode::BAD_GATEWAY,
            json!({}),
            Err(SendError::Unconfirmed(String::new())),
        ),
        (
            "unknown webhook",
            StatusCode::NOT_FOUND,
            json!({"code": 10015}),
            Err(SendError::Rejected(String::new())),
        ),
        (
            "accepted without a body it understands",
            StatusCode::OK,
            json!({"nope": true}),
            Err(SendError::Unconfirmed(String::new())),
        ),
    ];

    for (case, status, body, expected) in cases {
        let (url, hits) = provider_at("/webhook", status, body, Duration::ZERO).await;
        let discord = Discord::new(http(), SecretString::from(format!("{url}/webhook")));

        let result = discord.send(&message()).await;

        match expected {
            Ok(receipt) => assert_eq!(result.ok(), Some(receipt), "should accept: {case}"),
            Err(SendError::Unconfirmed(_)) => assert!(
                is_unconfirmed(&result),
                "should be unconfirmed: {case}: {result:?}"
            ),
            Err(SendError::Rejected(_)) => {
                assert!(is_rejected(&result), "should reject: {case}: {result:?}");
            }
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1, "one call: {case}");
    }
}

#[tokio::test]
async fn a_provider_that_does_not_answer_in_time_is_unconfirmed() {
    let (url, _) = provider_at(
        "/webhook",
        StatusCode::OK,
        json!({"id": "1"}),
        Duration::from_secs(2),
    )
    .await;
    let discord = Discord::new(http(), SecretString::from(format!("{url}/webhook")));

    let result = discord.send(&message()).await;

    assert!(is_unconfirmed(&result), "{result:?}");
}

#[tokio::test]
async fn an_answer_streamed_past_the_bound_is_given_up_on_as_unconfirmed() {
    let chunk = axum::body::Bytes::from(vec![b'x'; 16 * 1024]);
    let router = Router::new().route(
        "/webhook",
        post(move || {
            let chunks = std::iter::repeat(chunk.clone()).map(Ok::<_, std::io::Error>);
            async move { axum::body::Body::from_stream(futures::stream::iter(chunks)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("the provider must bind: {error}"));
    let url = format!(
        "http://{}/webhook",
        listener
            .local_addr()
            .unwrap_or_else(|error| panic!("the provider must have an address: {error}"))
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let discord = Discord::new(http(), SecretString::from(url));

    let result = discord.send(&message()).await;

    assert!(
        matches!(&result, Err(SendError::Unconfirmed(reason)) if reason == "answer too large"),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_redirected_post_is_a_rejection_and_the_target_is_never_called() {
    let moved_hits = Arc::new(AtomicUsize::new(0));
    let counted = moved_hits.clone();
    let router = Router::new()
        .route(
            "/webhook",
            post(|| async { (StatusCode::FOUND, [("location", "/moved")]) }),
        )
        .route(
            "/moved",
            post(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                async { axum::Json(json!({"id": "1"})) }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|error| panic!("the provider must bind: {error}"));
    let url = format!(
        "http://{}/webhook",
        listener
            .local_addr()
            .unwrap_or_else(|error| panic!("the provider must have an address: {error}"))
    );
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let discord = Discord::new(http(), SecretString::from(url));

    let result = discord.send(&message()).await;

    assert!(
        matches!(&result, Err(SendError::Rejected(reason)) if reason.contains("302")),
        "{result:?}"
    );
    assert_eq!(moved_hits.load(Ordering::SeqCst), 0, "nothing follows");
}

#[tokio::test]
async fn telegram_reads_ok_and_the_message_id() {
    let cases = [
        (
            "ok",
            StatusCode::OK,
            json!({"ok": true, "result": {"message_id": 77}}),
            Ok(Receipt("77".into())),
        ),
        (
            "not ok with a description",
            StatusCode::OK,
            json!({"ok": false, "description": "chat not found"}),
            Err(SendError::Rejected(String::new())),
        ),
        (
            "unauthorized",
            StatusCode::UNAUTHORIZED,
            json!({"ok": false}),
            Err(SendError::Rejected(String::new())),
        ),
        (
            "too many requests",
            StatusCode::TOO_MANY_REQUESTS,
            json!({"ok": false, "parameters": {"retry_after": 3}}),
            Err(SendError::Unconfirmed(String::new())),
        ),
    ];

    for (case, status, body, expected) in cases {
        let (url, _) = provider_at("/bot{token}/sendMessage", status, body, Duration::ZERO).await;
        let telegram = Telegram::new(http(), SecretString::from("token"), "42".to_owned()).at(&url);

        let result = telegram.send(&message()).await;

        match expected {
            Ok(receipt) => assert_eq!(result.ok(), Some(receipt), "should accept: {case}"),
            Err(SendError::Unconfirmed(_)) => assert!(
                is_unconfirmed(&result),
                "should be unconfirmed: {case}: {result:?}"
            ),
            Err(SendError::Rejected(_)) => {
                assert!(is_rejected(&result), "should reject: {case}: {result:?}");
            }
        }
    }
}

#[tokio::test]
async fn zulip_reads_success_and_the_message_id() {
    let cases = [
        (
            "success",
            StatusCode::OK,
            json!({"result": "success", "id": 9, "msg": ""}),
            Ok(Receipt("9".into())),
        ),
        (
            "error",
            StatusCode::BAD_REQUEST,
            json!({"result": "error", "msg": "Stream does not exist"}),
            Err(SendError::Rejected(String::new())),
        ),
        (
            "server error",
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({}),
            Err(SendError::Unconfirmed(String::new())),
        ),
    ];

    for (case, status, body, expected) in cases {
        let (url, _) = provider_at("/api/v1/messages", status, body, Duration::ZERO).await;
        let zulip = Zulip::new(
            http(),
            &url,
            "bot@example.test".to_owned(),
            SecretString::from("key"),
            "alerts".to_owned(),
            "insight".to_owned(),
        );

        let result = zulip.send(&message()).await;

        match expected {
            Ok(receipt) => assert_eq!(result.ok(), Some(receipt), "should accept: {case}"),
            Err(SendError::Unconfirmed(_)) => assert!(
                is_unconfirmed(&result),
                "should be unconfirmed: {case}: {result:?}"
            ),
            Err(SendError::Rejected(_)) => {
                assert!(is_rejected(&result), "should reject: {case}: {result:?}");
            }
        }
    }
}

#[test]
fn every_configured_destination_becomes_an_adapter_of_its_provider() {
    use std::collections::BTreeMap;

    use crate::config::DestinationConfig;

    let configured = BTreeMap::from([
        (
            "chat".to_owned(),
            DestinationConfig::Discord {
                webhook_url: SecretString::from("https://discord.example.test/api/webhooks/1/x"),
            },
        ),
        (
            "phone".to_owned(),
            DestinationConfig::Telegram {
                bot_token: SecretString::from("t"),
                chat_id: "1".to_owned(),
            },
        ),
        (
            "stream".to_owned(),
            DestinationConfig::Zulip {
                site_url: "https://zulip.example.test".to_owned(),
                bot_email: "b@example.test".to_owned(),
                api_key: SecretString::from("k"),
                stream: "s".to_owned(),
                topic: "t".to_owned(),
            },
        ),
    ]);

    let built = providers(&configured, Duration::from_secs(1))
        .unwrap_or_else(|error| panic!("adapters build: {error}"));
    let shown: Vec<String> = built
        .iter()
        .map(|(name, provider)| format!("{name}={provider:?}"))
        .collect();

    assert_eq!(shown.len(), 3);
    assert!(shown[0].starts_with("chat=Discord"), "{shown:?}");
    assert!(shown[1].starts_with("phone=Telegram"), "{shown:?}");
    assert!(shown[2].starts_with("stream=Zulip"), "{shown:?}");
    assert!(
        !shown.join(" ").contains('k'),
        "no credential is shown: {shown:?}"
    );
}
