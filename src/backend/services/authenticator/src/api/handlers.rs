//! HTTP handlers for the browser/gateway surface: `/auth/login`,
//! `/auth/callback`, `/auth/refresh`, `/auth/me`, `/auth/logout`,
//! `/internal/authz`, `/.well-known/jwks.json`.
//!
//! `/internal/token` lives on the dedicated token listener (`service_token`).
//!
//! Spec references in this file (`PRD §x`, `DESIGN §x`) point to the
//! authenticator specs in this repo:
//! `docs/components/backend/authenticator/PRD.md` and
//! `docs/components/backend/authenticator/DESIGN.md`.

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
use crate::api::error::{OidcError, PersonError, SessionError};
use crate::audit::AuditEvent;
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

    // Layer-2 cap (DESIGN §4.4): pre-auth there is no per-caller key, so the
    // guarded resource is the login-state store itself — refuse before any
    // state is written.
    let now = now_secs();
    match state.sessions.live_login_states(now).await {
        Ok(live) if live >= state.cfg.rate_limit.login_state_max => {
            tracing::warn!(
                live,
                "login-state cap reached: refusing /auth/login with 429"
            );
            return too_many_requests("login_state_cap", 30);
        }
        Ok(_) => {}
        Err(e) => return internal_problem("login_state_count", &e),
    }

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
            now,
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
    headers: axum::http::HeaderMap,
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

    // Layer-2 bucket keyed by the presented `state`
    // (docs/components/backend/authenticator/DESIGN.md §4.4): caps how
    // often one state value can drive the code-exchange path. Fail open on a
    // Redis error — the coarse gateway layer still guards, and the state
    // lookup below fails closed anyway.
    if !rate_limit_or_open(&state, "callback", &oidc_state, {
        crate::ratelimit::BucketSpec {
            burst: state.cfg.rate_limit.callback_burst,
            per_minute: state.cfg.rate_limit.callback_per_minute,
        }
    })
    .await
    {
        return too_many_requests("callback_rate_limited", 10);
    }

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

    // A session without a tenant is unusable — the gateway JWT's `tenant_id` is
    // the sole tenant authority and downstream fails closed without it. If the
    // id_token named no tenant and no `default_tenant_id` is configured, deny
    // the login here rather than minting a dead session (and never issue a
    // tenant-less JWT — enforced again at the signing chokepoint).
    if idp.identity.tenant_id.trim().is_empty() {
        tracing::warn!(
            target: "audit",
            event = "login_denied_no_tenant",
            idp_sub = %idp.identity.sub,
            email = %idp.identity.email,
            "login denied: id_token carried no tenant and no default_tenant_id is set"
        );
        return PersonError::permission_denied()
            .with_reason("tenant_unresolved")
            .create()
            .into_response();
    }

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
            let client = ClientInfo::from_headers(&headers);
            state.audit.emit(AuditEvent {
                action: "login",
                outcome: "failure",
                tenant_id: idp.identity.tenant_id.clone(),
                actor_person_id: String::new(),
                actor_ip: client.ip,
                actor_user_agent: client.user_agent,
                correlation_id: correlation_id(&headers),
                resource_type: "session",
                resource_id: String::new(),
                details: serde_json::json!({
                    "reason": "unknown_person",
                    "idp_sub": idp.identity.sub,
                }),
            });
            return PersonError::permission_denied()
                .with_reason("unknown_person")
                .create()
                .into_response();
        }
        Err(e) => return internal_problem("person_resolution", &e),
    };

    // `return_to` was sanitized at login time and stored with the login state.
    let return_to = login_state.return_to.clone();
    let client = ClientInfo::from_headers(&headers);
    match mint_and_store_session(&state, &idp, &resolution, &client).await {
        Ok((session_id, token)) => {
            state.audit.emit(AuditEvent {
                action: "login",
                outcome: "success",
                tenant_id: resolution.tenant_id.clone(),
                actor_person_id: resolution.person_id.clone(),
                actor_ip: client.ip,
                actor_user_agent: client.user_agent,
                correlation_id: correlation_id(&headers),
                resource_type: "session",
                resource_id: session_id,
                details: serde_json::json!({ "idp_sub": idp.identity.sub }),
            });
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

/// Client attribution captured at login for the session list (PRD 5.9):
/// the User-Agent and the client IP as the gateway saw it (first
/// `X-Forwarded-For` hop; nginx guards the header with `set_real_ip_from`).
struct ClientInfo {
    user_agent: String,
    ip: String,
}

impl ClientInfo {
    fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        let header = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
        };
        // Attribution only (never authorization) — cap length so a hostile
        // header can't bloat the session record.
        let mut user_agent = header("user-agent").to_owned();
        user_agent.truncate(256);
        let ip = header("x-forwarded-for")
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        Self { user_agent, ip }
    }
}

