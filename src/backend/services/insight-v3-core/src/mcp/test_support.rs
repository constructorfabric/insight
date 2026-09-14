//! A local issuer for the tests: it serves a JWKS and signs tokens against it,
//! so the verifier is exercised against real ES256 material rather than a stub.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use p256::SecretKey;
use p256::elliptic_curve::Generate as _;
use p256::elliptic_curve::sec1::ToSec1Point as _;
use p256::pkcs8::{EncodePrivateKey as _, LineEnding};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::auth::{MCP_PATH, MCP_SCOPE, TokenVerifier};

pub(crate) struct Issuer {
    pub(crate) origin: String,
    key: EncodingKey,
    server: JoinHandle<()>,
}

impl Issuer {
    pub(crate) async fn start() -> Self {
        let secret = SecretKey::generate();

        let Ok(pem) = secret.to_pkcs8_pem(LineEnding::LF) else {
            panic!("a generated key encodes as PKCS#8 PEM");
        };
        let Ok(key) = EncodingKey::from_ec_pem(pem.as_bytes()) else {
            panic!("a PKCS#8 PEM is a usable EC signing key");
        };

        let point = secret.public_key().to_sec1_point(false);
        let (Some(x), Some(y)) = (point.x(), point.y()) else {
            panic!("an uncompressed SEC1 point carries both coordinates");
        };

        let jwks = json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "use": "sig",
                "alg": "ES256",
                "kid": "test-key",
                "x": B64.encode(x),
                "y": B64.encode(y),
            }]
        });

        let app = axum::Router::new().route(
            "/.well-known/jwks.json",
            axum::routing::get(move || {
                let jwks = jwks.clone();
                async move { axum::Json(jwks) }
            }),
        );

        let Ok(listener) = tokio::net::TcpListener::bind("127.0.0.1:0").await else {
            panic!("an ephemeral loopback port is bindable");
        };
        let Ok(address) = listener.local_addr() else {
            panic!("a bound listener has a local address");
        };
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            origin: format!("http://{address}"),
            key,
            server,
        }
    }

    pub(crate) fn verifier(&self) -> TokenVerifier {
        let Ok(verifier) = TokenVerifier::new(&self.origin, "", true) else {
            panic!("a loopback origin is an allowed public URL");
        };

        verifier
    }

    pub(crate) fn claims(&self) -> Value {
        let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            panic!("the clock is after the epoch");
        };
        let now = now.as_secs();

        json!({
            "sub": "test-user",
            "tenant_id": "test-tenant",
            "roles": "user admin",
            "sub_type": "user",
            "sid": "test-session",
            "iss": self.origin,
            "aud": format!("{}{MCP_PATH}", self.origin),
            "scope": format!("openid {MCP_SCOPE}"),
            "iat": now,
            "exp": now + 600,
            "jti": "test-token",
        })
    }

    pub(crate) fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_owned());

        let Ok(token) = encode(&header, claims, &self.key) else {
            panic!("the claims encode against the signing key");
        };

        token
    }

    pub(crate) fn stop(self) {
        self.server.abort();
    }
}
