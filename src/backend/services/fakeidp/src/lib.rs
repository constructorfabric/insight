//! fakeidp — a deliberately silly fake OIDC provider for dev and e2e.
//!
//! It implements *just enough* OIDC (discovery, JWKS, an instant `/authorize`
//! with no login screen, a `/token` endpoint with authorization-code + PKCE and
//! rotating one-time-use refresh tokens, and RP-initiated logout) to drive the
//! authenticator's real code path — plus a set of `/_control/*` hooks that an
//! off-the-shelf IdP can't give us: forcing `invalid_grant` on refresh, firing a
//! back-channel `logout_token`, and simulating token-endpoint outages.
//!
//! It is NOT a toolkit gear and never ships in a production image. See
//! `cf/NGINX_BFF.md` §10 G6 for the decision and §4.1 for the flows it exercises.
//!
//! Everything lives in one process, in memory, behind one mutex. That is the
//! point — "as silly as it can be".
//!
//! The binary (`src/main.rs`) is a thin wrapper over [`run`]; the guts live
//! here so the integration test can build the same [`app`] router in-process.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    Json, Router,
    extract::{Form, Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rand::RngCore;
use rsa::RsaPrivateKey;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const KID: &str = "fakeidp-key-1";

/// Baked default users, used when `FAKEIDP_USERS` is not set (so `cargo run`
/// and the container both work with zero setup).
const DEFAULT_USERS_YAML: &str = include_str!("../users.yaml");

// ─── Config ────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct Config {
    pub issuer: String,
    pub bind: String,
    pub token_ttl: u64,
    /// Back-channel logout endpoint of the authenticator (the RP). Only used by
    /// `POST /_control/backchannel/{email}`.
    pub backchannel_url: Option<String>,
    /// `aud` used for the back-channel `logout_token` (the code flow uses the
    /// per-request `client_id` instead).
    pub default_aud: String,
}

impl Config {
    pub fn from_env() -> Self {
        let issuer =
            std::env::var("FAKEIDP_ISSUER").unwrap_or_else(|_| "http://localhost:8084".into());
        Self {
            issuer,
            bind: std::env::var("FAKEIDP_BIND").unwrap_or_else(|_| "0.0.0.0:8084".into()),
            token_ttl: std::env::var("FAKEIDP_TOKEN_TTL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
            backchannel_url: std::env::var("FAKEIDP_BACKCHANNEL_URL")
                .ok()
                .filter(|s| !s.is_empty()),
            default_aud: std::env::var("FAKEIDP_DEFAULT_AUD")
                .unwrap_or_else(|_| "authenticator".into()),
        }
    }
}

// ─── Users ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct User {
    pub email: String,
    pub name: String,
    pub sub: String,
    pub sid: String,
    /// The user's single tenant (single-tenant token contract, EPIC #1583).
    #[serde(default)]
    pub tenant_id: String,
}

#[derive(Deserialize)]
struct UsersFile {
    users: Vec<User>,
}

/// Load test users from `FAKEIDP_USERS` (a path) or fall back to the baked
/// `users.yaml`. Panics on malformed input — this is a test binary; loud is fine.
///
/// If `FAKEIDP_DEV_USER_EMAIL` is set, it overrides the *first* user's email.
/// Compose wires this from the wizard's `VITE_DEV_USER_EMAIL`, so the default
/// login always matches the dev person the seeder wrote into identity — even
/// when the operator picked a non-default dev email.
pub fn load_users() -> Vec<User> {
    let raw = match std::env::var("FAKEIDP_USERS") {
        Ok(path) => {
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("FAKEIDP_USERS={path}: {e}"))
        }
        Err(_) => DEFAULT_USERS_YAML.to_string(),
    };
    let parsed: UsersFile = serde_yaml::from_str(&raw).expect("users.yaml is not valid YAML");
    let mut users = parsed.users;
    assert!(
        !users.is_empty(),
        "users.yaml must define at least one user"
    );

    if let Ok(email) = std::env::var("FAKEIDP_DEV_USER_EMAIL")
        && !email.is_empty()
    {
        users[0].email = email;
    }
    users
}

// ─── Mutable state ─────────────────────────────────────────────────────────

/// A one-time authorization code, bound to the login context so `/token` can
/// validate PKCE and mint the right id_token.
struct AuthCode {
    email: String,
    nonce: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    client_id: String,
    used: bool,
}