/// The gateway-minted request correlation id (edge Lua, `X-Correlation-Id`).
fn correlation_id(headers: &axum::http::HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// An audit event attributed to a live session record.
fn session_audit(
    action: &'static str,
    outcome: &'static str,
    record: &SessionRecord,
    resource_id: &str,
    correlation_id: String,
    details: serde_json::Value,
) -> AuditEvent {
    AuditEvent {
        action,
        outcome,
        tenant_id: record.tenant_id.clone(),
        actor_person_id: record.person_id.clone(),
        actor_ip: record.ip.clone(),
        actor_user_agent: record.user_agent.clone(),
        correlation_id,
        resource_type: "session",
        resource_id: resource_id.to_owned(),
        details,
    }
}

/// Build claims, sign the linked JWT, and persist the session in one pipeline.
/// Returns the cookie token.
async fn mint_and_store_session(
    state: &AppState,
    idp: &crate::oidc::AuthenticatedIdp,
    resolution: &PersonResolution,
    client: &ClientInfo,
) -> anyhow::Result<(String, String)> {
    let now = now_secs();
    let cfg = &state.cfg;
    let expires_at = now + cfg.session_ttl_seconds;
    let mut absolute_expires_at = now + cfg.session_absolute_lifetime_seconds;

    // No refresh token → the refresher can't keep the IdP vouching for the
    // user. `strict` (default) caps the session at the IdP access-token
    // lifetime; `login_only` lets it live to the absolute cap (killed only by
    // back-channel logout / manual revoke). (PRD 5.12 policy knob.)
    if idp.refresh_token.is_none()
        && cfg.idp.no_refresh_token_policy == crate::config::NoRefreshTokenPolicy::Strict
        && let Some(ttl) = idp.expires_in
    {
        absolute_expires_at = absolute_expires_at.min(now + ttl);
        tracing::debug!(
            cap = absolute_expires_at,
            "no IdP refresh token: strict policy caps the session at the IdP token lifetime"
        );
    }
    let expires_at = expires_at.min(absolute_expires_at);

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

    // Schedule the background refresh — only when there is a grant to refresh
    // (no refresh token → the policy above already decided the lifetime).
    // Due-times are jittered at write so sessions never herd (G5).
    let refresh_due_at = if cfg.idp.refresh_enabled && idp.refresh_token.is_some() {
        idp.expires_in.map(|ttl| {
            let base = now + ttl.saturating_sub(cfg.idp.refresh_safety_margin_seconds);
            base.saturating_add_signed(jitter_seconds(cfg.idp.refresh_due_jitter_seconds))
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
        user_agent: client.user_agent.clone(),
        ip: client.ip.clone(),
        csrf_token,
        current_token: token.clone(),
    };

    state
        .sessions
        .create_session(&NewSession {
            session_id: session_id.clone(),
            token: token.clone(),
            record,
            jwt,
            jwt_reissue_after_seconds: cfg.jwt_reissue_after_seconds,
            refresh_due_at,
        })
        .await?;

    Ok((session_id, token))
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

    let refresh_at = refresh_at_for(&state.cfg, record.expires_at);

    let body = serde_json::json!({
        "user": record.person_id,
        "email": record.email,
        "tenant_id": record.tenant_id,
        "roles": record.roles,
        "expires_at": record.expires_at,
        "refresh_at": refresh_at,
        "csrf_token": record.csrf_token,
    })
    .to_string();
    json_ok(body)
}

// ── /auth/csrf ───────────────────────────────────────────────────────────────

/// Issue the CSRF token bound to the current session (PRD 5.11). The SPA sends
/// it back as `X-CSRF-Token` on state-changing `/auth/*` requests; `/auth/me`
/// echoes the same value so a page load primes both timers in one call.
pub async fn csrf(Extension(state): Extension<Arc<AppState>>, jar: CookieJar) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated();
    };
    let (_, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated(),
        Err(e) => return internal_problem("session_store", &e),
    };
    let now = now_secs();
    if record.expires_at <= now || record.absolute_expires_at <= now {
        return unauthenticated();
    }
    json_ok(serde_json::json!({ "csrf_token": record.csrf_token }).to_string())
}

// ── /auth/refresh ────────────────────────────────────────────────────────────

/// Rotate the session credential and extend the session (PRD 5.4, G10 model):
/// new CSPRNG token mapping, old mapping demoted to the grace TTL, session
/// `expires_at` advanced to `min(now + ttl, absolute_cap)` — one pipeline. The
/// stable `session_id` and the linked JWT are untouched. A stale token still
/// inside the grace window resolves to the same session and is answered with
/// the current state, no second rotation; past grace → 401 + clear cookie.
pub async fn refresh(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated_clear_cookie(jar);
    };
    let (session_id, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated_clear_cookie(jar),
        Err(e) => return internal_problem("session_store", &e),
    };
    let now = now_secs();
    if record.expires_at <= now || record.absolute_expires_at <= now {
        return unauthenticated_clear_cookie(jar);
    }

    // Layer-2 bucket keyed by the stable session (DESIGN §4.4 — never IP:
    // corporate NAT makes per-IP keys wrong at the precise layer).
    if !rate_limit_or_open(&state, "refresh", &session_id, {
        crate::ratelimit::BucketSpec {
            burst: state.cfg.rate_limit.refresh_burst,
            per_minute: state.cfg.rate_limit.refresh_per_minute,
        }
    })
    .await
    {
        return too_many_requests("refresh_rate_limited", 10);
    }

    // Grace path: the presented token has already been rotated past (the old
    // mapping lives out its grace TTL). Answer with the current state and the
    // current cookie value — rotating again would burn the grace guarantee.
    if record.current_token != token {
        tracing::debug!(session_id = %session_id, "refresh within rotation grace: no re-rotation");
        return refresh_ok(&state, jar, &record.current_token, record.expires_at, now);
    }

    let new_token = csprng_token();
    let new_expires_at = (now + state.cfg.session_ttl_seconds).min(record.absolute_expires_at);
    let rotated = match state
        .sessions
        .rotate_session(
            &session_id,
            &record,
            &token,
            &new_token,
            new_expires_at,
            state.cfg.refresh_grace_ms,
        )
        .await
    {
        Ok(rotated) => rotated,
        Err(e) => return internal_problem("rotate_session", &e),
    };
    if !rotated {
        // Lost the compare-and-swap: a concurrent refresh already rotated this
        // credential (multi-tab). Answer the grace path with the now-current
        // credential rather than minting a second one. Re-load to read it.
        tracing::debug!(session_id = %session_id, "refresh lost the rotation CAS: answering grace path");
        return match state.sessions.load_session(&session_id).await {
            Ok(Some(current)) => {
                refresh_ok(&state, jar, &current.current_token, current.expires_at, now)
            }
            Ok(None) => unauthenticated_clear_cookie(jar),
            Err(e) => internal_problem("session_store", &e),
        };
    }
    tracing::debug!(session_id = %session_id, expires_at = new_expires_at, "session refreshed (credential rotated)");
    state.audit.emit(session_audit(
        "session_refresh",
        "success",
        &record,
        &session_id,
        correlation_id(&headers),
        serde_json::json!({ "expires_at": new_expires_at }),
    ));
    refresh_ok(&state, jar, &new_token, new_expires_at, now)
}

