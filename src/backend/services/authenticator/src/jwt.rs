use std::path::Path;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use toolkit_security::crypto::hash::Sha256;
use toolkit_security::crypto::signature::EcdsaP256KeyPair;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayClaims {
    pub sub: String,
    pub tenant_id: String,
    /// Serialized as one space-delimited string (OAuth `scope` convention): the downstream
    /// verifier (`cf-gears-oidc-authn-plugin`) reads `token_scopes` via
    /// `as_str().split_whitespace()`, so a JSON array would be dropped.
    #[serde(with = "space_delimited")]
    pub roles: Vec<String>,
    pub sub_type: String,
    pub sid: String,
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAccessClaims {
    pub sub: String,
    pub tenant_id: String,
    #[serde(with = "space_delimited")]
    pub roles: Vec<String>,
    pub sub_type: String,
    pub sid: String,
    pub iss: String,
    pub aud: String,
    pub scope: String,
    pub iat: u64,
    pub exp: u64,
    pub jti: String,
}

struct LoadedKey {
    kid: String,
    signing: EcdsaP256KeyPair,
    #[allow(dead_code)]
    decoding: DecodingKey,
    jwk: serde_json::Value,
}

impl LoadedKey {
    fn from_pkcs8_pem(pem: &str) -> anyhow::Result<Self> {
        let signing = EcdsaP256KeyPair::from_pkcs8_pem(pem)
            .context("parse PKCS#8 EC P-256 private key (expected an ES256 signing key)")?;

        // Uncompressed SEC1 point: 0x04 || X(32) || Y(32).
        let point = signing.public_key();
        let (x, y) = point
            .get(1..)
            .context("EC public key is not an uncompressed point")?
            .split_at(32);
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

        let decoding = DecodingKey::from_ec_components(&x_b64, &y_b64)
            .context("build ES256 decoding key from EC components")?;

        Ok(Self {
            kid,
            signing,
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

pub struct KeyStore {
    current: LoadedKey,
    previous: Option<LoadedKey>,
}

impl KeyStore {
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

    #[must_use]
    pub fn jwks(&self) -> serde_json::Value {
        let mut keys = vec![self.current.jwk.clone()];
        if let Some(prev) = &self.previous {
            keys.push(prev.jwk.clone());
        }
        serde_json::json!({ "keys": keys })
    }

    pub fn sign(&self, claims: &GatewayClaims) -> anyhow::Result<String> {
        anyhow::ensure!(
            !claims.tenant_id.trim().is_empty(),
            "refusing to sign a gateway JWT with an empty tenant_id (sub={})",
            claims.sub
        );
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.current.kid.clone());
        toolkit_security::jwt::encode(&header, claims, &self.current.signing)
            .context("sign gateway JWT")
    }

    pub fn sign_mcp_access_token(&self, claims: &McpAccessClaims) -> anyhow::Result<String> {
        anyhow::ensure!(
            !claims.tenant_id.trim().is_empty(),
            "refusing to sign an MCP access token with an empty tenant_id"
        );
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.current.kid.clone());
        toolkit_security::jwt::encode(&header, claims, &self.current.signing)
            .context("sign MCP access token")
    }

    #[cfg(test)]
    pub(crate) fn from_pem_for_test(current_pem: &str) -> anyhow::Result<Self> {
        Ok(Self {
            current: LoadedKey::from_pkcs8_pem(current_pem)?,
            previous: None,
        })
    }

    #[allow(dead_code)] // exercised by unit tests; kept as the in-process verify path
    pub fn verify(&self, token: &str, audience: &str) -> anyhow::Result<GatewayClaims> {
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&[audience]);
        validation.validate_exp = true;

        let mut last_err = None;
        for key in std::iter::once(&self.current).chain(self.previous.iter()) {
            match toolkit_security::jwt::decode::<GatewayClaims>(token, &key.decoding, &validation)
            {
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

mod space_delimited {
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(roles: &[String], ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&roles.join(" "))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<String>, D::Error> {
        let raw = String::deserialize(de)?;
        Ok(raw.split_whitespace().map(str::to_owned).collect())
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
            tenant_id: "t-a".to_owned(),
            roles: vec!["user".to_owned(), "admin".to_owned()],
            sub_type: "user".to_owned(),
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

        let header = jsonwebtoken::decode_header(&jwt).unwrap();
        assert_eq!(header.alg, Algorithm::ES256);
        assert!(header.kid.is_some());

        let claims = store.verify(&jwt, "internal-services").unwrap();
        assert_eq!(claims.sub, "person-123");
        assert_eq!(claims.tenant_id, "t-a");
        assert_eq!(claims.roles, vec!["user", "admin"]);
        assert_eq!(claims.sub_type, "user");
        assert_eq!(claims.aud, "internal-services");
    }

    #[test]
    fn sign_refuses_empty_tenant() {
        let store = KeyStore::from_pem_for_test(&gen_pem()).unwrap();
        let mut claims = sample_claims();
        claims.tenant_id = String::new();
        assert!(
            store.sign(&claims).is_err(),
            "empty tenant_id must be refused"
        );
        claims.tenant_id = "   ".to_owned();
        assert!(
            store.sign(&claims).is_err(),
            "whitespace tenant_id must be refused"
        );
    }

    #[test]
    fn roles_serialize_as_space_delimited_string() {
        let json = serde_json::to_value(sample_claims()).unwrap();
        assert_eq!(json["roles"], serde_json::json!("user admin"));

        let back: GatewayClaims = serde_json::from_value(json).unwrap();
        assert_eq!(back.roles, vec!["user", "admin"]);
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
    fn published_jwk_verifies_tokens_independently_of_the_facade() {
        use p256::elliptic_curve::sec1::ToSec1Point as _;
        use p256::pkcs8::DecodePrivateKey as _;
        let pem = gen_pem();
        let store = KeyStore::from_pem_for_test(&pem).unwrap();
        let jwks = store.jwks();
        let jwk = &jwks["keys"][0];

        let point = p256::SecretKey::from_pkcs8_pem(&pem)
            .unwrap()
            .public_key()
            .to_sec1_point(false);
        assert_eq!(jwk["x"], B64.encode(point.x().unwrap()));
        assert_eq!(jwk["y"], B64.encode(point.y().unwrap()));

        let set: jsonwebtoken::jwk::JwkSet = serde_json::from_value(jwks.clone()).unwrap();
        let key = DecodingKey::from_jwk(&set.keys[0]).unwrap();
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_audience(&["internal-services"]);
        let jwt = store.sign(&sample_claims()).unwrap();
        let data = jsonwebtoken::decode::<GatewayClaims>(&jwt, &key, &validation).unwrap();
        assert_eq!(data.claims.sub, "person-123");
        assert_eq!(data.header.kid.as_deref(), jwk["kid"].as_str());
    }

    #[test]
    fn kid_is_stable_for_a_key() {
        let pem = gen_pem();
        let a = KeyStore::from_pem_for_test(&pem).unwrap();
        let b = KeyStore::from_pem_for_test(&pem).unwrap();
        assert_eq!(a.jwks()["keys"][0]["kid"], b.jwks()["keys"][0]["kid"]);
    }
}
