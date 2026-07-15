//! Service tokens (§10 G1, DD-AUTH-05) and the dedicated second listener
//! (§10 G4, §11.8).
//!
//! No-user workloads (background jobs, seeds, the future permissions service)
//! prove their identity with an RFC 7523 `private_key_jwt` client assertion:
//! a short-lived JWT signed by the service's private key, verified here against
//! a gitops-reviewable **registry** of public keys. Out comes a **normal
//! gateway JWT** (`sub = service:<name>`, `sid = service:<name>`, `roles`
//! including `"service"`), signed with the same ES256 key and published in the
//! same JWKS — so a downstream service keeps exactly one verification path for
//! user and service traffic.
//!
//! Service tokens are **always tenant-scoped** (tenant isolation): the request
//! must name at least one tenant (`tenants=[...]`), else it is rejected 400.
//! There is no cross-tenant service token and no per-service opt-in flag.
//!
//! The endpoint lives on its own axum server bound to `token_bind_addr`
//! (suggested 8093), NOT on the main REST host: the token surface must never
//! share the browser/gateway port (§11.8). It carries only `POST /internal/token`
//! and its own `/health`, and is deliberately absent from the public OpenAPI
//! document. Graceful shutdown rides the toolkit's cancellation token.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::AppState;
use crate::api::error::ServiceTokenError;
use crate::config::{AuthenticatorConfig, ServiceTokensConfig};
use crate::jwt::GatewayClaims;

/// RFC 7523 client-assertion type for `client_credentials` (the only one we accept).
const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

/// The role every service token carries, so downstream can authorize
/// machine callers off a single well-known role (§10 G1).
const SERVICE_ROLE: &str = "service";

// ── Service registry (parsed at boot) ─────────────────────────────────────

/// One registry entry, with its public keys pre-parsed into verifiers so a bad
/// PEM fails the gear at boot rather than on the first token request.
struct RegistryEntry {
    /// Verifiers for the service's public key(s) — two during a rotation overlap.
    keys: Vec<DecodingKey>,
    /// Roles to bake into the issued token (`"service"` is always added).
    roles: Vec<String>,
}

/// The parsed service registry — name -> public identity. Built from
/// [`ServiceTokensConfig`]; immutable for the life of the process (reloaded on
/// a pod roll when the gitops ConfigMap changes).
pub struct ServiceRegistry {
    entries: HashMap<String, RegistryEntry>,
}

impl ServiceRegistry {
    /// Parse every registry entry's public keys (inline `public_keys` and any
    /// `public_key_paths`, resolved against `public_key_dir`) into ES256
    /// verifiers, so a bad or missing PEM fails the gear at boot rather than on
    /// the first token request.
    ///
    /// # Errors
    /// Fails when a key file cannot be read or a PEM is not a valid EC public key.
    pub fn build(cfg: &ServiceTokensConfig) -> anyhow::Result<Self> {
        let key_dir = std::path::Path::new(&cfg.public_key_dir);
        let mut entries = HashMap::with_capacity(cfg.services.len());
        for (name, entry) in &cfg.services {
            let mut keys =
                Vec::with_capacity(entry.public_keys.len() + entry.public_key_paths.len());
            for (i, pem) in entry.public_keys.iter().enumerate() {
                keys.push(parse_ec_public_key(pem).with_context(|| {
                    format!(
                        "service_tokens.services.{name}.public_keys[{i}]: invalid EC public key PEM"
                    )
                })?);
            }
            for (i, rel) in entry.public_key_paths.iter().enumerate() {
                let path = if cfg.public_key_dir.is_empty() {
                    std::path::PathBuf::from(rel)
                } else {
                    key_dir.join(rel)
                };
                let pem = std::fs::read_to_string(&path).with_context(|| {
                    format!(
                        "service_tokens.services.{name}.public_key_paths[{i}]: read {}",
                        path.display()
                    )
                })?;
                keys.push(parse_ec_public_key(&pem).with_context(|| {
                    format!(
                        "service_tokens.services.{name}.public_key_paths[{i}] ({}): invalid EC public key PEM",
                        path.display()
                    )
                })?);
            }
            entries.insert(
                name.clone(),
                RegistryEntry {
                    keys,
                    roles: entry.roles.clone(),
                },
            );
        }
        Ok(Self { entries })
    }

    fn get(&self, service: &str) -> Option<&RegistryEntry> {
        self.entries.get(service)
    }
}

/// Parse an SPKI EC public-key PEM into an ES256 verifier.
fn parse_ec_public_key(pem: &str) -> anyhow::Result<DecodingKey> {
    DecodingKey::from_ec_pem(pem.as_bytes()).context("parse EC public key PEM")
}

