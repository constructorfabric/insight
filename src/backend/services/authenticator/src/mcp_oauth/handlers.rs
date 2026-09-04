use std::sync::Arc;

use axum::Extension;
use axum::extract::{Form, Query};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION, RETRY_AFTER,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum_extra::extract::cookie::CookieJar;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand::Rng as _;
use serde_json::json;
use sha2::{Digest as _, Sha256};
use url::Url;
use uuid::Uuid;

use crate::api::AppState;
use crate::audit::AuditEvent;
use crate::cookie;
use crate::jwt::McpAccessClaims;
use crate::ratelimit::BucketSpec;
use crate::session::SessionRecord;

use super::store::McpOAuthStoreError;
use super::types::{
    AuthorizationCodeGrant, AuthorizationDecision, AuthorizeQuery, ClientRegistrationRequest,
    MCP_SCOPE, OAuthError, PendingAuthorization, RefreshGrant, RegisteredClient, RevocationRequest,
    TokenRequest, TokenResponse, redirect_uri_matches, valid_code_verifier, validated_registration,
};

const ADMIN_ROLE: &str = "admin";
const CLIENT_REGISTRATION_TTL_SECONDS: u64 = 365 * 24 * 60 * 60;
const MAX_REGISTERED_CLIENTS: u64 = 1_000;
const REGISTRATION_RATE_LIMIT: BucketSpec = BucketSpec {
    burst: 20,
    per_minute: 20,
};
const MAX_PARAMETER_LENGTH: usize = 2_048;
const REFERRER_POLICY: &str = "referrer-policy";
const X_CONTENT_TYPE_OPTIONS: &str = "x-content-type-options";

#[derive(Debug)]
struct OAuthFailure {
    status: StatusCode,
    error: &'static str,
    description: String,
}

impl OAuthFailure {
    fn response(self) -> Response {
        oauth_error(self.status, self.error, &self.description)
    }
}

pub async fn register_client(
    Extension(state): Extension<Arc<AppState>>,
    axum::Json(request): axum::Json<ClientRegistrationRequest>,
) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let registration_allowed = state
        .sessions
        .rate_limit_take(
            "mcp_registration",
            "global",
            REGISTRATION_RATE_LIMIT,
            now_secs(),
        )
        .await;
    match registration_allowed {
        Ok(true) => {}
        Ok(false) => return registration_limited("client registration rate limit exceeded"),
        Err(error) => {
            tracing::error!(error = %error, "could not enforce MCP client registration limit");
            return temporarily_unavailable();
        }
    }

    let client = match validated_registration(request, random_token()) {
        Ok(client) => client,
        Err(reason) => {
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", reason);
        }
    };
    match state
        .mcp_oauth
        .put_client(
            &client,
            CLIENT_REGISTRATION_TTL_SECONDS,
            MAX_REGISTERED_CLIENTS,
            now_secs(),
        )
        .await
    {
        Ok(()) => {}
        Err(McpOAuthStoreError::ClientQuotaExceeded) => {
            return registration_limited("client registration capacity exhausted");
        }
        Err(error) => {
            tracing::error!(error = %error, "could not store MCP OAuth client");
            return temporarily_unavailable();
        }
    }

    json_response(StatusCode::CREATED, &client)
}

pub async fn authorization_server_metadata(Extension(state): Extension<Arc<AppState>>) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let origin = public_origin(&state);
    json_response(
        StatusCode::OK,
        &json!({
            "issuer": origin,
            "authorization_endpoint": endpoint(&origin, "/auth/oauth/authorize"),
            "token_endpoint": endpoint(&origin, "/auth/oauth/token"),
            "registration_endpoint": endpoint(&origin, "/auth/oauth/register"),
            "revocation_endpoint": endpoint(&origin, "/auth/oauth/revoke"),
            "jwks_uri": endpoint(&origin, "/.well-known/jwks.json"),
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": ["S256"],
            "token_endpoint_auth_methods_supported": ["none"],
            "revocation_endpoint_auth_methods_supported": ["none"],
            "scopes_supported": [MCP_SCOPE],
        }),
    )
}

