//! JWT Issuer, Key Store, and JWKS (§3.8 claim contract, DESIGN 3.10).
//!
//! **§9.6 decision — ES256.** The deleted spec mandated EdDSA (DD-BFF-05: small
//! signatures, fast verify), but the downstream verifier (`oidc-authn-plugin`)
//! validates RS256/ES256 today; EdDSA would need a non-trivial plugin change,
//! and downstream verification is a later phase. ES256 (P-256 / ECDSA-SHA256)
//! satisfies DD-BFF-05's rationale — 64-byte signatures, fast verify — with
//! zero downstream friction, so the authenticator mints ES256. Recorded in the
//! DESIGN and the PR.
//!
//! Keys are a plain mounted directory (`signing_keys_path`): `current.pem`
//! (PKCS#8 EC P-256 private key, required) and optional `previous.pem` for the
//! rotation overlap. The `kid` is the RFC 7638 JWK thumbprint — stable, needs
//! no manifest. Rotation = drop a new `current.pem` and re-read (or roll pods).

use std::path::Path;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToSec1Point as _;
use p256::pkcs8::DecodePrivateKey as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// The gateway JWT claim set (§3.8). Serializes to the wire claims verbatim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayClaims {
    /// Internal `person_id` (or `service:<name>` for service tokens, later).
    pub sub: String,
    /// All tenant memberships resolved at login — the only tenant authority.
    pub tenants: Vec<String>,
    /// Access-control roles (default `["user"]` until the permissions service).
    pub roles: Vec<String>,
    /// Stable session id (UUIDv7) — survives cookie rotation.
    pub sid: String,
    /// Gateway origin issuer URL.
    pub iss: String,
    /// Audience — `internal-services`.
    pub aud: String,
    /// Issued-at (epoch seconds).
    pub iat: u64,
    /// Expiry (epoch seconds), clamped to the session absolute cap.
    pub exp: u64,
    /// Unique token id (UUIDv7) — replay/audit correlation.
    pub jti: String,
}

/// One loaded signing key: its `kid`, the signer, the public JWK, and a matching
/// verifier (used by the JWKS-verify path and tests).
struct LoadedKey {
    kid: String,
    encoding: EncodingKey,
    // Read by `KeyStore::verify` (unit-tested; downstream services verify via
    // the published JWKS rather than this in-process path).
    #[allow(dead_code)]
    decoding: DecodingKey,
    jwk: serde_json::Value,
}

impl LoadedKey {
    /// Load from a PKCS#8 EC P-256 private-key PEM.
    fn from_pkcs8_pem(pem: &str) -> anyhow::Result<Self> {
        let secret = SecretKey::from_pkcs8_pem(pem)
            .context("parse PKCS#8 EC P-256 private key (expected an ES256 signing key)")?;
        let public = secret.public_key();

        // Uncompressed SEC1 point: 0x04 || X(32) || Y(32).
        let point = public.to_sec1_point(false);
        let x = point.x().context("EC public key missing X coordinate")?;
        let y = point.y().context("EC public key missing Y coordinate")?;
        let x_b64 = B64.encode(x);
        let y_b64 = B64.encode(y);

        let kid = jwk_thumbprint_es256(&x_b64, &y_b64);

        let jwk = serde_json::json!({
            "kty": "EC",
            "crv": "P-256",
            "x": x_b64,
            "y": y_b64,
            "use": "sig",
            "alg": "ES256",
            "kid": kid,
        });

        let encoding = EncodingKey::from_ec_pem(pem.as_bytes())
            .context("build ES256 encoding key from PEM")?;
        let decoding = DecodingKey::from_ec_components(&x_b64, &y_b64)
            .context("build ES256 decoding key from EC components")?;

        Ok(Self {
            kid,
            encoding,
            decoding,
            jwk,
        })
    }
}

/// RFC 7638 thumbprint of an EC P-256 public JWK, base64url-encoded.
///
/// Canonical members in lexicographic order, no whitespace:
/// `{"crv":"P-256","kty":"EC","x":"..","y":".."}`.
fn jwk_thumbprint_es256(x_b64: &str, y_b64: &str) -> String {
    let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x_b64}","y":"{y_b64}"}}"#);
    let digest = Sha256::digest(canonical.as_bytes());
    B64.encode(digest)
}

/// Loads the signing keys and serves both signing and JWKS.
pub struct KeyStore {
    current: LoadedKey,
    previous: Option<LoadedKey>,
}

impl KeyStore {
    /// Load `current.pem` (required) and optional `previous.pem` from `dir`.
    ///
    /// # Errors
    /// Fails when `current.pem` is missing or not a valid PKCS#8 EC P-256 key.
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        let current_path = dir.join("current.pem");
        let current_pem = std::fs::read_to_string(&current_path).with_context(|| {
            format!(
                "read signing key {} (mount the authenticator signing-keys Secret here)",
                current_path.display()
            )
        })?;
        let current = LoadedKey::from_pkcs8_pem(&current_pem)?;