// ── Assertion verification (pure, unit-tested) ─────────────────────────────

/// The RFC 7523 assertion claim set we read. `iss`/`aud`/`exp` are validated by
/// `jsonwebtoken` (issuer/audience/expiry); the rest are checked explicitly.
#[derive(Debug, Deserialize)]
struct AssertionClaims {
    sub: String,
    jti: String,
    exp: u64,
    iat: u64,
}

/// A successfully verified assertion: which service signed it, and the `jti`
/// (for the replay guard) plus `exp` (for the guard TTL).
#[derive(Debug)]
struct VerifiedAssertion {
    service: String,
    jti: String,
    exp: u64,
}

/// Verify a client assertion against the registry. Returns the reason string
/// for a 401 on any failure — deliberately coarse so a caller learns nothing
/// about *why* beyond "the assertion was not accepted".
///
/// Checks: `iss` names a registered service; the signature verifies against one
/// of that service's public keys; `aud` equals the token endpoint URL; `exp`
/// is in the future; `sub == iss == <service>`; and `exp - iat` is within the
/// configured cap (RFC 7523 assertions are short-lived and single-use).
fn verify_assertion(
    registry: &ServiceRegistry,
    expected_aud: &str,
    max_lifetime_seconds: u64,
    leeway_seconds: u64,
    assertion: &str,
) -> Result<VerifiedAssertion, &'static str> {
    // `iss` picks the key set. Read it from the unverified payload first; the
    // signature check against that service's registered key is what actually
    // authenticates — a forged `iss` simply finds keys that won't verify.
    let iss = unverified_claim(assertion, "iss").ok_or("missing_iss")?;
    let entry = registry.get(&iss).ok_or("unknown_service")?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[expected_aud]);
    validation.set_issuer(&[iss.as_str()]);
    // `iat`/`jti` presence is enforced by the non-Option fields on
    // AssertionClaims (deserialization fails without them), so they need not be
    // repeated here — this list is the reserved-claim presence check.
    validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
    validation.validate_exp = true;
    validation.leeway = leeway_seconds;

    let mut verified: Option<AssertionClaims> = None;
    for key in &entry.keys {
        if let Ok(data) = decode::<AssertionClaims>(assertion, key, &validation) {
            verified = Some(data.claims);
            break;
        }
    }
    let claims = verified.ok_or("assertion_verification_failed")?;

    if claims.sub != iss {
        return Err("sub_mismatch");
    }
    if claims.exp.saturating_sub(claims.iat) > max_lifetime_seconds {
        return Err("assertion_lifetime_too_long");
    }
    Ok(VerifiedAssertion {
        service: iss,
        jti: claims.jti,
        exp: claims.exp,
    })
}

/// Decode a string claim from an (unverified) compact JWT payload — used only
/// to read `iss` so the right verifier key can be selected.
fn unverified_claim(jwt: &str, field: &str) -> Option<String> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = B64.decode(payload_b64).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get(field)?.as_str().map(ToOwned::to_owned)
}

/// Build the gateway JWT claims for a service token (§10 G1 / §3.8):
/// `sub = sid = service:<name>`, `roles` from the registry with `"service"`
/// guaranteed, and the requested `tenants` (always present — service tokens are
/// tenant-scoped by design; the handler rejects a request that names none).
fn build_service_claims(
    cfg: &AuthenticatorConfig,
    service: &str,
    registry_roles: &[String],
    tenants: &[String],
    now: u64,
) -> GatewayClaims {
    let mut roles = registry_roles.to_vec();
    if !roles.iter().any(|r| r == SERVICE_ROLE) {
        roles.push(SERVICE_ROLE.to_owned());
    }
    GatewayClaims {
        sub: format!("service:{service}"),
        tenants: tenants.to_vec(),
        roles,
        // sid choice (DESIGN §3.8): service tokens have no session, so `sid`
        // carries `service:<name>` — a stable, non-empty value that keeps the
        // claim shape fixed and correlates a service's issuance in audit/trace.
        sid: format!("service:{service}"),
        iss: cfg.gateway_issuer.clone(),
        aud: cfg.jwt_audience.clone(),
        iat: now,
        exp: now + cfg.service_tokens.token_ttl_seconds,
        jti: Uuid::now_v7().to_string(),
    }
}

/// Split the optional comma-separated `tenants` form field into ids.
fn parse_tenants(raw: Option<&str>) -> Vec<String> {
    raw.map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    })
    .unwrap_or_default()
}