/// `200 {expires_at, refresh_at}` + the (re-)issued session cookie. `Max-Age`
/// is the session's actual remaining life, so the cookie can never outlive the
/// absolute cap.
fn refresh_ok(
    state: &AppState,
    jar: CookieJar,
    token: &str,
    expires_at: u64,
    now: u64,
) -> Response {
    let body = serde_json::json!({
        "expires_at": expires_at,
        "refresh_at": refresh_at_for(&state.cfg, expires_at),
    })
    .to_string();
    let jar = jar.add(cookie::session_cookie(
        token,
        expires_at.saturating_sub(now),
    ));
    (jar, json_ok(body)).into_response()
}

// ── /auth/logout ─────────────────────────────────────────────────────────────

/// Revoke the session, clear the cookie, and return the RP-logout URL.
pub async fn logout(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: axum::http::HeaderMap,
) -> Response {
    let mut rp_logout_url = serde_json::Value::Null;

    if let Some(token) = cookie::read(&jar)
        && let Ok(Some((session_id, record))) = state.sessions.resolve_by_token(&token).await
    {
        let _ = state.sessions.revoke_session(&session_id).await;
        tracing::info!(session_id = %session_id, "logout: session revoked");
        state.audit.emit(session_audit(
            "logout",
            "success",
            &record,
            &session_id,
            correlation_id(&headers),
            serde_json::json!({}),
        ));
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

// ── /auth/sessions (PRD 5.9) ─────────────────────────────────────────────────

/// List the caller's active sessions from the per-user index (score > now):
/// created_at, expires_at, user_agent, ip, and a `current` flag.
pub async fn sessions_list(Extension(state): Extension<Arc<AppState>>, jar: CookieJar) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated();
    };
    let (current_id, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated(),
        Err(e) => return internal_problem("session_store", &e),
    };
    let now = now_secs();
    if record.expires_at <= now || record.absolute_expires_at <= now {
        return unauthenticated();
    }

    let sessions = match state
        .sessions
        .list_user_sessions(&record.person_id, now)
        .await
    {
        Ok(s) => s,
        Err(e) => return internal_problem("session_list", &e),
    };
    let items: Vec<serde_json::Value> = sessions
        .iter()
        .map(|(sid, r)| {
            serde_json::json!({
                "session_id": sid,
                "created_at": r.created_at,
                "expires_at": r.expires_at,
                "user_agent": r.user_agent,
                "ip": r.ip,
                "current": *sid == current_id,
            })
        })
        .collect();
    json_ok(serde_json::json!({ "sessions": items }).to_string())
}