/// A refresh token. Rotation flips `active` to false on the old token so reuse
/// is detectable (→ `invalid_grant`) rather than silently accepted.
struct RefreshEntry {
    email: String,
    sid: String,
    /// The `client_id` from the original authorize request, so rotated ID
    /// tokens keep the same `aud` an RP with a non-default client would expect.
    client_id: String,
    active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outage {
    Off,
    ServerError,
    Timeout,
}

#[derive(Default)]
struct Mutable {
    codes: HashMap<String, AuthCode>,
    refresh_tokens: HashMap<String, RefreshEntry>,
    revoked: HashSet<String>,
    outage: Option<Outage>,
}

pub struct AppState {
    config: Config,
    users: Vec<User>,
    signing_key: EncodingKey,
    jwks: Value,
    inner: Mutex<Mutable>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn new(config: Config, users: Vec<User>) -> Self {
        let (signing_key, jwks) = generate_signing_material();
        Self {
            config,
            users,
            signing_key,
            jwks,
            inner: Mutex::new(Mutable::default()),
        }
    }

    fn user_by_email(&self, email: &str) -> Option<&User> {
        self.users.iter().find(|u| u.email == email)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Mutable> {
        self.inner.lock().expect("state mutex poisoned")
    }
}

// ─── Crypto helpers ──────────────────────────────────────────────────────

/// Generate a fresh RS256 keypair at startup and return the signing key plus a
/// matching JWKS derived from it, so `/jwks` and the signer can never drift.
///
/// Nothing is persisted: the key lives only in this process. It fakes the
/// *customer* IdP (whose id_tokens the authenticator verifies via our JWKS),
/// not our own gateway JWT, and consumers fetch `/jwks` at runtime — so a fresh
/// key per boot is exactly right and keeps key material out of the repo.
fn generate_signing_material() -> (EncodingKey, Value) {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate RSA key");
    let pem = priv_key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("encode generated key as PKCS#8 PEM");
    let signing_key =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("generated key is valid RSA PEM");

    let pub_key = priv_key.to_public_key();
    let n = URL_SAFE_NO_PAD.encode(pub_key.n().to_bytes_be());
    let e = URL_SAFE_NO_PAD.encode(pub_key.e().to_bytes_be());
    let jwks = json!({
        "keys": [{
            "kty": "RSA",
            "use": "sig",
            "alg": "RS256",
            "kid": KID,
            "n": n,
            "e": e,
        }]
    });
    (signing_key, jwks)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock is before 1970")
        .as_secs()
}

/// Opaque, unguessable token (authorization code / access token / refresh token).
fn opaque() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// RFC 7636 PKCE verification. Returns Ok(()) when the code carried no
/// challenge (PKCE not used), or when the verifier matches.
fn verify_pkce(
    challenge: &Option<String>,
    method: &Option<String>,
    verifier: &Option<String>,
) -> Result<(), &'static str> {
    let Some(challenge) = challenge else {
        return Ok(());
    };
    let verifier = verifier.as_deref().ok_or("code_verifier required")?;
    let method = method.as_deref().unwrap_or("plain");
    let computed = match method {
        "plain" => verifier.to_string(),
        "S256" => {
            let digest = Sha256::digest(verifier.as_bytes());
            URL_SAFE_NO_PAD.encode(digest)
        }
        _ => return Err("unsupported code_challenge_method"),
    };
    if computed == *challenge {
        Ok(())
    } else {
        Err("PKCE verification failed")
    }
}

// ─── Token minting ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct IdTokenClaims<'a> {
    iss: &'a str,
    sub: &'a str,
    aud: &'a str,
    exp: u64,
    iat: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    email: &'a str,
    name: &'a str,
    sid: &'a str,
    /// The single tenant from users.yaml (one and only one tenant per token).
    /// Always emitted (possibly empty) for predictability.
    tenant_id: &'a str,
}

fn sign_id_token(state: &AppState, user: &User, aud: &str, nonce: Option<String>) -> String {
    let iat = now();
    let claims = IdTokenClaims {
        iss: &state.config.issuer,
        sub: &user.sub,
        aud,
        exp: iat + state.config.token_ttl,
        iat,
        nonce,
        email: &user.email,
        name: &user.name,
        sid: &user.sid,
        tenant_id: &user.tenant_id,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(&header, &claims, &state.signing_key).expect("id_token signing")
}

#[derive(Serialize)]
struct LogoutTokenClaims<'a> {
    iss: &'a str,
    aud: &'a str,
    iat: u64,
    jti: String,
    sub: &'a str,
    sid: &'a str,
    events: Value,
}

