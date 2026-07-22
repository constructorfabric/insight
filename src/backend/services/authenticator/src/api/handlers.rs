//! HTTP handlers for the step-04 surface: `/auth/login`, `/auth/callback`,
//! `/internal/authz`, `/.well-known/jwks.json`, `/auth/me`, `/auth/logout`.
//!
//! Deferred (later steps): `/auth/refresh`, `/auth/sessions`, CSRF enforcement,
//! back-channel logout, `/internal/token`.

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::Query;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum_extra::extract::cookie::CookieJar;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand::RngCore as _;
use serde::Deserialize;
use uuid::Uuid;

use crate::api::AppState;
use crate::api::error::{OidcError, PersonError};
use crate::cookie;
use crate::identity::PersonResolution;
use crate::jwt::GatewayClaims;
use crate::session::{LoginState, NewSession, SessionRecord};

/// Header carrying the minted JWT back to nginx (`auth_request_set`).
static X_GATEWAY_JWT: HeaderName = HeaderName::from_static("x-gateway-jwt");

// ── /auth/login ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LoginParams {
    #[serde(default)]
    return_to: Option<String>,
}

/// Start the OIDC code+PKCE flow: stash state/nonce/verifier, 302 to the IdP.
pub async fn login(
    Extension(state): Extension<Arc<AppState>>,
    Query(params): Query<LoginParams>,
) -> Response {
    let return_to = sanitize_return_to(params.return_to.as_deref(), &state.cfg.default_return_to);

    // openidconnect generates the state, nonce, and PKCE pair; we stash the
    // verifier + nonce under the state key for the callback to replay.
    let start = match state
        .oidc
        .authorize(&state.cfg.redirect_uri, &state.cfg.oidc_scopes)
        .await
    {
        Ok(s) => s,
        Err(e) => return internal_problem("oidc_authorize", &e),
    };

    if let Err(e) = state
        .sessions
        .put_login_state(
            &start.state,
            &LoginState {
                pkce_verifier: start.pkce_verifier,
                nonce: start.nonce,
                return_to,
            },
            300,
        )
        .await
    {
        return internal_problem("login_state_store", &e);
    }

    build_response(
        StatusCode::FOUND,
        vec![(LOCATION.clone(), start.url)],
        Body::empty(),
    )
}

// ── /auth/callback ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// Complete login: validate state, exchange the code, guard against session
/// fixation, resolve the person (with audited bootstrap), then create the
/// session + linked JWT in one pipeline and set the cookie.
// One linear login flow with per-step error mapping; splitting it would scatter
// the sequence without making it clearer.
#[allow(clippy::too_many_lines)]
pub async fn callback(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(params): Query<CallbackParams>,
) -> Response {
    if let Some(err) = params.error {
        return OidcError::invalid_argument()
            .with_field_violation("error", err, "IDP_ERROR")
            .create()
            .into_response();
    }
    let (Some(code), Some(oidc_state)) = (params.code, params.state) else {
        return OidcError::invalid_argument()
            .with_field_violation("state", "missing code or state", "MISSING")
            .create()
            .into_response();
    };

    // Validate state -> recover PKCE verifier + nonce (one-shot).
    let login_state = match state.sessions.take_login_state(&oidc_state).await {
        Ok(Some(ls)) => ls,
        Ok(None) => {
            return OidcError::invalid_argument()
                .with_field_violation("state", "unknown or expired state", "STATE_MISMATCH")
                .create()
                .into_response();
        }
        Err(e) => return internal_problem("login_state_take", &e),
    };

    let idp = match state
        .oidc
        .exchange_code_pkce(
            &state.cfg.redirect_uri,
            &code,
            &login_state.pkce_verifier,
            &login_state.nonce,
        )
        .await
    {
        Ok(idp) => idp,
        Err(e) => {
            // {:#} = full anyhow chain, so the log names WHY (incl. the IdP's error_description).
            tracing::warn!(
                error = format!("{e:#}"),
                "oidc code exchange / id_token validation failed"
            );
            return OidcError::invalid_argument()
                .with_field_violation("code", "token exchange failed", "EXCHANGE_FAILED")
                .create()
                .into_response();
        }
    };

    // Session-fixation guard: never reuse an incoming session; revoke any live
    // one named by the presented cookie before minting the new session.
    if let Some(old_token) = cookie::read(&jar)
        && let Ok(Some((old_sid, _))) = state.sessions.resolve_by_token(&old_token).await
    {
        let _ = state.sessions.revoke_session(&old_sid).await;
        tracing::info!(session_id = %old_sid, "session-fixation guard: revoked presented session");
    }

    // Resolve the internal person. Unknown -> 403 (first-admin bootstrap / RBAC
    // are out of step-04 scope; local dev seeds the persons table).
    let resolution = match state.resolver.resolve(&idp.identity).await {
        Ok(Some(p)) => p,
        Ok(None) => {
            tracing::warn!(
                target: "audit",
                event = "login_denied_unknown_person",
                idp_sub = %idp.identity.sub,
                email = %idp.identity.email,
                "login denied: no matching person in Identity"
            );
            return PersonError::permission_denied()
                .with_reason("unknown_person")
                .create()
                .into_response();
        }
        Err(e) => return internal_problem("person_resolution", &e),
    };

    // `return_to` was sanitized at login time and stored with the login state.
    let return_to = login_state.return_to.clone();
    match mint_and_store_session(&state, &idp, &resolution).await {
        Ok(token) => {
            let jar = jar.add(cookie::session_cookie(
                &token,
                state.cfg.session_ttl_seconds,
            ));
            let redirect = build_response(
                StatusCode::FOUND,
                vec![(LOCATION.clone(), return_to)],
                Body::empty(),
            );
            (jar, redirect).into_response()
        }
        Err(e) => internal_problem("create_session", &e),
    }
}