pub async fn protected_resource_metadata(Extension(state): Extension<Arc<AppState>>) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let origin = public_origin(&state);
    json_response(
        StatusCode::OK,
        &json!({
            "resource": resource_url(&state),
            "authorization_servers": [origin],
            "scopes_supported": [MCP_SCOPE],
            "bearer_methods_supported": ["header"],
        }),
    )
}

pub async fn authorize(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(query): Query<AuthorizeQuery>,
) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let (request_id, pending) = match authorization_request(&state, query).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some((session_id, record)) = browser_session(&state, &jar).await else {
        return redirect_to_login(&state, &request_id);
    };
    if !fresh_admin(&state, &record).await {
        return authorization_redirect(&pending, Some(("access_denied", "admin access required")));
    }

    consent_page(&record, &session_id, &request_id, &pending)
}

pub async fn decide(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(decision): Form<AuthorizationDecision>,
) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    let Some((session_id, record)) = browser_session(&state, &jar).await else {
        return oauth_error(StatusCode::UNAUTHORIZED, "access_denied", "session expired");
    };
    let pending = match state.mcp_oauth.take_pending(&decision.request_id).await {
        Ok(Some(pending)) => pending,
        Ok(None) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "authorization request expired",
            );
        }
        Err(error) => {
            tracing::error!(error = %error, "could not consume MCP authorization request");
            return oauth_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                "authorization server unavailable",
            );
        }
    };
    if decision.decision != "approve" {
        audit(
            &state,
            "mcp_authorization",
            "failure",
            &record,
            &session_id,
            &headers,
            json!({"reason": "denied"}),
        );
        return redirect_json(&pending, Some(("access_denied", "authorization denied")));
    }
    if !fresh_admin(&state, &record).await {
        return redirect_json(&pending, Some(("access_denied", "admin access required")));
    }

    let code = random_token();
    let grant = AuthorizationCodeGrant {
        client_id: pending.client_id.clone(),
        redirect_uri: pending.redirect_uri.clone(),
        code_challenge: pending.code_challenge.clone(),
        session_id: session_id.clone(),
        resource: pending.resource.clone(),
        scope: pending.scope.clone(),
    };
    if let Err(error) = state
        .mcp_oauth
        .put_code(
            &code,
            &grant,
            state.cfg.mcp_oauth.authorization_code_ttl_seconds,
        )
        .await
    {
        tracing::error!(error = %error, "could not store MCP authorization code");
        return oauth_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
            "authorization server unavailable",
        );
    }

    audit(
        &state,
        "mcp_authorization",
        "success",
        &record,
        &session_id,
        &headers,
        json!({"client_id": pending.client_id}),
    );
    redirect_json_with_code(&pending, &code)
}

pub async fn token(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Form(request): Form<TokenRequest>,
) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    match request.grant_type.as_str() {
        "authorization_code" => exchange_code(&state, &headers, request).await,
        "refresh_token" => exchange_refresh(&state, &headers, request).await,
        _ => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant type is not supported",
        ),
    }
}

pub async fn revoke(
    Extension(state): Extension<Arc<AppState>>,
    Form(request): Form<RevocationRequest>,
) -> Response {
    if !state.cfg.mcp_oauth.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }

    if let Err(error) = state
        .mcp_oauth
        .delete_refresh(&token_hash(&request.token))
        .await
    {
        tracing::warn!(error = %error, "could not revoke MCP refresh token");
        return temporarily_unavailable();
    }
    no_store(StatusCode::OK.into_response())
}