/// Revoke one of the caller's sessions by id. A session that does not exist or
/// belongs to someone else is answered 404 (no existence oracle). Revoking the
/// current session also clears the cookie.
pub async fn sessions_revoke_one(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: axum::http::HeaderMap,
    axum::extract::Path(target_id): axum::extract::Path<String>,
) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated();
    };
    let (current_id, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated(),
        Err(e) => return internal_problem("session_store", &e),
    };

    let target = match state.sessions.load_session(&target_id).await {
        Ok(t) => t,
        Err(e) => return internal_problem("session_load", &e),
    };
    let owned = target
        .as_ref()
        .is_some_and(|t| t.person_id == record.person_id);
    if !owned {
        return not_found(&target_id);
    }
    if let Err(e) = state.sessions.revoke_session(&target_id).await {
        return internal_problem("session_revoke", &e);
    }
    tracing::info!(
        target: "audit",
        event = "session_revoked",
        session_id = %target_id,
        person_id = %record.person_id,
        by = "self",
        "session revoked"
    );
    state.audit.emit(session_audit(
        "session_revoke",
        "success",
        &record,
        &target_id,
        correlation_id(&headers),
        serde_json::json!({ "by": "self", "scope": "single" }),
    ));

    let resp = json_ok(serde_json::json!({ "revoked": 1 }).to_string());
    if target_id == current_id {
        return (jar.add(cookie::clear_cookie()), resp).into_response();
    }
    resp
}

/// Revoke every session of the current user ("log out everywhere") and clear
/// the cookie.
pub async fn sessions_revoke_all(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(token) = cookie::read(&jar) else {
        return unauthenticated();
    };
    let (_, record) = match state.sessions.resolve_by_token(&token).await {
        Ok(Some(r)) => r,
        Ok(None) => return unauthenticated(),
        Err(e) => return internal_problem("session_store", &e),
    };

    let revoked = match state.sessions.revoke_user_sessions(&record.person_id).await {
        Ok(n) => n,
        Err(e) => return internal_problem("session_revoke_all", &e),
    };
    tracing::info!(
        target: "audit",
        event = "sessions_revoked_all",
        person_id = %record.person_id,
        revoked,
        by = "self",
        "all sessions revoked"
    );
    state.audit.emit(session_audit(
        "session_revoke",
        "success",
        &record,
        &record.person_id.clone(),
        correlation_id(&headers),
        serde_json::json!({ "by": "self", "scope": "all", "revoked": revoked }),
    ));
    let resp = json_ok(serde_json::json!({ "revoked": revoked }).to_string());
    (jar.add(cookie::clear_cookie()), resp).into_response()
}