/// Build claims, sign the linked JWT, and persist the session in one pipeline.
/// Returns the cookie token.
async fn mint_and_store_session(
    state: &AppState,
    idp: &crate::oidc::AuthenticatedIdp,
    resolution: &PersonResolution,
) -> anyhow::Result<String> {
    let now = now_secs();
    let cfg = &state.cfg;
    let expires_at = now + cfg.session_ttl_seconds;
    let absolute_expires_at = now + cfg.session_absolute_lifetime_seconds;

    let session_id = Uuid::now_v7().to_string();
    let token = csprng_token();
    let csrf_token = csprng_token();

    // Default roles only — RBAC/ACL is a later initiative (DD-AUTH-07); the
    // permissions service will replace these values, never the claim shape.
    let roles = cfg.default_roles.clone();

    // exp clamped to the session absolute cap (cheap hygiene, G3).
    let exp = (now + cfg.jwt_ttl_seconds).min(absolute_expires_at);
    let claims = GatewayClaims {
        sub: resolution.person_id.clone(),
        tenant_id: resolution.tenant_id.clone(),
        roles: roles.clone(),
        sub_type: "user".to_owned(),
        sid: session_id.clone(),
        iss: cfg.gateway_issuer.clone(),
        aud: cfg.jwt_audience.clone(),
        iat: now,
        exp,
        jti: Uuid::now_v7().to_string(),
    };
    let jwt = state.keystore.sign(&claims)?;

    // Schedule the IdP background refresh (consumer lands in step 10).
    let refresh_due_at = if cfg.idp.refresh_enabled {
        idp.expires_in.map(|ttl| {
            let base = now + ttl.saturating_sub(cfg.idp.refresh_safety_margin_seconds);
            base.saturating_add_signed(jitter_seconds(30))
        })
    } else {
        None
    };

    let record = SessionRecord {
        person_id: resolution.person_id.clone(),
        email: idp.identity.email.clone(),
        tenant_id: resolution.tenant_id.clone(),
        roles,
        idp_iss: idp.issuer.clone(),
        idp_sub: idp.identity.sub.clone(),
        idp_sid: idp.idp_sid.clone(),
        id_token: idp.id_token.clone(),
        idp_refresh_token: idp.refresh_token.clone(),
        idp_access_expires_at: idp.expires_in.map(|ttl| now + ttl),
        created_at: now,
        expires_at,
        absolute_expires_at,
        user_agent: String::new(),
        ip: String::new(),
        csrf_token,
        current_token: token.clone(),
    };

    state
        .sessions
        .create_session(&NewSession {
            session_id,
            token: token.clone(),
            record,
            jwt,
            jwt_reissue_after_seconds: cfg.jwt_reissue_after_seconds,
            refresh_due_at,
        })
        .await?;

    Ok(token)
}