// ── HTTP layer (the second listener) ───────────────────────────────────────

/// `POST /internal/token` success body (OAuth2 `client_credentials` shape).
#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
}

/// `POST /internal/token` request body (form-encoded, RFC 7523 shape).
#[derive(Debug, Default, Deserialize)]
struct TokenRequest {
    #[serde(default)]
    grant_type: Option<String>,
    #[serde(default)]
    client_assertion_type: Option<String>,
    #[serde(default)]
    client_assertion: Option<String>,
    /// Optional comma-separated tenant ids for a tenant-scoped token.
    #[serde(default)]
    tenants: Option<String>,
}

/// Handle a service-token request: validate the grant shape, verify the
/// assertion, replay-guard its `jti`, apply the tenant-scope policy, then mint
/// and return a normal gateway JWT.
async fn token_handler(
    State(state): State<Arc<AppState>>,
    Form(req): Form<TokenRequest>,
) -> Response {
    let st = &state.cfg.service_tokens;

    // Grant shape (RFC 7523 client_credentials). Malformed = 400.
    if req.grant_type.as_deref() != Some("client_credentials") {
        return bad_request(
            "grant_type",
            "expected client_credentials",
            "UNSUPPORTED_GRANT",
        );
    }
    if req.client_assertion_type.as_deref() != Some(ASSERTION_TYPE) {
        return bad_request(
            "client_assertion_type",
            "expected jwt-bearer assertion type",
            "UNSUPPORTED_ASSERTION_TYPE",
        );
    }
    let Some(assertion) = req.client_assertion.as_deref().filter(|a| !a.is_empty()) else {
        return bad_request("client_assertion", "missing assertion", "MISSING");
    };

    // Tenant isolation: service tokens are always tenant-scoped. A request that
    // names no tenant is rejected up front (400) — before the assertion is
    // verified or its jti consumed — so the caller can fix and retry. Where the
    // service gets its tenant is the service's concern.
    let requested_tenants = parse_tenants(req.tenants.as_deref());
    if requested_tenants.is_empty() {
        return bad_request(
            "tenants",
            "a service token must name a tenant",
            "TENANT_REQUIRED",
        );
    }

    // Verify signature + claims against the registry.
    let verified = match verify_assertion(
        &state.service_registry,
        &st.audience,
        st.assertion_max_lifetime_seconds,
        st.clock_skew_leeway_seconds,
        assertion,
    ) {
        Ok(v) => v,
        Err(reason) => {
            tracing::warn!(reason, "service-token assertion rejected");
            return unauthenticated(reason);
        }
    };

    // Replay guard: a valid assertion is single-use. Guard until it can no
    // longer be accepted (its own remaining lifetime plus the skew leeway).
    let now = now_secs();
    let guard_ttl = verified.exp.saturating_sub(now) + st.clock_skew_leeway_seconds;
    match state
        .sessions
        .guard_service_jti(&verified.service, &verified.jti, guard_ttl)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(service = %verified.service, jti = %verified.jti, "service-token assertion replayed");
            return unauthenticated("assertion_replayed");
        }
        Err(e) => {
            tracing::warn!(error = %e, "service-token replay guard: store unavailable");
            return CanonicalError::service_unavailable()
                .with_detail("session store unavailable")
                .create()
                .into_response();
        }
    }

    // Registry entry is guaranteed present (verify_assertion matched it).
    let Some(entry) = state.service_registry.get(&verified.service) else {
        return unauthenticated("unknown_service");
    };

    let claims = build_service_claims(
        &state.cfg,
        &verified.service,
        &entry.roles,
        &requested_tenants,
        now,
    );
    let jwt = match state.keystore.sign(&claims) {
        Ok(j) => j,
        Err(e) => {
            tracing::error!(error = %e, "service-token signing failed");
            return CanonicalError::internal("service token signing failed")
                .create()
                .into_response();
        }
    };

    // Every issuance is audited (§10 G1).
    tracing::info!(
        target: "audit",
        event = "service_token_issued",
        service = %verified.service,
        jti = %verified.jti,
        roles = ?claims.roles,
        tenants = ?claims.tenants,
        "service token issued"
    );

    Json(TokenResponse {
        access_token: jwt,
        token_type: "Bearer",
        expires_in: st.token_ttl_seconds,
    })
    .into_response()
}

/// Health of the token listener (liveness of the socket; the gear only binds it
/// once Redis + keys are up, so a reachable `/health` implies both). Uses the
/// same `/health` path the main host + k8s probes use, for one convention.
async fn health_handler() -> Response {
    (StatusCode::OK, "ok").into_response()
}