async fn authorization_request(
    state: &AppState,
    query: AuthorizeQuery,
) -> Result<(String, PendingAuthorization), Response> {
    if let Some(request_id) = query.request_id {
        if !valid_opaque_parameter(&request_id) {
            return Err(oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "invalid request_id",
            ));
        }
        return match state.mcp_oauth.pending(&request_id).await {
            Ok(Some(pending)) => Ok((request_id, pending)),
            Ok(None) => Err(oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "authorization request expired",
            )),
            Err(error) => {
                tracing::error!(error = %error, "could not load MCP authorization request");
                Err(oauth_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "temporarily_unavailable",
                    "authorization server unavailable",
                ))
            }
        };
    }

    let client_id =
        required(query.client_id.clone(), "client_id").map_err(OAuthFailure::response)?;
    let redirect_uri =
        required(query.redirect_uri.clone(), "redirect_uri").map_err(OAuthFailure::response)?;
    let client = load_client(state, &client_id).await?;
    if !client
        .redirect_uris
        .iter()
        .any(|registered| redirect_uri_matches(registered, &redirect_uri))
    {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "redirect_uri is not registered",
        ));
    }
    let error_state = query
        .state
        .clone()
        .filter(|state| state.len() <= MAX_PARAMETER_LENGTH);
    let pending = match validate_authorize_query(state, query, client, redirect_uri.clone()) {
        Ok(pending) => pending,
        Err(error) => {
            return Err(authorization_error_redirect(
                &redirect_uri,
                error_state.as_deref(),
                &error,
            ));
        }
    };
    let request_id = random_token();
    if let Err(error) = state
        .mcp_oauth
        .put_pending(
            &request_id,
            &pending,
            state.cfg.mcp_oauth.authorization_code_ttl_seconds,
        )
        .await
    {
        tracing::error!(error = %error, "could not store MCP authorization request");
        return Err(authorization_redirect(
            &pending,
            Some((
                "temporarily_unavailable",
                "authorization server unavailable",
            )),
        ));
    }
    Ok((request_id, pending))
}

fn validate_authorize_query(
    state: &AppState,
    query: AuthorizeQuery,
    client: RegisteredClient,
    redirect_uri: String,
) -> Result<PendingAuthorization, OAuthFailure> {
    if query.response_type.as_deref() != Some("code") {
        return Err(failure(
            "unsupported_response_type",
            "response_type must be code",
        ));
    }
    if query.code_challenge_method.as_deref() != Some("S256") {
        return Err(failure("invalid_request", "PKCE S256 is required"));
    }
    let code_challenge = required(query.code_challenge, "code_challenge")?;
    if code_challenge.len() != 43 || !valid_opaque_parameter(&code_challenge) {
        return Err(failure("invalid_request", "invalid code_challenge"));
    }
    let state_value = required(query.state, "state")?;
    if state_value.len() > MAX_PARAMETER_LENGTH {
        return Err(failure("invalid_request", "state is too long"));
    }
    let resource = required(query.resource, "resource")?;
    if resource != resource_url(state) {
        return Err(failure("invalid_target", "resource is not supported"));
    }
    let scope = query.scope.unwrap_or_else(|| MCP_SCOPE.to_owned());
    if scope != MCP_SCOPE {
        return Err(failure("invalid_scope", "scope is not supported"));
    }

    Ok(PendingAuthorization {
        client_id: client.client_id,
        client_name: client.client_name,
        redirect_uri,
        code_challenge,
        state: state_value,
        resource,
        scope,
    })
}

async fn exchange_code(state: &AppState, headers: &HeaderMap, request: TokenRequest) -> Response {
    let Ok(client_id) = required(request.client_id, "client_id") else {
        return invalid_grant();
    };
    let Ok(code) = required(request.code, "code") else {
        return invalid_grant();
    };
    let Ok(redirect_uri) = required(request.redirect_uri, "redirect_uri") else {
        return invalid_grant();
    };
    let Ok(verifier) = required(request.code_verifier, "code_verifier") else {
        return invalid_grant();
    };
    if !valid_code_verifier(&verifier) {
        return invalid_grant();
    }
    let grant = match state.mcp_oauth.take_code(&code).await {
        Ok(Some(grant)) => grant,
        Ok(None) => return invalid_grant(),
        Err(error) => {
            tracing::error!(error = %error, "could not consume MCP authorization code");
            return temporarily_unavailable();
        }
    };
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    if grant.client_id != client_id
        || grant.redirect_uri != redirect_uri
        || grant.code_challenge != challenge
        || request.resource.as_deref() != Some(grant.resource.as_str())
    {
        return invalid_grant();
    }
    let Some(record) = active_admin_session(state, &grant.session_id).await else {
        return invalid_grant();
    };

    issue_initial_tokens(state, headers, &record, &grant).await
}

