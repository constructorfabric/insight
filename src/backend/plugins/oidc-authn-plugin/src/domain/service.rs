//! OIDC token validation service.
//!
//! Uses `modkit-auth` `JwksKeyProvider` for JWT signature validation
//! and `validate_claims` for standard claim checks.

use modkit_auth::traits::KeyProvider;
use modkit_auth::{JwksKeyProvider, ValidationConfig, validate_claims};
use modkit_security::SecurityContext;
use secrecy::SecretString;
use std::sync::Arc;
use uuid::Uuid;

use crate::config::OidcAuthnPluginConfig;

/// Errors from OIDC token validation.
#[derive(Debug, thiserror::Error)]
pub enum OidcError {
    #[error("token signature validation failed: {0}")]
    SignatureInvalid(String),

    #[error("token claims validation failed: {0}")]
    ClaimsInvalid(String),

    #[error("missing required claim: {0}")]
    MissingClaim(String),

    #[error("invalid claim format: {field} — {reason}")]
    InvalidClaimFormat { field: String, reason: String },
}

/// OIDC token validation service.
///
/// Validates JWT bearer tokens using JWKS key discovery and
/// builds a `SecurityContext` from the validated claims.
pub struct OidcService {
    key_provider: Arc<JwksKeyProvider>,
    validation_config: ValidationConfig,
    issuer_url: String,
    tenant_claim: String,
    require_tenant_claim: bool,
    subject_type: String,
}

impl OidcService {
    /// Creates a new OIDC service from plugin configuration.
    #[must_use]
    pub fn new(config: &OidcAuthnPluginConfig, key_provider: Arc<JwksKeyProvider>) -> Self {
        let mut allowed_issuers = Vec::new();
        if !config.issuer_url.is_empty() {
            allowed_issuers.push(config.issuer_url.clone());
        }

        let mut allowed_audiences = Vec::new();
        if !config.audience.is_empty() {
            allowed_audiences.push(config.audience.clone());
        }

        let validation_config = ValidationConfig {
            allowed_issuers,
            allowed_audiences,
            leeway_seconds: config.leeway_seconds,
            require_exp: true,
        };

        Self {
            key_provider,
            validation_config,
            issuer_url: config.issuer_url.clone(),
            tenant_claim: config.tenant_claim.clone(),
            require_tenant_claim: config.require_tenant_claim,
            subject_type: config.subject_type.clone(),
        }
    }

    /// Validates a JWT bearer token and returns a `SecurityContext`.
    ///
    /// # Flow
    ///
    /// 1. Validate JWT signature using JWKS keys
    /// 2. Validate standard claims (iss, aud, exp, nbf)
    /// 3. Extract subject (`sub` claim) → `subject_id`
    /// 4. Extract tenant (`tenant_claim`) → `subject_tenant_id`
    /// 5. Extract scopes (`scp` or `scope` claim) → `token_scopes`
    /// 6. Build `SecurityContext`
    ///
    /// # Errors
    ///
    /// Returns `OidcError` if token is invalid, expired, or missing required claims.
    pub async fn validate_token(&self, token: &str) -> Result<SecurityContext, OidcError> {
        // 1. Validate signature and decode claims
        let (_header, claims) = self
            .key_provider
            .validate_and_decode(token)
            .await
            .map_err(|e| OidcError::SignatureInvalid(e.to_string()))?;

        // 2. Validate standard claims
        validate_claims(&claims, &self.validation_config)
            .map_err(|e| OidcError::ClaimsInvalid(e.to_string()))?;

        // 3. Extract subject_id from `sub` claim
        let sub_str = claims
            .get("sub")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| OidcError::MissingClaim("sub".to_owned()))?;

        // OIDC `sub` is often not a UUID — use a deterministic UUID v5 from issuer+sub
        // to prevent collisions across different IdPs
        let subject_id = uuid_from_issuer_sub(&self.issuer_url, sub_str);

        // 4. Extract tenant_id from configured claim
        let subject_tenant_id = match claims
            .get(&self.tenant_claim)
            .and_then(serde_json::Value::as_str)
        {
            Some(s) if Uuid::parse_str(s).is_ok() => {
                // Safe: just checked is_ok
                Uuid::parse_str(s).unwrap_or_default()
            }
            Some(s) => {
                tracing::warn!(
                    claim = %self.tenant_claim,
                    value = %s,
                    "tenant claim is not a valid UUID"
                );
                if self.require_tenant_claim {
                    return Err(OidcError::InvalidClaimFormat {
                        field: self.tenant_claim.clone(),
                        reason: format!("not a valid UUID: {s}"),
                    });
                }
                Uuid::default()
            }
            None => {
                tracing::warn!(
                    claim = %self.tenant_claim,
                    "tenant claim missing from JWT"
                );
                if self.require_tenant_claim {
                    return Err(OidcError::MissingClaim(self.tenant_claim.clone()));
                }
                Uuid::default()
            }
        };