/// Admin/service revoke-by-user (PRD 5.9 "admin variant"): the host authn
/// pipeline has already verified the gateway JWT and built the
/// [`SecurityContext`]; this handler enforces the authorized role
/// (`admin_revoke_roles`) and delegates to the SDK contract
/// (`AuthenticatorClientV1::revoke_user_sessions`) — the same lever the
/// future permissions service pulls on grant changes (DD-AUTH-07).
pub async fn admin_revoke_user_sessions(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<toolkit_security::SecurityContext>,
    headers: axum::http::HeaderMap,
    axum::extract::Path(person_id): axum::extract::Path<Uuid>,
) -> Response {
    let allowed = ctx
        .token_scopes()
        .iter()
        .any(|scope| state.cfg.admin_revoke_roles.iter().any(|r| r == scope));
    if !allowed {
        tracing::warn!(
            target: "audit",
            event = "admin_session_revoke_denied",
            subject = %ctx.subject_id(),
            subject_type = ctx.subject_type().unwrap_or(""),
            person_id = %person_id,
            "admin session revoke denied: missing authorized role"
        );
        return SessionError::permission_denied()
            .with_reason("missing_authorized_role")
            .create()
            .into_response();
    }

    match state
        .authn_client
        .revoke_user_sessions(&person_id.to_string())
        .await
    {
        Ok(revoked) => {
            tracing::info!(
                target: "audit",
                event = "sessions_revoked_all",
                person_id = %person_id,
                revoked,
                by = "admin",
                subject = %ctx.subject_id(),
                subject_type = ctx.subject_type().unwrap_or(""),
                "all sessions revoked (admin)"
            );
            state.audit.emit(AuditEvent {
                action: "session_revoke",
                outcome: "success",
                tenant_id: ctx.subject_tenant_id().to_string(),
                actor_person_id: ctx.subject_id().to_string(),
                actor_ip: String::new(),
                actor_user_agent: String::new(),
                correlation_id: correlation_id(&headers),
                resource_type: "session",
                resource_id: person_id.to_string(),
                details: serde_json::json!({
                    "by": "admin",
                    "scope": "all",
                    "revoked": revoked,
                    "subject_type": ctx.subject_type().unwrap_or(""),
                }),
            });
            json_ok(serde_json::json!({ "revoked": revoked }).to_string())
        }
        Err(e) => e.into_response(),
    }
}

// ── /auth/oidc/back-channel-logout (PRD 5.10) ────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BackChannelForm {
    #[serde(default)]
    logout_token: Option<String>,
}

