//! End-to-end service-token loop against a running authenticator (nginx+auth
//! step 06). `#[ignore]` by default — needs the stack up (Redis + authenticator
//! with the dev `testclient` registry). `run-e2e.sh` brings it up and runs it.
//!
//! It drives the whole flow through the SDK `ServiceTokenClient`:
//!   1. obtain a tenant-scoped service token and verify it against the JWKS
//!      (`sub = sid = service:testclient`, `roles` includes `service`,
//!      `tenant_id = tenant-a`);
//!   2. a replayed assertion is rejected (single-use `jti`);
//!   3. an assertion signed with a key the registry does not hold is rejected;
//!   4. a request that names no tenant is rejected (tenant isolation).
//!
//! ```text
//! AUTH_BASE=http://localhost:8083 TOKEN_ENDPOINT=http://localhost:8093/internal/token \
//!   cargo test -p authenticator --test e2e_service_token -- --ignored --nocapture
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use authenticator_sdk::ServiceTokenClient;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::Deserialize;

fn env(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_owned())
}

fn auth_base() -> String {
    env("AUTH_BASE", "http://localhost:8083")
}

fn token_endpoint() -> String {
    env("TOKEN_ENDPOINT", "http://localhost:8093/internal/token")
}

/// Path to the generated dev private key matching the `testclient` registry
/// entry. run-e2e.sh generates the keypair and exports `SVC_KEY` (no key
/// material is committed).
fn testclient_key_path() -> String {
    std::env::var("SVC_KEY").expect("SVC_KEY must point at the generated testclient private key")
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}
#[derive(Deserialize)]
struct Jwk {
    x: String,
    y: String,
    #[serde(default)]
    kid: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
    tenant_id: String,
    roles: Vec<String>,
    sid: String,
    aud: String,
}

/// Fetch + verify a gateway JWT against the authenticator's published JWKS.
async fn verify_against_jwks(jwt: &str) -> Claims {
    let http = reqwest::Client::new();
    let jwks: Jwks = http
        .get(format!("{}/.well-known/jwks.json", auth_base()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let header = jsonwebtoken::decode_header(jwt).unwrap();
    assert_eq!(header.alg, Algorithm::ES256);
    let jwk = jwks
        .keys
        .iter()
        .find(|k| header.kid.is_none() || k.kid == header.kid)
        .expect("a JWKS key matching the token kid");
    let decoding = DecodingKey::from_ec_components(&jwk.x, &jwk.y).unwrap();
    let mut validation = Validation::new(Algorithm::ES256);
    validation.set_audience(&["internal-services"]);
    decode::<Claims>(jwt, &decoding, &validation)
        .expect("service JWT verifies against the JWKS")
        .claims
}

#[tokio::test]
#[ignore = "requires a running authenticator + Redis stack with the dev registry"]
async fn service_token_full_loop() {
    let endpoint = token_endpoint();
    let client =
        ServiceTokenClient::from_key_file("testclient", testclient_key_path(), &endpoint).unwrap();
    let tenant = "tenant-a";

    // 1. Obtain a (tenant-scoped) service token and verify it against the JWKS.
    let bearer = client
        .bearer(tenant)
        .await
        .expect("bearer() should mint a tenant-scoped token");
    let jwt = bearer
        .strip_prefix("Bearer ")
        .expect("bearer() returns a Bearer value");
    let claims = verify_against_jwks(jwt).await;
    assert_eq!(claims.sub, "service:testclient", "sub is service:<name>");
    assert_eq!(claims.sid, "service:testclient", "sid is service:<name>");
    assert_eq!(claims.aud, "internal-services");
    assert!(
        claims.roles.contains(&"service".to_owned()),
        "service role always present, got {:?}",
        claims.roles
    );
    assert_eq!(
        claims.tenant_id, tenant,
        "the requested tenant is carried in the token"
    );

    // 2. Replay: the same assertion must be accepted once, then rejected.
    let assertion = client.make_assertion().unwrap();
    client
        .post(&assertion, tenant)
        .await
        .expect("first use of an assertion succeeds");
    assert!(
        client.post(&assertion, tenant).await.is_err(),
        "a replayed assertion must be rejected"
    );

    // 3. Wrong key: an assertion signed by a key the registry does not hold is
    //    rejected, even though it claims iss/sub = testclient.
    let (wrong_priv, _) = generate_keypair();
    let impostor = ServiceTokenClient::from_key_pem("testclient", &wrong_priv, &endpoint).unwrap();
    let forged = impostor.make_assertion().unwrap();
    assert!(
        impostor.post(&forged, tenant).await.is_err(),
        "an assertion signed with an unregistered key must be rejected"
    );

    // 4. Tenant isolation: a request that names no tenant is refused (400).
    assert!(
        client.fetch("").await.is_err(),
        "a service token must name a tenant; an unscoped request is rejected"
    );
}

/// A fresh P-256 keypair: (PKCS#8 private PEM, SPKI public PEM).
fn generate_keypair() -> (String, String) {
    use p256::SecretKey;
    use p256::elliptic_curve::Generate as _;
    use p256::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _, LineEnding};
    let secret = SecretKey::generate();
    let priv_pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let pub_pem = secret
        .public_key()
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    (priv_pem, pub_pem)
}