async fn exchange_refresh(
    state: &AppState,
    headers: &HeaderMap,
    request: TokenRequest,
) -> Response {
    let Ok(client_id) = required(request.client_id, "client_id") else {
        return invalid_grant();
    };
    let Ok(refresh_token) = required(request.refresh_token, "refresh_token") else {
        return invalid_grant();
    };
    let old_hash = token_hash(&refresh_token);
    let grant = match state.mcp_oauth.refresh(&old_hash).await {
        Ok(Some(grant)) => grant,
        Ok(None) => return invalid_grant(),
        Err(error) => {
            tracing::error!(error = %error, "could not load MCP refresh token");
            return temporarily_unavailable();
        }
    };
    if grant.client_id != client_id || request.resource.as_deref() != Some(grant.resource.as_str())
    {
        return invalid_grant();
    }
    let Some(record) = active_admin_session(state, &grant.session_id).await else {
        return invalid_grant();
    };

    issue_rotated_tokens(state, headers, &record, &old_hash, &grant).await
}

async fn issue_initial_tokens(
    state: &AppState,
    headers: &HeaderMap,
    record: &SessionRecord,
    grant: &AuthorizationCodeGrant,
) -> Response {
    let refresh_token = random_token();
    let refresh_grant = RefreshGrant {
        client_id: grant.client_id.clone(),
        session_id: grant.session_id.clone(),
        resource: grant.resource.clone(),
        scope: grant.scope.clone(),
    };
    let Some(ttl) = session_ttl(record) else {
        return invalid_grant();
    };
    if let Err(error) = state
        .mcp_oauth
        .put_refresh(&token_hash(&refresh_token), &refresh_grant, ttl)
        .await
    {
        tracing::error!(error = %error, "could not store MCP refresh token");
        return temporarily_unavailable();
    }

    token_response(
        state,
        headers,
        record,
        &refresh_grant,
        refresh_token,
        "mcp_token_issued",
    )
}

async fn issue_rotated_tokens(
    state: &AppState,
    headers: &HeaderMap,
    record: &SessionRecord,
    old_hash: &str,
    grant: &RefreshGrant,
) -> Response {
    let refresh_token = random_token();
    let new_hash = token_hash(&refresh_token);
    let Some(ttl) = session_ttl(record) else {
        return invalid_grant();
    };
    match state
        .mcp_oauth
        .rotate_refresh(old_hash, grant, &new_hash, grant, ttl)
        .await
    {
        Ok(true) => token_response(
            state,
            headers,
            record,
            grant,
            refresh_token,
            "mcp_token_refreshed",
        ),
        Ok(false) => invalid_grant(),
        Err(error) => {
            tracing::error!(error = %error, "could not rotate MCP refresh token");
            temporarily_unavailable()
        }
    }
}

fn token_response(
    state: &AppState,
    headers: &HeaderMap,
    record: &SessionRecord,
    grant: &RefreshGrant,
    refresh_token: String,
    action: &'static str,
) -> Response {
    let now = now_secs();
    let expires_in = state
        .cfg
        .mcp_oauth
        .access_token_ttl_seconds
        .min(record.expires_at.saturating_sub(now))
        .min(record.absolute_expires_at.saturating_sub(now));
    if expires_in == 0 {
        return invalid_grant();
    }
    let claims = McpAccessClaims {
        sub: record.person_id.clone(),
        tenant_id: record.tenant_id.clone(),
        roles: vec![ADMIN_ROLE.to_owned()],
        sub_type: "user".to_owned(),
        sid: grant.session_id.clone(),
        iss: public_origin(state),
        aud: grant.resource.clone(),
        scope: grant.scope.clone(),
        iat: now,
        exp: now + expires_in,
        jti: Uuid::now_v7().to_string(),
    };
    let access_token = match state.keystore.sign_mcp_access_token(&claims) {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(error = %error, "could not sign MCP access token");
            return temporarily_unavailable();
        }
    };

    audit(
        state,
        action,
        "success",
        record,
        &grant.session_id,
        headers,
        json!({"client_id": grant.client_id}),
    );
    json_response(
        StatusCode::OK,
        &TokenResponse {
            access_token,
            token_type: "Bearer",
            expires_in,
            refresh_token,
            scope: grant.scope.clone(),
        },
    )
}

