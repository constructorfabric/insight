use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use axum::body::to_bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum::{Extension, Form, Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::elliptic_curve::Generate as _;
use p256::pkcs8::{EncodePrivateKey as _, LineEnding};
use redis::AsyncCommands as _;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use uuid::Uuid;

use crate::api::AppState;
use crate::audit::AuditEmitter;
use crate::config::AuthenticatorConfig;
use crate::identity::IdentityPersonResolver;
use crate::issuers::IssuerSelector;
use crate::jwt::KeyStore;
use crate::local_client::LocalClient;
use crate::mcp_oauth::McpOAuthStore;
use crate::service_token::ServiceRegistry;
use crate::session::{SessionManager, SessionRecord};

use super::super::types::{
    AuthorizationCodeGrant, GRANT_LIFETIME_SECONDS, RefreshGrant, RevocationRequest, TokenRequest,
};
use super::{issue_initial_tokens, now_secs, revoke, token, token_hash};

type R = Result<(), Box<dyn Error>>;

fn grant(expires_at: u64) -> RefreshGrant {
    RefreshGrant {
        client_id: Uuid::now_v7().to_string(),
        grant_id: Uuid::now_v7().to_string(),
        person_id: Uuid::from_u128(1).to_string(),
        tenant_id: Uuid::from_u128(2).to_string(),
        resource: "https://example.com/mcp".to_owned(),
        scope: "mcp:query".to_owned(),
        expires_at,
    }
}

#[test]
fn grant_expiry_is_fixed_and_access_tokens_default_to_ten_minutes() {
    let grant = grant(100 + GRANT_LIFETIME_SECONDS);
    for (now, expected) in [
        (100, Some(2_592_000)),
        (101, Some(2_591_999)),
        (grant.expires_at - 1, Some(1)),
        (grant.expires_at, None),
        (grant.expires_at + 1, None),
    ] {
        assert_eq!(grant.remaining_seconds(now), expected, "time: {now}");
    }
    assert_eq!(
        AuthenticatorConfig::default()
            .mcp_oauth
            .access_token_ttl_seconds,
        600
    );
}

async fn body(response: Response, expected: StatusCode) -> Result<Value, Box<dyn Error>> {
    assert_eq!(response.status(), expected);
    let bytes = to_bytes(response.into_body(), 16_384).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn refresh(state: &Arc<AppState>, grant: &RefreshGrant, refresh_token: &str) -> Response {
    token(
        Extension(state.clone()),
        HeaderMap::new(),
        Form(TokenRequest {
            grant_type: "refresh_token".to_owned(),
            client_id: Some(grant.client_id.clone()),
            code: None,
            redirect_uri: None,
            code_verifier: None,
            refresh_token: Some(refresh_token.to_owned()),
            resource: Some(grant.resource.clone()),
        }),
    )
    .await
}

async fn state(redis_url: &str, identity_url: &str) -> Result<Arc<AppState>, Box<dyn Error>> {
    let mut cfg = AuthenticatorConfig::default();
    cfg.mcp_oauth.enabled = true;
    cfg.mcp_oauth.public_url = "https://example.com".to_owned();
    let pem = p256::SecretKey::generate().to_pkcs8_pem(LineEnding::LF)?;
    let keystore = Arc::new(KeyStore::from_pem_for_test(&pem)?);
    let sessions = SessionManager::connect(redis_url).await?;
    let resolver = Arc::new(IdentityPersonResolver::new(
        identity_url,
        keystore.clone(),
        cfg.gateway_issuer.clone(),
        cfg.jwt_audience.clone(),
        cfg.idp.source_type.clone(),
    ));
    Ok(Arc::new(AppState {
        oidc: IssuerSelector::build(&cfg)?,
        service_registry: ServiceRegistry::build(&cfg.service_tokens)?,
        authn_client: Arc::new(LocalClient::new(sessions.clone())),
        mcp_oauth: McpOAuthStore::connect(redis_url).await?,
        cfg,
        sessions,
        keystore,
        resolver,
        audit: AuditEmitter::disabled(),
    }))
}

#[tokio::test]
#[ignore = "requires MCP_TEST_REDIS_URL pointing to an isolated Redis"]
async fn refresh_survives_absent_browser_session_but_enforces_grant_and_admin_access() -> R {
    let redis_url = std::env::var("MCP_TEST_REDIS_URL")?;
    let role_status = Arc::new(AtomicU16::new(200));
    let endpoint_status = role_status.clone();
    let router = Router::new().route(
        "/internal/persons/active-roles",
        get(move || {
            let status = endpoint_status.load(Ordering::Relaxed);
            async move {
                match status {
                    200 => Json(json!({"roles": ["admin"]})).into_response(),
                    204 => Json(json!({"roles": []})).into_response(),
                    _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let identity_url = format!("http://{}", listener.local_addr()?);
    let server = tokio::spawn(async move { axum::serve(listener, router).await });
    let state = state(&redis_url, &identity_url).await?;
    let (stored, old_token) = initial_grant(&state).await?;

    let refreshed = body(refresh(&state, &stored, &old_token).await, StatusCode::OK).await?;
    let new_token = refreshed["refresh_token"]
        .as_str()
        .ok_or("missing rotated token")?;
    assert_ne!(new_token, old_token);
    let rotated = state
        .mcp_oauth
        .refresh(&token_hash(new_token))
        .await?
        .ok_or("missing rotated grant")?;
    assert_eq!(rotated.expires_at, stored.expires_at);
    assert_eq!(rotated.grant_id, stored.grant_id);
    assert_binding_rejected(&state, &stored, new_token).await?;
    let access_token = refreshed["access_token"]
        .as_str()
        .ok_or("missing access token")?;
    let payload = access_token.split('.').nth(1).ok_or("missing claims")?;
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
    assert_eq!(claims["sub"], stored.person_id);
    assert_eq!(claims["tenant_id"], stored.tenant_id);
    assert_eq!(claims["sid"], stored.grant_id);
    assert_eq!(
        claims["exp"].as_u64().ok_or("exp")? - claims["iat"].as_u64().ok_or("iat")?,
        600
    );
    assert_eq!(
        body(
            refresh(&state, &stored, &old_token).await,
            StatusCode::BAD_REQUEST
        )
        .await?["error"],
        "invalid_grant"
    );

    role_status.store(503, Ordering::Relaxed);
    assert_eq!(
        body(
            refresh(&state, &stored, new_token).await,
            StatusCode::SERVICE_UNAVAILABLE
        )
        .await?["error"],
        "temporarily_unavailable"
    );
    assert!(
        state
            .mcp_oauth
            .refresh(&token_hash(new_token))
            .await?
            .is_some()
    );
    role_status.store(204, Ordering::Relaxed);
    assert_eq!(
        body(
            refresh(&state, &stored, new_token).await,
            StatusCode::BAD_REQUEST
        )
        .await?["error"],
        "invalid_grant"
    );
    role_status.store(200, Ordering::Relaxed);

    let response = revoke(
        Extension(state.clone()),
        Form(RevocationRequest {
            token: new_token.to_owned(),
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        body(
            refresh(&state, &stored, new_token).await,
            StatusCode::BAD_REQUEST
        )
        .await?["error"],
        "invalid_grant"
    );

    assert_expired_and_legacy_rejected(&state, &redis_url, &stored).await?;
    assert_access_expiry_capped(&state).await?;
    server.abort();
    Ok(())
}

async fn assert_binding_rejected(state: &Arc<AppState>, grant: &RefreshGrant, token: &str) -> R {
    let mut wrong_client = grant.clone();
    wrong_client.client_id = "another-client".to_owned();
    let mut wrong_resource = grant.clone();
    wrong_resource.resource = "https://other.example.com/mcp".to_owned();
    for wrong in [wrong_client, wrong_resource] {
        assert_eq!(
            body(refresh(state, &wrong, token).await, StatusCode::BAD_REQUEST).await?["error"],
            "invalid_grant"
        );
    }
    assert!(state.mcp_oauth.refresh(&token_hash(token)).await?.is_some());
    Ok(())
}

async fn assert_access_expiry_capped(state: &Arc<AppState>) -> R {
    let grant = grant(now_secs() + 60);
    let token = Uuid::now_v7().to_string();
    state
        .mcp_oauth
        .put_refresh(&token_hash(&token), &grant, 60)
        .await?;
    let result = body(refresh(state, &grant, &token).await, StatusCode::OK).await?;
    let expires_in = result["expires_in"].as_u64().ok_or("missing expiry")?;
    assert!((1..=60).contains(&expires_in));
    let rotated = result["refresh_token"].as_str().ok_or("missing token")?;
    state.mcp_oauth.delete_refresh(&token_hash(rotated)).await?;
    Ok(())
}

async fn initial_grant(state: &Arc<AppState>) -> Result<(RefreshGrant, String), Box<dyn Error>> {
    let original = grant(now_secs() + GRANT_LIFETIME_SECONDS);
    let browser_id = Uuid::now_v7().to_string();
    assert!(state.sessions.load_session(&browser_id).await?.is_none());
    let record = SessionRecord {
        person_id: original.person_id.clone(),
        email: "user@example.com".to_owned(),
        tenant_id: original.tenant_id.clone(),
        roles: vec!["admin".to_owned()],
        idp_iss: String::new(),
        idp_sub: String::new(),
        idp_sid: None,
        id_token: String::new(),
        idp_refresh_token: None,
        idp_access_expires_at: None,
        created_at: now_secs(),
        expires_at: now_secs() + 60,
        absolute_expires_at: now_secs() + 120,
        user_agent: String::new(),
        ip: String::new(),
        csrf_token: String::new(),
        current_token: String::new(),
        impersonator_person_id: String::new(),
        impersonator_email: String::new(),
    };
    let code = AuthorizationCodeGrant {
        client_id: original.client_id.clone(),
        redirect_uri: "http://127.0.0.1:3210/callback".to_owned(),
        code_challenge: String::new(),
        session_id: browser_id,
        resource: original.resource.clone(),
        scope: original.scope.clone(),
        expires_at: original.expires_at,
    };
    let issued = body(
        issue_initial_tokens(state, &HeaderMap::new(), &record, &code).await,
        StatusCode::OK,
    )
    .await?;
    assert_eq!(issued["expires_in"], 600);
    let old_token = issued["refresh_token"]
        .as_str()
        .ok_or("missing refresh token")?;
    let stored = state
        .mcp_oauth
        .refresh(&token_hash(old_token))
        .await?
        .ok_or("missing grant")?;
    assert_eq!(stored.expires_at, original.expires_at);

    Ok((stored, old_token.to_owned()))
}

async fn assert_expired_and_legacy_rejected(
    state: &Arc<AppState>,
    redis_url: &str,
    stored: &RefreshGrant,
) -> R {
    let expired_token = Uuid::now_v7().to_string();
    let expired = grant(now_secs());
    state
        .mcp_oauth
        .put_refresh(&token_hash(&expired_token), &expired, 60)
        .await?;
    assert_eq!(
        body(
            refresh(state, &expired, &expired_token).await,
            StatusCode::BAD_REQUEST
        )
        .await?["error"],
        "invalid_grant"
    );
    state
        .mcp_oauth
        .delete_refresh(&token_hash(&expired_token))
        .await?;

    let legacy_token = Uuid::now_v7().to_string();
    let mut redis = redis::Client::open(redis_url)?
        .get_multiplexed_async_connection()
        .await?;
    let legacy_key = format!("{{mcp_oauth}}:refresh:{}", token_hash(&legacy_token));
    redis
        .set_ex::<_, _, ()>(
            &legacy_key,
            json!({"session_id": "legacy-session"}).to_string(),
            60,
        )
        .await?;
    assert_eq!(
        body(
            refresh(state, stored, &legacy_token).await,
            StatusCode::BAD_REQUEST
        )
        .await?["error"],
        "invalid_grant"
    );
    redis.del::<_, ()>(&legacy_key).await?;
    Ok(())
}