fn sign_logout_token(state: &AppState, user: &User) -> String {
    let claims = LogoutTokenClaims {
        iss: &state.config.issuer,
        aud: &state.config.default_aud,
        iat: now(),
        jti: uuid::Uuid::now_v7().to_string(),
        sub: &user.sub,
        sid: &user.sid,
        events: json!({ "http://schemas.openid.net/event/backchannel-logout": {} }),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(KID.to_string());
    encode(&header, &claims, &state.signing_key).expect("logout_token signing")
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    refresh_token: String,
    id_token: String,
    scope: String,
}

fn oauth_error(status: StatusCode, error: &str, desc: &str) -> Response {
    (
        status,
        Json(json!({ "error": error, "error_description": desc })),
    )
        .into_response()
}

// ─── OIDC endpoints ──────────────────────────────────────────────────────

async fn discovery(State(state): State<Shared>) -> Json<Value> {
    let iss = &state.config.issuer;
    Json(json!({
        "issuer": iss,
        "authorization_endpoint": format!("{iss}/authorize"),
        "token_endpoint": format!("{iss}/token"),
        "jwks_uri": format!("{iss}/jwks"),
        "end_session_endpoint": format!("{iss}/end_session"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "scopes_supported": ["openid", "email", "profile", "offline_access"],
        "code_challenge_methods_supported": ["S256", "plain"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post"],
    }))
}

async fn jwks(State(state): State<Shared>) -> Json<Value> {
    Json(state.jwks.clone())
}

#[derive(Deserialize)]
struct AuthorizeParams {
    client_id: Option<String>,
    redirect_uri: String,
    state: Option<String>,
    nonce: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    /// Which test user to log in as. Defaults to the first user in users.yaml.
    user: Option<String>,
}

/// No login screen: pick the requested (or default) user, mint a one-time code
/// bound to (user, nonce, PKCE challenge), and 302 straight back to the RP.
async fn authorize(State(state): State<Shared>, Query(params): Query<AuthorizeParams>) -> Response {
    let email = match &params.user {
        Some(email) => email.clone(),
        None => state.users[0].email.clone(),
    };
    if state.user_by_email(&email).is_none() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "access_denied",
            &format!("unknown test user: {email}"),
        );
    }

    let code = opaque();
    state.lock().codes.insert(
        code.clone(),
        AuthCode {
            email,
            nonce: params.nonce,
            code_challenge: params.code_challenge,
            code_challenge_method: params.code_challenge_method,
            client_id: params
                .client_id
                .unwrap_or_else(|| state.config.default_aud.clone()),
            used: false,
        },
    );

    let sep = if params.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut location = format!("{}{}code={}", params.redirect_uri, sep, urlencode(&code));
    if let Some(st) = &params.state {
        location.push_str(&format!("&state={}", urlencode(st)));
    }
    redirect(&location)
}

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: String,
    // authorization_code grant
    code: Option<String>,
    code_verifier: Option<String>,
    // refresh_token grant
    refresh_token: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

async fn token(State(state): State<Shared>, Form(req): Form<TokenRequest>) -> Response {
    // Outage simulation happens before any grant logic — the token endpoint
    // is the thing we want to misbehave (G6 / transient-vs-definitive test).
    // Read the mode into a local so the (non-Send) MutexGuard is dropped
    // before the `.await` in the timeout arm.
    let outage = state.lock().outage.unwrap_or(Outage::Off);
    match outage {
        Outage::Off => {}
        Outage::ServerError => {
            return oauth_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "temporarily_unavailable",
                "simulated outage",
            );
        }
        Outage::Timeout => {
            tokio::time::sleep(Duration::from_secs(60)).await;
            return oauth_error(
                StatusCode::GATEWAY_TIMEOUT,
                "temporarily_unavailable",
                "simulated timeout",
            );
        }
    }

    let scope = req
        .scope
        .clone()
        .unwrap_or_else(|| "openid email profile".into());
    match req.grant_type.as_str() {
        "authorization_code" => token_code_grant(&state, &req, &scope),
        "refresh_token" => token_refresh_grant(&state, &req, &scope),
        other => oauth_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            &format!("unsupported grant_type: {other}"),
        ),
    }
}