async fn load_client(state: &AppState, client_id: &str) -> Result<RegisteredClient, Response> {
    if !valid_opaque_parameter(client_id) {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid client_id",
        ));
    }
    match state.mcp_oauth.client(client_id).await {
        Ok(Some(client)) => Ok(client),
        Ok(None) => Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "unknown client_id",
        )),
        Err(error) => {
            tracing::error!(error = %error, "could not load MCP OAuth client");
            Err(temporarily_unavailable())
        }
    }
}

async fn browser_session(state: &AppState, jar: &CookieJar) -> Option<(String, SessionRecord)> {
    let token = cookie::read(jar)?;
    let (session_id, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(value) => value?,
        Err(error) => {
            tracing::warn!(error = %error, "could not resolve MCP authorization session");
            return None;
        }
    };
    if session_expired(&record) || !record.impersonator_person_id.is_empty() {
        return None;
    }
    Some((session_id, record))
}

async fn active_admin_session(state: &AppState, session_id: &str) -> Option<SessionRecord> {
    let record = match state.sessions.load_session(session_id).await {
        Ok(Some(record)) => record,
        Ok(None) => return None,
        Err(error) => {
            tracing::warn!(error = %error, "could not load MCP token session");
            return None;
        }
    };
    if session_expired(&record)
        || !record.impersonator_person_id.is_empty()
        || !fresh_admin(state, &record).await
    {
        return None;
    }
    Some(record)
}

async fn fresh_admin(state: &AppState, record: &SessionRecord) -> bool {
    match state
        .resolver
        .active_roles(&record.person_id, &record.tenant_id)
        .await
    {
        Ok(roles) => roles.iter().any(|role| role == ADMIN_ROLE),
        Err(error) => {
            tracing::warn!(error = %error, "could not refresh MCP admin authorization");
            false
        }
    }
}