// ── /internal/authz ─────────────────────────────────────────────────────────

/// The gateway `auth_request` target: cookie -> linked JWT, reissue ahead of
/// expiry, emit `X-Gateway-Jwt` + `Cache-Control`.
pub async fn authz(Extension(state): Extension<Arc<AppState>>, jar: CookieJar) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated();
    };
    let resolved = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated(),
        Err(e) => {
            tracing::warn!(error = %e, "authz: session store unavailable");
            return internal_problem("session_store", &e);
        }
    };
    let (session_id, record) = resolved;
    let now = now_secs();
    if record.expires_at <= now || record.absolute_expires_at <= now {
        return unauthenticated();
    }

    // Serve the stored JWT while fresh; reissue-ahead when the stored copy has
    // expired (its TTL is jwt_reissue_after_seconds).
    let jwt = match state.sessions.get_linked_jwt(&session_id).await {
        Ok(Some(jwt)) => jwt,
        Ok(None) => match reissue_jwt(&state, &session_id, &record, now).await {
            Ok(jwt) => jwt,
            Err(e) => return internal_problem("jwt_reissue", &e),
        },
        Err(e) => return internal_problem("linked_jwt_read", &e),
    };

    let exp = jwt_exp(&jwt).unwrap_or(now + state.cfg.jwt_ttl_seconds);
    let cache_control = cache_control_for(exp, now, state.cfg.authz_cache_max_age_seconds);
    let bearer = format!("Bearer {jwt}");

    build_response(
        StatusCode::OK,
        vec![
            (X_GATEWAY_JWT.clone(), bearer),
            (CACHE_CONTROL.clone(), cache_control),
        ],
        Body::empty(),
    )
}

/// Rebuild claims from the session record and store a reissued JWT (stampede-safe).
async fn reissue_jwt(
    state: &AppState,
    session_id: &str,
    record: &SessionRecord,
    now: u64,
) -> anyhow::Result<String> {
    let exp = (now + state.cfg.jwt_ttl_seconds).min(record.absolute_expires_at);
    let claims = GatewayClaims {
        sub: record.person_id.clone(),
        tenant_id: record.tenant_id.clone(),
        roles: record.roles.clone(),
        sub_type: "user".to_owned(),
        sid: session_id.to_owned(),
        iss: state.cfg.gateway_issuer.clone(),
        aud: state.cfg.jwt_audience.clone(),
        iat: now,
        exp,
        jti: Uuid::now_v7().to_string(),
    };
    let jwt = state.keystore.sign(&claims)?;
    let won = state
        .sessions
        .store_reissued_jwt(session_id, &jwt, state.cfg.jwt_reissue_after_seconds)
        .await?;
    if won {
        Ok(jwt)
    } else {
        // Lost the race — return the canonical winner (fallback to ours).
        Ok(state
            .sessions
            .get_linked_jwt(session_id)
            .await?
            .unwrap_or(jwt))
    }
}

// ── /.well-known/openid-configuration ─────────────────────────────────────

/// Serve a minimal OIDC discovery document so downstream verifiers
/// (`cf-gears-oidc-authn-plugin`) can resolve the JWKS from the issuer. The
/// plugin fetches `{issuer}/.well-known/openid-configuration` and reads
/// `jwks_uri` — it does not accept a directly-configured JWKS URL. Both fields
/// are derived from `gateway_issuer` (the JWT `iss`), so the advertised issuer
/// matches the token and the JWKS is served from the same origin.
pub async fn openid_configuration(Extension(state): Extension<Arc<AppState>>) -> Response {
    let issuer = state.cfg.gateway_issuer.trim_end_matches('/');
    let body = serde_json::json!({
        "issuer": issuer,
        "jwks_uri": format!("{issuer}/.well-known/jwks.json"),
        // Advertised for OIDC-discovery completeness; downstream verifiers only
        // consume `issuer` + `jwks_uri`. Signing is ES256 (gateway JWT).
        "id_token_signing_alg_values_supported": ["ES256"],
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
    })
    .to_string();
    build_response(
        StatusCode::OK,
        vec![
            (CONTENT_TYPE.clone(), "application/json".to_owned()),
            (CACHE_CONTROL.clone(), "public, max-age=3600".to_owned()),
        ],
        Body::from(body),
    )
}