fn token_code_grant(state: &AppState, req: &TokenRequest, scope: &str) -> Response {
    let Some(code) = &req.code else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_request", "code required");
    };

    // Consume the code (one-time) under the lock, capturing what we need.
    let (email, nonce, client_id) = {
        let mut guard = state.lock();
        let Some(entry) = guard.codes.get_mut(code) else {
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "unknown code");
        };
        if entry.used {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "code already used",
            );
        }
        if let Err(msg) = verify_pkce(
            &entry.code_challenge,
            &entry.code_challenge_method,
            &req.code_verifier,
        ) {
            return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", msg);
        }
        entry.used = true;
        (
            entry.email.clone(),
            entry.nonce.clone(),
            entry.client_id.clone(),
        )
    };

    let Some(user) = state.user_by_email(&email) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "user vanished");
    };
    let user = user.clone();

    let id_token = sign_id_token(state, &user, &client_id, nonce);
    let refresh_token = opaque();
    state.lock().refresh_tokens.insert(
        refresh_token.clone(),
        RefreshEntry {
            email: user.email.clone(),
            sid: user.sid.clone(),
            client_id,
            active: true,
        },
    );

    Json(TokenResponse {
        access_token: opaque(),
        token_type: "Bearer",
        expires_in: state.config.token_ttl,
        refresh_token,
        id_token,
        scope: scope.to_string(),
    })
    .into_response()
}

fn token_refresh_grant(state: &AppState, req: &TokenRequest, scope: &str) -> Response {
    let Some(token) = &req.refresh_token else {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "refresh_token required",
        );
    };

    // Validate + rotate under one lock: reuse of a rotated token, an unknown
    // token, or a revoked user all fail closed with invalid_grant.
    let (email, client_id, new_refresh) = {
        let mut guard = state.lock();
        let Some(entry) = guard.refresh_tokens.get(token) else {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "unknown refresh_token",
            );
        };
        if !entry.active {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "refresh_token already used (rotation reuse)",
            );
        }
        let email = entry.email.clone();
        let sid = entry.sid.clone();
        let client_id = entry.client_id.clone();
        if guard.revoked.contains(&email) {
            // Retire the token so the debug dump reflects the kill.
            if let Some(e) = guard.refresh_tokens.get_mut(token) {
                e.active = false;
            }
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "user revoked at the IdP",
            );
        }
        // Rotate: retire the old token, mint a new active one.
        if let Some(e) = guard.refresh_tokens.get_mut(token) {
            e.active = false;
        }
        let new_refresh = opaque();
        guard.refresh_tokens.insert(
            new_refresh.clone(),
            RefreshEntry {
                email: email.clone(),
                sid,
                client_id: client_id.clone(),
                active: true,
            },
        );
        (email, client_id, new_refresh)
    };

    let Some(user) = state.user_by_email(&email) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "user vanished");
    };
    let user = user.clone();

    // Reuse the original client's audience so a non-default RP still accepts
    // the rotated ID token.
    let id_token = sign_id_token(state, &user, &client_id, None);
    Json(TokenResponse {
        access_token: opaque(),
        token_type: "Bearer",
        expires_in: state.config.token_ttl,
        refresh_token: new_refresh,
        id_token,
        scope: scope.to_string(),
    })
    .into_response()
}

#[derive(Deserialize)]
struct EndSessionParams {
    post_logout_redirect_uri: Option<String>,
    state: Option<String>,
}

/// RP-initiated logout target: just 302 to the requested URI (or 200 if none).
async fn end_session(Query(params): Query<EndSessionParams>) -> Response {
    match params.post_logout_redirect_uri {
        Some(uri) => {
            let location = match params.state {
                Some(st) => {
                    let sep = if uri.contains('?') { '&' } else { '?' };
                    format!("{uri}{sep}state={}", urlencode(&st))
                }
                None => uri,
            };
            redirect(&location)
        }
        None => (StatusCode::OK, "logged out").into_response(),
    }
}

// ─── Test-control hooks (the reason fakeidp exists) ────────────────────────

