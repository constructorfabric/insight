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
use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::api::AppState;
use crate::api::error::ServiceTokenError;
use crate::config::{AuthenticatorConfig, ServiceTokensConfig};
use crate::jwt::GatewayClaims;

const ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";

const SERVICE_ROLE: &str = "service";
const SERVICE_SUBJECT_TYPE: &str = "service";

struct RegistryEntry {
    keys: Vec<DecodingKey>,
    roles: Vec<String>,
}

pub struct ServiceRegistry {
    entries: HashMap<String, RegistryEntry>,
}

impl ServiceRegistry {
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

fn parse_ec_public_key(pem: &str) -> anyhow::Result<DecodingKey> {
    DecodingKey::from_ec_pem(pem.as_bytes()).context("parse EC public key PEM")
}

#[derive(Debug, Deserialize)]
struct AssertionClaims {
    sub: String,
    jti: String,
    exp: u64,
    iat: u64,
}

#[derive(Debug)]
struct VerifiedAssertion {
    service: String,
    jti: String,
    exp: u64,
}

fn verify_assertion(
    registry: &ServiceRegistry,
    expected_aud: &str,
    max_lifetime_seconds: u64,
    leeway_seconds: u64,
    assertion: &str,
) -> Result<VerifiedAssertion, &'static str> {
    let iss = unverified_claim(assertion, "iss").ok_or("missing_iss")?;
    let entry = registry.get(&iss).ok_or("unknown_service")?;

    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&[expected_aud]);
    validation.set_issuer(&[iss.as_str()]);
    validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
    validation.validate_exp = true;
    validation.leeway = leeway_seconds;

    let mut verified: Option<AssertionClaims> = None;
    for key in &entry.keys {
        match toolkit_security::jwt::decode::<AssertionClaims>(assertion, key, &validation) {
            Ok(data) => {
                verified = Some(data.claims);
                break;
            }
            Err(error) if matches!(error.kind(), ErrorKind::Provider(_)) => {
                return Err("crypto_unavailable");
            }
            Err(_) => {}
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

fn unverified_claim(jwt: &str, field: &str) -> Option<String> {
    let payload_b64 = jwt.split('.').nth(1)?;
    let bytes = B64.decode(payload_b64).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get(field)?.as_str().map(ToOwned::to_owned)
}

fn build_service_claims(
    cfg: &AuthenticatorConfig,
    service: &str,
    registry_roles: &[String],
    tenant_id: &str,
    now: u64,
) -> GatewayClaims {
    let mut roles = registry_roles.to_vec();
    if !roles.iter().any(|r| r == SERVICE_ROLE) {
        roles.push(SERVICE_ROLE.to_owned());
    }
    let subject = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("service:{service}").as_bytes(),
    );
    GatewayClaims {
        sub: subject.to_string(),
        tenant_id: tenant_id.to_owned(),
        roles,
        sub_type: SERVICE_SUBJECT_TYPE.to_owned(),
        sid: format!("service:{service}"),
        iss: cfg.gateway_issuer.clone(),
        aud: cfg.jwt_audience.clone(),
        iat: now,
        exp: now + cfg.service_tokens.token_ttl_seconds,
        jti: Uuid::now_v7().to_string(),
    }
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
}

#[derive(Debug, Default, Deserialize)]
struct TokenRequest {
    #[serde(default)]
    grant_type: Option<String>,
    #[serde(default)]
    client_assertion_type: Option<String>,
    #[serde(default)]
    client_assertion: Option<String>,
    #[serde(default)]
    tenant_id: Option<String>,
}

// Pedantic length lint: one linear parse → verify → replay-guard → mint flow.
#[allow(clippy::too_many_lines)]
async fn token_handler(
    State(state): State<Arc<AppState>>,
    Form(req): Form<TokenRequest>,
) -> Response {
    let st = &state.cfg.service_tokens;

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

    let requested_tenant = req.tenant_id.as_deref().map(str::trim).unwrap_or_default();
    if requested_tenant.is_empty() {
        return bad_request(
            "tenant_id",
            "a service token must name a tenant",
            "TENANT_REQUIRED",
        );
    }
    if requested_tenant.contains(',') {
        return bad_request(
            "tenant_id",
            "a service token must name exactly one tenant",
            "MULTIPLE_TENANTS",
        );
    }

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

    let Some(entry) = state.service_registry.get(&verified.service) else {
        return unauthenticated("unknown_service");
    };

    let claims = build_service_claims(
        &state.cfg,
        &verified.service,
        &entry.roles,
        requested_tenant,
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

    tracing::info!(
        target: "audit",
        event = "service_token_issued",
        service = %verified.service,
        jti = %verified.jti,
        roles = ?claims.roles,
        tenant_id = %claims.tenant_id,
        "service token issued"
    );
    state.audit.emit(crate::audit::AuditEvent {
        action: "service_token_issued",
        outcome: "success",
        tenant_id: claims.tenant_id.clone(),
        actor_person_id: String::new(),
        actor_ip: String::new(),
        actor_user_agent: String::new(),
        correlation_id: String::new(),
        resource_type: "service_token",
        resource_id: verified.service.clone(),
        details: serde_json::json!({
            "roles": claims.roles,
            "jti": verified.jti,
        }),
    });

    Json(TokenResponse {
        access_token: jwt,
        token_type: "Bearer",
        expires_in: st.token_ttl_seconds,
    })
    .into_response()
}

async fn health_handler() -> Response {
    (StatusCode::OK, "ok").into_response()
}

fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/internal/token", post(token_handler))
        .route("/health", get(health_handler))
        .with_state(state)
}

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
                exp: now + 3600,
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
        let claims = build_service_claims(&cfg, "seeder", &[], "", 1_000);
        assert_eq!(
            claims.sub,
            Uuid::new_v5(&Uuid::NAMESPACE_URL, b"service:seeder").to_string()
        );
        assert_eq!(claims.sub_type, "service");
        assert_eq!(claims.sid, "service:seeder");
        assert!(claims.roles.contains(&"service".to_owned()));
        assert_eq!(claims.tenant_id, "");
        assert_eq!(claims.exp, 1_000 + cfg.service_tokens.token_ttl_seconds);
    }

    #[test]
    fn service_claims_do_not_duplicate_service_role() {
        let cfg = AuthenticatorConfig::default();
        let claims = build_service_claims(
            &cfg,
            "perms",
            &["service".to_owned(), "revoke".to_owned()],
            "t-1",
            1_000,
        );
        assert_eq!(
            claims.roles.iter().filter(|r| *r == "service").count(),
            1,
            "service role must not be duplicated"
        );
        assert_eq!(claims.tenant_id, "t-1");
    }
}
