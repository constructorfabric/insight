use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use axum::http::header::WWW_AUTHENTICATE;
use serde_json::json;

use super::*;
use crate::mcp::test_support::Issuer;

#[tokio::test]
async fn a_token_issued_for_this_server_by_an_administrator_is_accepted() {
    let issuer = Issuer::start().await;

    let token = issuer.sign(&issuer.claims());

    assert!(issuer.verifier().verify(&token).await.is_ok());

    issuer.stop();
}

#[tokio::test]
async fn a_token_minted_for_another_resource_is_refused() {
    let issuer = Issuer::start().await;

    let mut claims = issuer.claims();
    claims["aud"] = json!(format!("{}/mcp", issuer.origin));
    let token = issuer.sign(&claims);

    let Err(failure) = issuer.verifier().verify(&token).await else {
        panic!("a token for the read-only server does not open this one");
    };
    assert!(matches!(failure, AuthFailure::Unauthorized), "{failure:?}");

    issuer.stop();
}

#[tokio::test]
async fn a_token_carrying_only_the_read_only_scope_is_refused() {
    let issuer = Issuer::start().await;

    let mut claims = issuer.claims();
    claims["scope"] = json!("openid mcp:query");
    let token = issuer.sign(&claims);

    let Err(failure) = issuer.verifier().verify(&token).await else {
        panic!("the read-only scope does not authorize authoring");
    };
    assert!(
        matches!(failure, AuthFailure::InsufficientScope),
        "{failure:?}"
    );

    issuer.stop();
}

#[tokio::test]
async fn a_token_whose_roles_do_not_include_admin_is_refused() {
    let issuer = Issuer::start().await;

    let mut claims = issuer.claims();
    claims["roles"] = json!("user");
    let token = issuer.sign(&claims);

    let Err(failure) = issuer.verifier().verify(&token).await else {
        panic!("the custom surfaces are administrator-only");
    };
    assert!(
        matches!(failure, AuthFailure::InsufficientScope),
        "{failure:?}"
    );

    issuer.stop();
}

#[tokio::test]
async fn a_token_for_a_service_rather_than_a_person_is_refused() {
    let issuer = Issuer::start().await;

    let mut claims = issuer.claims();
    claims["sub_type"] = json!("service");
    let token = issuer.sign(&claims);

    let Err(failure) = issuer.verifier().verify(&token).await else {
        panic!("an MCP grant belongs to a person, not a service");
    };
    assert!(
        matches!(failure, AuthFailure::InsufficientScope),
        "{failure:?}"
    );

    issuer.stop();
}

#[tokio::test]
async fn an_expired_token_is_refused() {
    let issuer = Issuer::start().await;

    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        panic!("the clock is after the epoch");
    };
    let now = now.as_secs();

    let mut claims = issuer.claims();
    claims["iat"] = json!(now - 1200);
    claims["exp"] = json!(now - 600);
    let token = issuer.sign(&claims);

    let Err(failure) = issuer.verifier().verify(&token).await else {
        panic!("an expired token is not a credential");
    };
    assert!(matches!(failure, AuthFailure::Unauthorized), "{failure:?}");

    issuer.stop();
}

#[tokio::test]
async fn a_token_from_another_issuer_is_refused() {
    let issuer = Issuer::start().await;

    let mut claims = issuer.claims();
    claims["iss"] = json!("https://elsewhere.example.invalid");
    let token = issuer.sign(&claims);

    let Err(failure) = issuer.verifier().verify(&token).await else {
        panic!("only this gateway issues tokens this server accepts");
    };
    assert!(matches!(failure, AuthFailure::Unauthorized), "{failure:?}");

    issuer.stop();
}

#[tokio::test]
async fn a_token_that_is_not_a_token_at_all_is_refused() {
    let issuer = Issuer::start().await;

    let Err(failure) = issuer.verifier().verify("not-a-token").await else {
        panic!("an unparseable bearer is not a credential");
    };
    assert!(matches!(failure, AuthFailure::Unauthorized), "{failure:?}");

    issuer.stop();
}

#[test]
fn a_public_url_that_is_not_an_origin_this_server_can_trust_is_refused() {
    for raw in [
        "",
        "not-a-url",
        "ftp://insight.example.invalid",
        "https://insight.example.invalid/path",
        "https://insight.example.invalid/?query=1",
        "https://user:secret@insight.example.invalid",
        "http://insight.example.invalid",
    ] {
        assert!(
            validate_public_url(raw, false).is_err(),
            "should reject: {raw:?}"
        );
    }
}

#[test]
fn an_https_origin_and_a_loopback_origin_are_both_allowed() {
    assert!(validate_public_url("https://insight.example.invalid", false).is_ok());
    assert!(validate_public_url("http://localhost:3000", false).is_ok());
}

#[test]
fn a_challenge_names_this_server_s_own_metadata_document_and_scope() {
    let Ok(verifier) = TokenVerifier::new("https://insight.example.invalid", "", false) else {
        panic!("a plain https origin is a valid public URL");
    };

    let response = verifier.challenge(StatusCode::UNAUTHORIZED, "invalid_token");

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let Some(header) = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
    else {
        panic!("a challenge carries WWW-Authenticate");
    };
    assert!(
        header.contains("oauth-protected-resource/mcp/v3"),
        "{header}"
    );
    assert!(header.contains(MCP_SCOPE), "{header}");
    assert!(header.contains("invalid_token"), "{header}");
}

#[test]
fn the_keys_can_be_fetched_from_somewhere_the_advertised_origin_is_not() {
    // A local stand advertises `localhost` because the MCP client will not send
    // credentials over http to anything else — and inside this process
    // `localhost` is this container, not the gateway.
    let Ok(verifier) = TokenVerifier::new(
        "http://localhost:8080",
        "http://gateway:8080/.well-known/jwks.json",
        true,
    ) else {
        panic!("a private-network origin with its own keys URL builds");
    };

    assert_eq!(
        verifier.inner.jwks_url,
        "http://gateway:8080/.well-known/jwks.json"
    );
    // The token still has to name the origin the client saw.
    assert_eq!(verifier.inner.audience, "http://localhost:8080/mcp/v3");
    assert_eq!(verifier.inner.issuer, "http://localhost:8080");
}

#[test]
fn without_its_own_keys_url_the_advertised_origin_serves_them() {
    let Ok(verifier) = TokenVerifier::new("https://insight.example.invalid", "", false) else {
        panic!("a public origin builds");
    };

    assert_eq!(
        verifier.inner.jwks_url,
        "https://insight.example.invalid/.well-known/jwks.json"
    );
}