fn consent_page(
    record: &SessionRecord,
    _session_id: &str,
    request_id: &str,
    pending: &PendingAuthorization,
) -> Response {
    let nonce = random_token();
    let redirect_origin = redirect_origin(&pending.redirect_uri);
    let body = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Authorize data access</title><style>body{{font:16px system-ui;max-width:42rem;margin:4rem auto;padding:0 1rem;color:#17202a}}main{{border:1px solid #d5d8dc;border-radius:12px;padding:2rem}}button{{font:inherit;padding:.65rem 1rem;margin-right:.5rem}}.approve{{background:#17202a;color:white;border:0;border-radius:6px}}</style></head><body><main><h1>Authorize data exploration</h1><p>Client-provided name: <strong>{}</strong> (unverified).</p><p>Redirect destination: <strong>{}</strong>.</p><p>This client is requesting read-only SQL access to this Insight instance.</p><p>This connection will act as <strong>{}</strong> and requires an active administrator role.</p><form id="decision"><input type="hidden" name="request_id" value="{}"><input type="hidden" id="csrf" value="{}"><button class="approve" name="decision" value="approve">Approve access</button><button name="decision" value="cancel">Cancel</button></form><p id="error" role="alert"></p></main><script nonce="{}">const form=document.getElementById('decision');form.addEventListener('submit',async event=>{{event.preventDefault();const button=event.submitter;const data=new URLSearchParams(new FormData(form));data.set('decision',button.value);const csrf=document.getElementById('csrf').value;const response=await fetch('/auth/oauth/decision',{{method:'POST',headers:{{'content-type':'application/x-www-form-urlencoded','x-csrf-token':csrf}},body:data}});const result=await response.json();if(response.ok&&result.redirect_to){{location.assign(result.redirect_to)}}else{{document.getElementById('error').textContent=result.error_description||'Authorization failed'}}}});</script></body></html>"#,
        html_escape(&pending.client_name),
        html_escape(&redirect_origin),
        html_escape(&record.email),
        html_escape(request_id),
        html_escape(&record.csrf_token),
        nonce,
    );
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(csp) = HeaderValue::from_str(&format!(
        "default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-{nonce}'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
    )) {
        response.headers_mut().insert(CONTENT_SECURITY_POLICY, csp);
    }
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn redirect_origin(redirect_uri: &str) -> String {
    Url::parse(redirect_uri).map_or_else(
        |_| "invalid redirect origin".to_owned(),
        |url| url.origin().ascii_serialization(),
    )
}

pub(crate) fn login_continuation_page(return_to: &str) -> Response {
    let body = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Continue authorization</title><style>body{{font:16px system-ui;max-width:42rem;margin:4rem auto;padding:0 1rem;color:#17202a}}main{{border:1px solid #d5d8dc;border-radius:12px;padding:2rem}}a{{display:inline-block;background:#17202a;color:white;border-radius:6px;padding:.65rem 1rem;text-decoration:none}}a:focus-visible{{outline:3px solid #5dade2;outline-offset:3px}}</style></head><body><main><h1>Continue authorization</h1><p>You are signed in. Continue to review the read-only data access request.</p><a href="{}">Continue</a></main></body></html>"#,
        html_escape(return_to),
    );
    let mut response = (StatusCode::OK, body).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    response
        .headers_mut()
        .insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
        .headers_mut()
        .insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

fn redirect_to_login(state: &AppState, request_id: &str) -> Response {
    let Ok(mut url) = Url::parse(&public_origin(state)) else {
        return temporarily_unavailable();
    };
    url.set_path("/auth/login");
    url.query_pairs_mut().append_pair(
        "return_to",
        &format!("/auth/oauth/authorize?request_id={request_id}"),
    );
    redirect(url.as_str())
}

fn authorization_redirect(pending: &PendingAuthorization, error: Option<(&str, &str)>) -> Response {
    match redirect_target(pending, error, None) {
        Ok(target) => redirect(&target),
        Err(error) => error.response(),
    }
}

fn authorization_error_redirect(
    redirect_uri: &str,
    state: Option<&str>,
    error: &OAuthFailure,
) -> Response {
    match redirect_uri_target(
        redirect_uri,
        state,
        Some((error.error, error.description.as_str())),
        None,
    ) {
        Ok(target) => redirect(&target),
        Err(error) => error.response(),
    }
}

fn redirect_json(pending: &PendingAuthorization, error: Option<(&str, &str)>) -> Response {
    match redirect_target(pending, error, None) {
        Ok(target) => json_response(StatusCode::OK, &json!({"redirect_to": target})),
        Err(error) => error.response(),
    }
}

fn redirect_json_with_code(pending: &PendingAuthorization, code: &str) -> Response {
    match redirect_target(pending, None, Some(code)) {
        Ok(target) => json_response(StatusCode::OK, &json!({"redirect_to": target})),
        Err(error) => error.response(),
    }
}

fn redirect_target(
    pending: &PendingAuthorization,
    error: Option<(&str, &str)>,
    code: Option<&str>,
) -> Result<String, OAuthFailure> {
    redirect_uri_target(&pending.redirect_uri, Some(&pending.state), error, code)
}

fn redirect_uri_target(
    redirect_uri: &str,
    state: Option<&str>,
    error: Option<(&str, &str)>,
    code: Option<&str>,
) -> Result<String, OAuthFailure> {
    let mut url =
        Url::parse(redirect_uri).map_err(|_| failure("invalid_request", "invalid redirect URI"))?;
    let mut query = url.query_pairs_mut();
    if let Some(state) = state {
        query.append_pair("state", state);
    }
    if let Some(code) = code {
        query.append_pair("code", code);
    }
    if let Some((name, description)) = error {
        query.append_pair("error", name);
        query.append_pair("error_description", description);
    }
    drop(query);
    Ok(url.into())
}

fn redirect(target: &str) -> Response {
    let Ok(location) = HeaderValue::from_str(target) else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "invalid redirect URI",
        );
    };
    let mut response = StatusCode::FOUND.into_response();
    response.headers_mut().insert(LOCATION, location);
    no_store(response)
}

fn required(value: Option<String>, name: &'static str) -> Result<String, OAuthFailure> {
    match value {
        Some(value) if !value.is_empty() && value.len() <= MAX_PARAMETER_LENGTH => Ok(value),
        _ => Err(failure("invalid_request", &format!("{name} is required"))),
    }
}

fn failure(error: &'static str, description: &str) -> OAuthFailure {
    OAuthFailure {
        status: StatusCode::BAD_REQUEST,
        error,
        description: description.to_owned(),
    }
}

fn invalid_grant() -> Response {
    oauth_error(
        StatusCode::BAD_REQUEST,
        "invalid_grant",
        "grant is invalid or expired",
    )
}

fn temporarily_unavailable() -> Response {
    oauth_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "temporarily_unavailable",
        "authorization server unavailable",
    )
}

fn registration_limited(description: &str) -> Response {
    let mut response = oauth_error(
        StatusCode::TOO_MANY_REQUESTS,
        "temporarily_unavailable",
        description,
    );
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("60"));
    response
}