/// The token listener's router: exactly the two endpoints it owns.
fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/internal/token", post(token_handler))
        .route("/health", get(health_handler))
        .with_state(state)
}

/// Bind the second listener and spawn its server, shutting down when `cancel`
/// fires. Returns after the bind so the gear's `start` stays prompt; a bind
/// failure surfaces at boot (fail closed).
///
/// # Errors
/// Fails when `token_bind_addr` cannot be bound.
// TODO(#1583): split the public and private surfaces into two containers
// (sidecar) instead of one process with two listeners. The token endpoint is a
// hand-rolled axum server here only because the toolkit REST host binds a
// single address and OperationBuilder/OpenAPI target that one port (§11.8). A
// clean split — the public gear (/auth/*, /internal/authz, JWKS) in one
// container and a private service-token gear in a sidecar — would let each run
// on the toolkit host with OperationBuilder + generated OpenAPI, gain a true
// separate failure domain, and be network-scoped at the pod boundary rather
// than per-port. Deferred: not worth a second image + shared-state (registry,
// signing keys, Redis) wiring until there is a service-token consumer (step 07).
pub async fn spawn(state: Arc<AppState>, cancel: CancellationToken) -> anyhow::Result<()> {
    let addr = state.cfg.service_tokens.token_bind_addr.clone();
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("bind service-token listener at {addr}"))?;
    tracing::info!(%addr, "service-token listener bound (POST /internal/token)");

    let app = router(state);
    tokio::spawn(async move {
        let shutdown = async move { cancel.cancelled().await };
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await
        {
            tracing::error!(error = %e, "service-token listener exited with error");
        }
    });
    Ok(())
}

fn now_secs() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

fn unauthenticated(reason: &str) -> Response {
    CanonicalError::unauthenticated()
        .with_reason(reason.to_owned())
        .create()
        .into_response()
}