// ── /.well-known/jwks.json ────────────────────────────────────────────────

/// Serve the public JWKS (current + previous keys), cacheable for an hour.
pub async fn jwks(Extension(state): Extension<Arc<AppState>>) -> Response {
    let body = state.keystore.jwks().to_string();
    build_response(
        StatusCode::OK,
        vec![
            (CONTENT_TYPE.clone(), "application/json".to_owned()),
            (CACHE_CONTROL.clone(), "public, max-age=3600".to_owned()),
        ],
        Body::from(body),
    )
}

// ── /auth/me ────────────────────────────────────────────────────────────────

/// Return the current session summary for the SPA.
pub async fn me(Extension(state): Extension<Arc<AppState>>, jar: CookieJar) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated();
    };
    let (_, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated(),
        Err(e) => return internal_problem("session_store", &e),
    };
    // Same expiry guard as the exchange: don't summarize a session that has
    // passed its cap in the window before Redis's EXPIREAT removes the key.
    let now = now_secs();
    if record.expires_at <= now || record.absolute_expires_at <= now {
        return unauthenticated();
    }

    let margin = state.cfg.session_refresh_safety_margin_seconds;
    let half_jitter = state.cfg.refresh_jitter_seconds / 2;
    let refresh_at = record
        .expires_at
        .saturating_sub(margin)
        .saturating_add_signed(jitter_seconds(half_jitter));

    let body = serde_json::json!({
        "user": record.person_id,
        "email": record.email,
        "tenant_id": record.tenant_id,
        "roles": record.roles,
        "expires_at": record.expires_at,
        "refresh_at": refresh_at,
    })
    .to_string();
    json_ok(body)
}

// ── /auth/logout ─────────────────────────────────────────────────────────────

/// Revoke the session, clear the cookie, and return the RP-logout URL.
pub async fn logout(Extension(state): Extension<Arc<AppState>>, jar: CookieJar) -> Response {
    let mut rp_logout_url = serde_json::Value::Null;

    if let Some(token) = cookie::read(&jar)
        && let Ok(Some((session_id, record))) = state.sessions.resolve_by_token(&token).await
    {
        let _ = state.sessions.revoke_session(&session_id).await;
        tracing::info!(session_id = %session_id, "logout: session revoked");
        if let Some(url) = state
            .oidc
            .rp_logout_url(&record.id_token, &state.cfg.default_return_to)
            .await
        {
            rp_logout_url = serde_json::Value::String(url);
        }
    }

    let body = serde_json::json!({ "rp_logout_url": rp_logout_url }).to_string();
    let jar = jar.add(cookie::clear_cookie());
    let resp = build_response(
        StatusCode::OK,
        vec![(CONTENT_TYPE.clone(), "application/json".to_owned())],
        Body::from(body),
    );
    (jar, resp).into_response()
}

// ── Pure helpers (unit-tested) ───────────────────────────────────────────────

/// Compute the `/internal/authz` 200 `Cache-Control`:
/// `max-age = min(authz_cache_max_age, jwt_exp - now - 60)`; `no-store` when
/// that is 0 or the cache is disabled. Preserves the 60 s travel margin.
#[must_use]
pub fn cache_control_for(exp: u64, now: u64, authz_cache_max_age: u64) -> String {
    let travel_bound = exp.saturating_sub(now).saturating_sub(60);
    let max_age = authz_cache_max_age.min(travel_bound);
    if max_age == 0 {
        "no-store".to_owned()
    } else {
        format!("max-age={max_age}")
    }
}

/// Sanitize an SPA-supplied `return_to`: accept only a site-relative path (one
/// leading `/`, not `//` — which would be protocol-relative / open-redirect).
#[must_use]
pub fn sanitize_return_to(candidate: Option<&str>, default: &str) -> String {
    match candidate {
        Some(p) if p.starts_with('/') && !p.starts_with("//") => p.to_owned(),
        _ => default.to_owned(),
    }
}