/// Receive an IdP back-channel `logout_token` (form-encoded, OIDC BCL §2.5):
/// validate it against the configured issuer's JWKS, replay-guard its `jti`
/// (one-shot — a replayed delivery answers 200 without another revoke), then
/// revoke the targeted sessions: by `(iss, sid)` via the sid index, or — the
/// documented sub-only fallback — everything for that user.
// Linear validate → replay-guard → resolve → revoke flow with per-step error
// mapping; splitting it would scatter the sequence without making it clearer.
#[allow(clippy::too_many_lines)]
pub async fn back_channel_logout(
    Extension(state): Extension<Arc<AppState>>,
    axum::extract::Form(form): axum::extract::Form<BackChannelForm>,
) -> Response {
    let Some(raw) = form.logout_token.as_deref().filter(|t| !t.is_empty()) else {
        return OidcError::invalid_argument()
            .with_field_violation("logout_token", "missing logout_token", "MISSING")
            .create()
            .into_response();
    };

    // The IdP's keys — fetched per call (cold path, picks up rotation).
    let jwks = match state.oidc.idp_jwks().await {
        Ok(jwks) => jwks,
        Err(e) => {
            tracing::warn!(
                error = format!("{e:#}"),
                "back-channel: IdP JWKS unavailable"
            );
            return toolkit_canonical_errors::CanonicalError::service_unavailable()
                .with_detail("IdP JWKS unavailable")
                .create()
                .into_response();
        }
    };

    let now = now_secs();
    let cfg = &state.cfg;
    let claims = match crate::backchannel::validate_logout_token(
        &jwks,
        raw,
        state.oidc.issuer(),
        state.oidc.client_id(),
        now,
        cfg.backchannel_clock_skew_seconds,
        cfg.backchannel_token_max_age_seconds,
    ) {
        Ok(c) => c,
        Err(reason) => {
            tracing::warn!(reason, "back-channel: logout_token rejected");
            return OidcError::invalid_argument()
                .with_field_violation("logout_token", reason, "INVALID_LOGOUT_TOKEN")
                .create()
                .into_response();
        }
    };

    // One-shot per (iss, jti): a replay answers 200 idempotently, no revoke.
    let ttl = crate::backchannel::replay_guard_ttl(
        claims.iat,
        now,
        cfg.backchannel_clock_skew_seconds,
        cfg.backchannel_token_max_age_seconds,
    );
    match state
        .sessions
        .guard_logout_jti(state.oidc.issuer(), &claims.jti, ttl)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(jti = %claims.jti, "back-channel: replayed logout_token (idempotent 200)");
            return no_content_ok();
        }
        Err(e) => return internal_problem("logout_jti_guard", &e),
    }

    let result = match &claims.sid {
        Some(idp_sid) => revoke_by_sid_index(&state, idp_sid).await,
        None => match &claims.sub {
            Some(sub) => revoke_by_sub_fallback(&state, sub).await,
            None => unreachable!("validator requires sub or sid"),
        },
    };
    match result {
        Ok(revoked) => {
            tracing::info!(
                target: "audit",
                event = "back_channel_logout",
                sid = claims.sid.as_deref().unwrap_or(""),
                sub = claims.sub.as_deref().unwrap_or(""),
                revoked,
                "back-channel logout processed"
            );
            state.audit.emit(AuditEvent {
                action: "back_channel_logout",
                outcome: "success",
                tenant_id: String::new(),
                actor_person_id: String::new(),
                actor_ip: String::new(),
                actor_user_agent: String::new(),
                correlation_id: String::new(),
                resource_type: "session",
                resource_id: claims
                    .sid
                    .clone()
                    .or(claims.sub.clone())
                    .unwrap_or_default(),
                details: serde_json::json!({
                    "revoked": revoked,
                    "sub_only_fallback": claims.sid.is_none(),
                }),
            });
            no_content_ok()
        }
        Err(e) => {
            // Release the replay guard so the IdP's retry actually revokes
            // (review M1) — the claim was consumed above, but the revoke did
            // not happen. Revoke is idempotent, so re-processing is safe.
            if let Err(re) = state
                .sessions
                .release_logout_jti(state.oidc.issuer(), &claims.jti)
                .await
            {
                tracing::warn!(error = %re, "back-channel: failed to release jti guard after revoke error");
            }
            internal_problem("back_channel_revoke", &e)
        }
    }
}

/// Revoke every session indexed under the token's `(iss, sid)`.
async fn revoke_by_sid_index(state: &AppState, idp_sid: &str) -> anyhow::Result<u64> {
    let session_ids = state
        .sessions
        .sessions_by_idp_sid(state.oidc.issuer(), idp_sid)
        .await?;
    let mut revoked = 0u64;
    for sid in &session_ids {
        if state.sessions.revoke_session(sid).await? {
            revoked += 1;
        }
    }
    Ok(revoked)
}