/// All future refresh attempts for `{email}` return `invalid_grant`, so e2e can
/// assert the authenticator kills every linked session (the G5 refuse path).
async fn control_revoke(State(state): State<Shared>, Path(email): Path<String>) -> Response {
    if state.user_by_email(&email).is_none() {
        return oauth_error(StatusCode::NOT_FOUND, "unknown_user", "no such test user");
    }
    state.lock().revoked.insert(email.clone());
    (StatusCode::OK, Json(json!({ "revoked": email }))).into_response()
}

/// Fire a signed back-channel `logout_token` at the configured RP endpoint.
async fn control_backchannel(State(state): State<Shared>, Path(email): Path<String>) -> Response {
    let Some(user) = state.user_by_email(&email) else {
        return oauth_error(StatusCode::NOT_FOUND, "unknown_user", "no such test user");
    };
    let Some(url) = state.config.backchannel_url.clone() else {
        return oauth_error(
            StatusCode::PRECONDITION_FAILED,
            "not_configured",
            "FAKEIDP_BACKCHANNEL_URL is not set",
        );
    };
    let logout_token = sign_logout_token(&state, user);

    // Bounded: a stalled RP must not hang the control hook indefinitely.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client builds");
    let resp = client
        .post(&url)
        .form(&[("logout_token", logout_token.as_str())])
        .send()
        .await;
    match resp {
        Ok(r) => (
            StatusCode::OK,
            Json(json!({ "sent_to": url, "rp_status": r.status().as_u16() })),
        )
            .into_response(),
        Err(e) => oauth_error(
            StatusCode::BAD_GATEWAY,
            "backchannel_failed",
            &format!("POST to {url} failed: {e}"),
        ),
    }
}

#[derive(Deserialize)]
struct OutageBody {
    mode: String,
}

async fn control_outage(State(state): State<Shared>, Json(body): Json<OutageBody>) -> Response {
    let mode = match body.mode.as_str() {
        "off" => Outage::Off,
        "5xx" => Outage::ServerError,
        "timeout" => Outage::Timeout,
        other => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_mode",
                &format!("mode must be off|5xx|timeout, got {other}"),
            );
        }
    };
    state.lock().outage = Some(mode);
    (StatusCode::OK, Json(json!({ "outage": body.mode }))).into_response()
}

/// Debug dump: users, the revoked set, and outstanding codes / refresh tokens.
async fn control_state(State(state): State<Shared>) -> Json<Value> {
    let guard = state.lock();
    let codes: Vec<Value> = guard
        .codes
        .iter()
        .map(|(code, c)| json!({ "code": code, "email": c.email, "used": c.used }))
        .collect();
    let refresh_tokens: Vec<Value> = guard
        .refresh_tokens
        .iter()
        .map(|(t, r)| json!({ "token": t, "email": r.email, "active": r.active }))
        .collect();
    let outage = match guard.outage.unwrap_or(Outage::Off) {
        Outage::Off => "off",
        Outage::ServerError => "5xx",
        Outage::Timeout => "timeout",
    };
    Json(json!({
        "users": state.users.iter().map(|u| &u.email).collect::<Vec<_>>(),
        "revoked": guard.revoked.iter().collect::<Vec<_>>(),
        "outage": outage,
        "codes": codes,
        "refresh_tokens": refresh_tokens,
    }))
}

// ─── Small helpers ─────────────────────────────────────────────────────────

fn redirect(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, location.to_string())],
    )
        .into_response()
}

/// Minimal percent-encoding for the handful of characters that break a query
/// value (`code`/`state` are our own opaque tokens, so this is enough).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the router. Public so the integration test can serve it in-process.
pub fn app(state: Shared) -> Router {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/end_session", get(end_session).post(end_session))
        .route("/_control/revoke/{email}", post(control_revoke))
        .route("/_control/backchannel/{email}", post(control_backchannel))
        .route("/_control/outage", post(control_outage))
        .route("/_control/state", get(control_state))
        .with_state(state)
}

/// Wire up logging, load config + users, bind, and serve. The binary calls this.
pub async fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env();
    let bind = config.bind.clone();
    let issuer = config.issuer.clone();
    let users = load_users();
    let state = Arc::new(AppState::new(config, users));

    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .unwrap_or_else(|e| panic!("bind {bind}: {e}"));
    tracing::info!(%bind, %issuer, "fakeidp listening (this is a TEST double — never run it in prod)");
    axum::serve(listener, app(state))
        .await
        .expect("server error");
}