fn bad_request(field: &str, message: &str, code: &str) -> Response {
    ServiceTokenError::invalid_argument()
        .with_field_violation(field, message, code)
        .create()
        .into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use p256::SecretKey;
    use p256::elliptic_curve::Generate as _;
    use p256::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _, LineEnding};
    use serde::Serialize;

    const AUD: &str = "http://authenticator:8093/internal/token";

    #[derive(Serialize)]
    struct Assertion {
        iss: String,
        sub: String,
        aud: String,
        jti: String,
        exp: u64,
        iat: u64,
    }

    /// A fresh P-256 keypair: (PKCS#8 private PEM, SPKI public PEM).
    fn keypair() -> (String, String) {
        let secret = SecretKey::generate();
        let priv_pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
        let pub_pem = secret
            .public_key()
            .to_public_key_pem(LineEnding::LF)
            .unwrap();
        (priv_pem, pub_pem)
    }

    fn registry(service: &str, pub_pem: &str) -> ServiceRegistry {
        let mut services = HashMap::new();
        services.insert(
            service.to_owned(),
            crate::config::ServiceRegistryEntry {
                public_keys: vec![pub_pem.to_owned()],
                public_key_paths: vec![],
                roles: vec!["service".to_owned()],
            },
        );
        let cfg = ServiceTokensConfig {
            services,
            ..Default::default()
        };
        ServiceRegistry::build(&cfg).unwrap()
    }

    fn sign(priv_pem: &str, claims: &Assertion) -> String {
        let key = EncodingKey::from_ec_pem(priv_pem.as_bytes()).unwrap();
        encode(&Header::new(Algorithm::ES256), claims, &key).unwrap()
    }

    /// Assertion with `iat = now` and `exp = now + exp_delta` (delta may be
    /// negative to model an already-expired assertion).
    fn assertion(service: &str, exp_delta: i64) -> Assertion {
        let now = now_secs();
        let exp = i64::try_from(now).unwrap() + exp_delta;
        Assertion {
            iss: service.to_owned(),
            sub: service.to_owned(),
            aud: AUD.to_owned(),
            jti: Uuid::now_v7().to_string(),
            exp: u64::try_from(exp).unwrap_or(0),
            iat: now,
        }
    }

    #[test]
    fn verifies_a_valid_assertion() {
        let (priv_pem, pub_pem) = keypair();
        let reg = registry("testclient", &pub_pem);
        let a = sign(&priv_pem, &assertion("testclient", 30));
        let v = verify_assertion(&reg, AUD, 60, 30, &a).unwrap();
        assert_eq!(v.service, "testclient");
        assert!(!v.jti.is_empty());
    }

    #[test]
    fn rejects_wrong_key() {
        let (_priv_pem, pub_pem) = keypair();
        let (attacker_priv, _) = keypair();
        let reg = registry("testclient", &pub_pem);
        // Signed by a key the registry does not hold.
        let a = sign(&attacker_priv, &assertion("testclient", 30));
        assert_eq!(
            verify_assertion(&reg, AUD, 60, 30, &a).unwrap_err(),
            "assertion_verification_failed"
        );
    }

    #[test]
    fn rejects_unknown_service() {
        let (priv_pem, pub_pem) = keypair();
        let reg = registry("testclient", &pub_pem);
        let a = sign(&priv_pem, &assertion("someone-else", 30));
        assert_eq!(
            verify_assertion(&reg, AUD, 60, 30, &a).unwrap_err(),
            "unknown_service"
        );
    }

    #[test]
    fn rejects_wrong_audience() {
        let (priv_pem, pub_pem) = keypair();
        let reg = registry("testclient", &pub_pem);
        let mut a = assertion("testclient", 30);
        a.aud = "http://evil.example/token".to_owned();
        let signed = sign(&priv_pem, &a);
        assert_eq!(
            verify_assertion(&reg, AUD, 60, 30, &signed).unwrap_err(),
            "assertion_verification_failed"
        );
    }

    #[test]
    fn rejects_expired_assertion() {
        let (priv_pem, pub_pem) = keypair();
        let reg = registry("testclient", &pub_pem);
        // exp 120 s in the past, well beyond the 5 s leeway.
        let a = sign(&priv_pem, &assertion("testclient", -120));
        assert_eq!(
            verify_assertion(&reg, AUD, 60, 5, &a).unwrap_err(),
            "assertion_verification_failed"
        );
    }

    #[test]
    fn rejects_overlong_lifetime() {
        let (priv_pem, pub_pem) = keypair();
        let reg = registry("testclient", &pub_pem);
        let now = now_secs();
        let a = sign(
            &priv_pem,
            &Assertion {
                iss: "testclient".to_owned(),
                sub: "testclient".to_owned(),
                aud: AUD.to_owned(),
                jti: Uuid::now_v7().to_string(),
                iat: now,
                exp: now + 3600, // 1 h lifetime, cap is 60 s
            },
        );
        assert_eq!(
            verify_assertion(&reg, AUD, 60, 30, &a).unwrap_err(),
            "assertion_lifetime_too_long"
        );
    }

    #[test]
    fn rejects_sub_iss_mismatch() {
        let (priv_pem, pub_pem) = keypair();
        let reg = registry("testclient", &pub_pem);
        let now = now_secs();
        // iss is the registered service, but sub disagrees.
        let a = sign(
            &priv_pem,
            &Assertion {
                iss: "testclient".to_owned(),
                sub: "someone-else".to_owned(),
                aud: AUD.to_owned(),
                jti: Uuid::now_v7().to_string(),
                iat: now,
                exp: now + 30,
            },
        );
        assert_eq!(
            verify_assertion(&reg, AUD, 60, 30, &a).unwrap_err(),
            "sub_mismatch"
        );
    }

    #[test]
    fn service_claims_always_carry_service_role_and_sid() {
        let cfg = AuthenticatorConfig {
            gateway_issuer: "http://gw".to_owned(),
            jwt_audience: "internal-services".to_owned(),
            ..Default::default()
        };
        let claims = build_service_claims(&cfg, "seeder", &[], &[], 1_000);
        assert_eq!(claims.sub, "service:seeder");
        assert_eq!(claims.sid, "service:seeder");
        assert!(claims.roles.contains(&"service".to_owned()));
        assert!(claims.tenants.is_empty());
        assert_eq!(claims.exp, 1_000 + cfg.service_tokens.token_ttl_seconds);
    }

    #[test]
    fn service_claims_do_not_duplicate_service_role() {
        let cfg = AuthenticatorConfig::default();
        let claims = build_service_claims(
            &cfg,
            "perms",
            &["service".to_owned(), "revoke".to_owned()],
            &["t-1".to_owned()],
            1_000,
        );
        assert_eq!(
            claims.roles.iter().filter(|r| *r == "service").count(),
            1,
            "service role must not be duplicated"
        );
        assert_eq!(claims.tenants, vec!["t-1"]);
    }

    #[test]
    fn parse_tenants_splits_and_trims() {
        assert_eq!(parse_tenants(Some(" t-a , t-b ,,")), vec!["t-a", "t-b"]);
        assert!(parse_tenants(None).is_empty());
        assert!(parse_tenants(Some("")).is_empty());
    }
}