/// The sub-only fallback (spec-compliant, blast radius documented): revoke
/// EVERYTHING for the users behind `(iss, sub)` — with the operator-facing
/// log line the runbook calls out, so a misconfigured IdP that omits `sid`
/// is visible, not silent.
async fn revoke_by_sub_fallback(state: &AppState, idp_sub: &str) -> anyhow::Result<u64> {
    let session_ids = state
        .sessions
        .sessions_by_idp_sub(state.oidc.issuer(), idp_sub)
        .await?;
    // Resolve the distinct person(s) behind those sessions, then run the
    // standard revoke-everything pipeline per person.
    let mut persons: Vec<String> = Vec::new();
    for sid in &session_ids {
        if let Some(record) = state.sessions.load_session(sid).await?
            && !persons.contains(&record.person_id)
        {
            persons.push(record.person_id);
        }
    }
    let mut revoked = 0u64;
    for person_id in &persons {
        tracing::warn!(
            target: "audit",
            event = "back_channel_logout_sub_fallback",
            idp_sub,
            person_id = %person_id,
            "back-channel logout_token carried no sid: revoking ALL sessions for this user \
             (OIDC-compliant fallback — configure the IdP to emit sid to narrow the blast radius)"
        );
        revoked += state.sessions.revoke_user_sessions(person_id).await?;
    }
    Ok(revoked)
}

/// 200 with an empty body and `no-store` (OIDC BCL §2.7 — the response must
/// not be cached).
fn no_content_ok() -> Response {
    build_response(
        StatusCode::OK,
        vec![(CACHE_CONTROL.clone(), "no-store".to_owned())],
        Body::empty(),
    )
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

/// The server-supplied refresh moment: `expires_at − margin ± jitter/2` (G8 —
/// the deliberately big jitter spreads NAT'd-office refresh waves into a
/// uniform trickle and keeps an attacker from aligning to the rotation grace
/// window). Re-jittered on every call.
fn refresh_at_for(cfg: &crate::config::AuthenticatorConfig, expires_at: u64) -> u64 {
    expires_at
        .saturating_sub(cfg.session_refresh_safety_margin_seconds)
        .saturating_add_signed(jitter_seconds(cfg.refresh_jitter_seconds / 2))
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

/// Take a token from a layer-2 bucket; a Redis failure fails OPEN (`true`) —
/// the gateway's coarse layer still guards, and turning a Redis blip into a
/// 429 storm would be a self-inflicted outage. (Auth itself always fails
/// closed; this is only the limiter.)
async fn rate_limit_or_open(
    state: &AppState,
    class: &str,
    key: &str,
    spec: crate::ratelimit::BucketSpec,
) -> bool {
    match state
        .sessions
        .rate_limit_take(class, key, spec, now_secs())
        .await
    {
        Ok(allowed) => {
            if !allowed {
                tracing::warn!(class, "layer-2 rate limit tripped");
            }
            allowed
        }
        Err(e) => {
            tracing::warn!(class, error = %e, "rate limiter unavailable: failing open");
            true
        }
    }
}

/// 429 with a quota violation + retry hint (RFC 9457 problem body).
fn too_many_requests(subject: &str, retry_after_seconds: u64) -> Response {
    SessionError::resource_exhausted("rate limited")
        .with_quota_violation(subject, "too many requests")
        .with_quota_violation_retry_after_seconds(retry_after_seconds)
        .create()
        .into_response()
}

/// 404 that does not distinguish "absent" from "not yours" (no existence oracle).
fn not_found(resource: &str) -> Response {
    SessionError::not_found("session not found")
        .with_resource(resource)
        .create()
        .into_response()
}

/// 401 that also clears the session cookie — for `/auth/refresh`, where a dead
/// credential must not linger in the browser (PRD 5.4 case 1).
fn unauthenticated_clear_cookie(jar: CookieJar) -> Response {
    let jar = jar.add(cookie::clear_cookie());
    (jar, unauthenticated()).into_response()
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
    fn refresh_at_stays_inside_the_jitter_window() {
        // margin 90, full jitter 120 (±60): refresh_at ∈ [exp−150, exp−30] —
        // the late edge still leaves ≥30 s of session life (G8).
        let cfg = crate::config::AuthenticatorConfig::default();
        let expires_at = 10_000;
        for _ in 0..200 {
            let at = refresh_at_for(&cfg, expires_at);
            assert!((expires_at - 150..=expires_at - 30).contains(&at), "{at}");
        }
    }

    #[test]
    fn jwt_exp_reads_payload_without_verification() {
        // header.payload.sig with payload = {"exp": 4000000000}
        let payload = B64.encode(br#"{"exp":4000000000}"#);
        let token = format!("aGVhZGVy.{payload}.c2ln");
        assert_eq!(jwt_exp(&token), Some(4_000_000_000));
    }
}