        let previous_path = dir.join("previous.pem");
        let previous = match std::fs::read_to_string(&previous_path) {
            Ok(pem) => Some(LoadedKey::from_pkcs8_pem(&pem)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(e).with_context(|| format!("read {}", previous_path.display()));
            }
        };

        Ok(Self { current, previous })
    }

    /// The JWKS document: current key first, then previous during an overlap.
    #[must_use]
    pub fn jwks(&self) -> serde_json::Value {
        let mut keys = vec![self.current.jwk.clone()];
        if let Some(prev) = &self.previous {
            keys.push(prev.jwk.clone());
        }
        serde_json::json!({ "keys": keys })
    }

    /// Sign `claims` with the current key.
    ///
    /// # Errors
    /// Fails only on an internal serialization/signing error.
    pub fn sign(&self, claims: &GatewayClaims) -> anyhow::Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.current.kid.clone());
        encode(&header, claims, &self.current.encoding).context("sign gateway JWT")
    }

    /// Build a store directly from a PKCS#8 PEM (tests only).
    #[cfg(test)]
    fn from_pem_for_test(current_pem: &str) -> anyhow::Result<Self> {
        Ok(Self {
            current: LoadedKey::from_pkcs8_pem(current_pem)?,
            previous: None,
        })
    }

    /// Verify a gateway JWT against current, then previous, key.
    ///
    /// Validates the signature, `exp`, and `aud`. Used by the JWKS-verify e2e
    /// path and unit tests; downstream services verify independently via JWKS.
    ///
    /// # Errors
    /// Fails when the token verifies against no held key.
    #[allow(dead_code)] // exercised by unit tests; kept as the in-process verify path
    pub fn verify(&self, token: &str, audience: &str) -> anyhow::Result<GatewayClaims> {
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&[audience]);
        validation.validate_exp = true;

        let mut last_err = None;
        for key in std::iter::once(&self.current).chain(self.previous.iter()) {
            match decode::<GatewayClaims>(token, &key.decoding, &validation) {
                Ok(data) => return Ok(data.claims),
                Err(e) => last_err = Some(e),
            }
        }
        Err(anyhow::anyhow!(
            "gateway JWT verified against no held key: {}",
            last_err.map_or_else(|| "no keys loaded".to_owned(), |e| e.to_string())
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use p256::elliptic_curve::Generate as _;
    use p256::pkcs8::{EncodePrivateKey as _, LineEnding};

    fn gen_pem() -> String {
        p256::SecretKey::generate()
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string()
    }

    fn sample_claims() -> GatewayClaims {
        GatewayClaims {
            sub: "person-123".to_owned(),
            tenants: vec!["t-a".to_owned(), "t-b".to_owned()],
            roles: vec!["user".to_owned(), "admin".to_owned()],
            sid: "sid-xyz".to_owned(),
            iss: "http://gateway.local".to_owned(),
            aud: "internal-services".to_owned(),
            iat: 1_000,
            exp: 4_000_000_000,
            jti: "jti-1".to_owned(),
        }
    }

    #[test]
    fn sign_then_verify_preserves_claims() {
        let store = KeyStore::from_pem_for_test(&gen_pem()).unwrap();
        let jwt = store.sign(&sample_claims()).unwrap();

        // Header advertises ES256 + a kid.
        let header = jsonwebtoken::decode_header(&jwt).unwrap();
        assert_eq!(header.alg, Algorithm::ES256);
        assert!(header.kid.is_some());

        let claims = store.verify(&jwt, "internal-services").unwrap();
        assert_eq!(claims.sub, "person-123");
        assert_eq!(claims.tenants, vec!["t-a", "t-b"]);
        assert_eq!(claims.roles, vec!["user", "admin"]);
        assert_eq!(claims.sid, "sid-xyz");
        assert_eq!(claims.aud, "internal-services");
    }

    #[test]
    fn verify_rejects_wrong_audience() {
        let store = KeyStore::from_pem_for_test(&gen_pem()).unwrap();
        let jwt = store.sign(&sample_claims()).unwrap();
        assert!(store.verify(&jwt, "some-other-aud").is_err());
    }

    #[test]
    fn jwks_has_es256_ec_shape() {
        let store = KeyStore::from_pem_for_test(&gen_pem()).unwrap();
        let jwks = store.jwks();
        let key = &jwks["keys"][0];
        assert_eq!(key["kty"], "EC");
        assert_eq!(key["crv"], "P-256");
        assert_eq!(key["use"], "sig");
        assert_eq!(key["alg"], "ES256");
        assert!(key["kid"].as_str().is_some_and(|k| !k.is_empty()));
        assert!(key["x"].as_str().is_some());
        assert!(key["y"].as_str().is_some());
    }

    #[test]
    fn kid_is_stable_for_a_key() {
        let pem = gen_pem();
        let a = KeyStore::from_pem_for_test(&pem).unwrap();
        let b = KeyStore::from_pem_for_test(&pem).unwrap();
        assert_eq!(a.jwks()["keys"][0]["kid"], b.jwks()["keys"][0]["kid"]);
    }
}