fn oauth_error(status: StatusCode, error: &'static str, description: &str) -> Response {
    json_response(
        status,
        &OAuthError {
            error,
            error_description: description.to_owned(),
        },
    )
}

fn json_response<T: serde::Serialize>(status: StatusCode, body: &T) -> Response {
    no_store((status, axum::Json(body)).into_response())
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn public_origin(state: &AppState) -> String {
    state
        .cfg
        .mcp_oauth
        .public_url
        .trim_end_matches('/')
        .to_owned()
}

fn resource_url(state: &AppState) -> String {
    endpoint(&public_origin(state), "/mcp")
}

fn endpoint(origin: &str, path: &str) -> String {
    format!("{}{path}", origin.trim_end_matches('/'))
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    B64.encode(bytes)
}

fn token_hash(token: &str) -> String {
    B64.encode(Sha256::digest(token.as_bytes()))
}

fn valid_opaque_parameter(value: &str) -> bool {
    (32..=256).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn session_expired(record: &SessionRecord) -> bool {
    let now = now_secs();
    now >= record.expires_at || now >= record.absolute_expires_at
}

fn session_ttl(record: &SessionRecord) -> Option<u64> {
    let ttl = record.absolute_expires_at.saturating_sub(now_secs());
    (ttl > 0).then_some(ttl)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn audit(
    state: &AppState,
    action: &'static str,
    outcome: &'static str,
    record: &SessionRecord,
    resource_id: &str,
    headers: &HeaderMap,
    details: serde_json::Value,
) {
    state.audit.emit(AuditEvent {
        action,
        outcome,
        tenant_id: record.tenant_id.clone(),
        actor_person_id: record.person_id.clone(),
        actor_ip: record.ip.clone(),
        actor_user_agent: record.user_agent.clone(),
        correlation_id: headers
            .get("x-correlation-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
        resource_type: "mcp_connection",
        resource_id: resource_id.to_owned(),
        details,
    });
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use axum::http::StatusCode;
    use axum::http::header::{CACHE_CONTROL, LOCATION};
    use url::Url;

    use super::{OAuthFailure, authorization_error_redirect, redirect_origin, redirect_uri_target};

    type R = Result<(), Box<dyn Error>>;

    #[test]
    fn authorization_error_redirect_returns_error_and_state_to_client() -> R {
        let failure = OAuthFailure {
            status: StatusCode::BAD_REQUEST,
            error: "invalid_request",
            description: "response_type must be code".to_owned(),
        };

        let response = authorization_error_redirect(
            "http://127.0.0.1:3210/callback?client=codex",
            Some("opaque-state"),
            &failure,
        );

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
        let location = response.headers()[LOCATION].to_str()?;
        let location = Url::parse(location)?;
        let query = location
            .query_pairs()
            .collect::<std::collections::HashMap<_, _>>();
        assert!(matches!(query.get("client"), Some(value) if value == "codex"));
        assert!(matches!(query.get("error"), Some(value) if value == "invalid_request"));
        assert!(matches!(
            query.get("error_description"),
            Some(value) if value == "response_type must be code"
        ));
        assert!(matches!(query.get("state"), Some(value) if value == "opaque-state"));
        Ok(())
    }

    #[test]
    fn authorization_error_redirect_omits_absent_state() -> R {
        let target = redirect_uri_target(
            "https://client.example/callback",
            None,
            Some(("invalid_scope", "scope is not supported")),
            None,
        )
        .map_err(|error| error.description)?;

        let target = Url::parse(&target)?;
        assert!(target.query_pairs().all(|(name, _)| name != "state"));
        Ok(())
    }

    #[test]
    fn consent_identity_uses_redirect_origin_not_path() {
        assert_eq!(
            redirect_origin("http://127.0.0.1:49152/callback?state=secret"),
            "http://127.0.0.1:49152"
        );
    }
}