        // 5. Extract scopes from `scp` (Okta) or `scope` (standard) claim
        let token_scopes = extract_scopes(&claims);

        // 6. Build SecurityContext
        let ctx = SecurityContext::builder()
            .subject_id(subject_id)
            .subject_type(&self.subject_type)
            .subject_tenant_id(subject_tenant_id)
            .token_scopes(token_scopes)
            .bearer_token(SecretString::from(token.to_owned()))
            .build()
            .map_err(|e| OidcError::InvalidClaimFormat {
                field: "security_context".to_owned(),
                reason: e.to_string(),
            })?;

        Ok(ctx)
    }
}

/// Creates a deterministic UUID v5 from an OIDC issuer + `sub` claim.
/// Includes issuer to prevent collisions when the same `sub` appears across different providers.
fn uuid_from_issuer_sub(issuer: &str, sub: &str) -> Uuid {
    let input = format!("{issuer}#{sub}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, input.as_bytes())
}

/// Extracts scopes from JWT claims.
/// Supports Okta-style `scp` (array) and standard `scope` (space-delimited string).
fn extract_scopes(claims: &serde_json::Value) -> Vec<String> {
    // Try `scp` first (Okta style — array of strings)
    if let Some(scp) = claims.get("scp").and_then(serde_json::Value::as_array) {
        return scp
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(String::from)
            .collect();
    }

    // Try `scope` (standard — space-delimited string)
    if let Some(scope) = claims.get("scope").and_then(serde_json::Value::as_str) {
        return scope.split_whitespace().map(String::from).collect();
    }

    // No scopes claim present — return empty (authz layer decides access)
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISSUER_A: &str = "https://okta-a.example.com";
    const ISSUER_B: &str = "https://okta-b.example.com";

    #[test]
    fn uuid_from_issuer_sub_is_deterministic() {
        let id1 = uuid_from_issuer_sub(ISSUER_A, "user123");
        let id2 = uuid_from_issuer_sub(ISSUER_A, "user123");
        assert_eq!(id1, id2);
    }

    #[test]
    fn uuid_from_issuer_sub_different_subs_differ() {
        let id1 = uuid_from_issuer_sub(ISSUER_A, "user123");
        let id2 = uuid_from_issuer_sub(ISSUER_A, "user456");
        assert_ne!(id1, id2);
    }

    #[test]
    fn uuid_from_issuer_sub_different_issuers_differ() {
        // Same sub across different IdPs must NOT collide
        let id1 = uuid_from_issuer_sub(ISSUER_A, "user123");
        let id2 = uuid_from_issuer_sub(ISSUER_B, "user123");
        assert_ne!(id1, id2);
    }

    #[test]
    fn uuid_from_issuer_sub_empty_sub() {
        // Should not panic — empty sub is technically valid OIDC
        let id = uuid_from_issuer_sub(ISSUER_A, "");
        assert_ne!(id, Uuid::default());
    }

    #[test]
    fn extract_scopes_okta_style() {
        let claims = serde_json::json!({
            "scp": ["openid", "profile", "email"]
        });
        let scopes = extract_scopes(&claims);
        assert_eq!(scopes, vec!["openid", "profile", "email"]);
    }

    #[test]
    fn extract_scopes_standard_style() {
        let claims = serde_json::json!({
            "scope": "openid profile email"
        });
        let scopes = extract_scopes(&claims);
        assert_eq!(scopes, vec!["openid", "profile", "email"]);
    }

    #[test]
    fn extract_scopes_none_returns_empty() {
        let claims = serde_json::json!({});
        let scopes = extract_scopes(&claims);
        assert!(
            scopes.is_empty(),
            "no scope claims should return empty vec, not wildcard"
        );
    }

    #[test]
    fn extract_scopes_scp_takes_priority() {
        let claims = serde_json::json!({
            "scp": ["admin"],
            "scope": "read write"
        });
        let scopes = extract_scopes(&claims);
        assert_eq!(scopes, vec!["admin"]);
    }

    #[test]
    fn extract_scopes_empty_scope_string_returns_empty() {
        let claims = serde_json::json!({
            "scope": ""
        });
        let scopes = extract_scopes(&claims);
        assert!(scopes.is_empty());
    }

    #[test]
    fn extract_scopes_scp_with_non_string_elements_filtered() {
        let claims = serde_json::json!({
            "scp": ["openid", 123, true, "email"]
        });
        let scopes = extract_scopes(&claims);
        assert_eq!(scopes, vec!["openid", "email"]);
    }
}