// ── Internal plumbing ────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

/// A CSPRNG token (256 bits, base64url) — session token, CSRF token, state, nonce.
fn csprng_token() -> String {
    let mut raw = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    B64.encode(raw)
}

/// A uniform jitter in `[-window, +window]` seconds (thread RNG is fine here).
fn jitter_seconds(window: u64) -> i64 {
    if window == 0 {
        return 0;
    }
    let w = i64::try_from(window).unwrap_or(0);
    rand::Rng::gen_range(&mut rand::thread_rng(), -w..=w)
}

/// Read `exp` from a JWT without verifying (it is our own token).
fn jwt_exp(jwt: &str) -> Option<u64> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = B64.decode(payload_b64).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("exp")?.as_u64()
}

fn build_response(status: StatusCode, headers: Vec<(HeaderName, String)>, body: Body) -> Response {
    let mut resp = Response::new(body);
    *resp.status_mut() = status;
    let h = resp.headers_mut();
    for (name, value) in headers {
        if let Ok(v) = HeaderValue::from_str(&value) {
            h.append(name, v);
        }
    }
    resp
}

fn json_ok(body: String) -> Response {
    build_response(
        StatusCode::OK,
        vec![(CONTENT_TYPE.clone(), "application/json".to_owned())],
        Body::from(body),
    )
}

/// 401 with `Cache-Control: no-store` — a cached 401 would trap a fresh login.
fn unauthenticated() -> Response {
    build_response(
        StatusCode::UNAUTHORIZED,
        vec![
            (CONTENT_TYPE.clone(), "application/json".to_owned()),
            (CACHE_CONTROL.clone(), "no-store".to_owned()),
        ],
        Body::from(r#"{"error":"unauthenticated"}"#),
    )
}

fn internal_problem(context: &str, err: &anyhow::Error) -> Response {
    tracing::error!(context, error = %err, "authenticator internal error");
    toolkit_canonical_errors::CanonicalError::internal(format!("{context}: {err}"))
        .create()
        .into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn cache_control_keeps_60s_travel_margin() {
        // Fresh 300s token, 30s cache bound: min(30, 300-60) = 30.
        assert_eq!(cache_control_for(1_000 + 300, 1_000, 30), "max-age=30");
    }

    #[test]
    fn cache_control_shrinks_near_reissue() {
        // 80s of validity left: min(30, 80-60) = 20 -> shorter cache life so the
        // 60s travel guarantee survives caching by construction.
        assert_eq!(cache_control_for(1_000 + 80, 1_000, 30), "max-age=20");
    }

    #[test]
    fn cache_control_no_store_within_travel_margin() {
        // <=60s left: nothing cacheable, must not hand out a stale-cached hit.
        assert_eq!(cache_control_for(1_000 + 60, 1_000, 30), "no-store");
        assert_eq!(cache_control_for(1_000 + 45, 1_000, 30), "no-store");
    }

    #[test]
    fn cache_control_disabled_is_no_store() {
        // authz_cache_max_age = 0 -> per-request checks, instant revocation.
        assert_eq!(cache_control_for(1_000 + 300, 1_000, 0), "no-store");
    }

    #[test]
    fn return_to_accepts_site_relative_paths() {
        assert_eq!(sanitize_return_to(Some("/dashboard"), "/"), "/dashboard");
    }

    #[test]
    fn return_to_rejects_open_redirects() {
        // Protocol-relative and absolute URLs fall back to the default.
        assert_eq!(sanitize_return_to(Some("//evil.example"), "/"), "/");
        assert_eq!(sanitize_return_to(Some("https://evil.example"), "/"), "/");
        assert_eq!(sanitize_return_to(None, "/home"), "/home");
    }

    #[test]
    fn jwt_exp_reads_payload_without_verification() {
        // header.payload.sig with payload = {"exp": 4000000000}
        let payload = B64.encode(br#"{"exp":4000000000}"#);
        let token = format!("aGVhZGVy.{payload}.c2ln");
        assert_eq!(jwt_exp(&token), Some(4_000_000_000));
    }
}
