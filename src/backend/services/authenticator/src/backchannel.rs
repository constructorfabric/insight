use jsonwebtoken::errors::ErrorKind;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode_header};
use serde::Deserialize;

const LOGOUT_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";

#[derive(Debug)]
pub struct LogoutClaims {
    pub sub: Option<String>,
    pub sid: Option<String>,
    pub jti: String,
    pub iat: u64,
}

#[derive(Debug, Deserialize)]
struct RawClaims {
    iss: String,
    #[serde(default)]
    aud: serde_json::Value,
    iat: u64,
    jti: String,
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    sid: Option<String>,
    #[serde(default)]
    events: Option<serde_json::Value>,
    #[serde(default)]
    nonce: Option<serde_json::Value>,
}

pub fn validate_logout_token(
    jwks: &JwkSet,
    raw: &str,
    expected_iss: &str,
    expected_aud: &str,
    now: u64,
    clock_skew_seconds: u64,
    max_age_seconds: u64,
) -> Result<LogoutClaims, &'static str> {
    let header = decode_header(raw).map_err(|_| "malformed_token")?;
    let alg = header.alg;
    if !matches!(alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err("unsupported_alg");
    }

    let keys: Vec<&jsonwebtoken::jwk::Jwk> = match &header.kid {
        Some(kid) => jwks
            .keys
            .iter()
            .filter(|k| k.common.key_id.as_deref() == Some(kid))
            .collect(),
        None => jwks.keys.iter().collect(),
    };
    if keys.is_empty() {
        return Err("unknown_kid");
    }

    let mut validation = Validation::new(alg);
    // A logout token has no `exp` requirement; freshness is `iat`-based below.
    validation.validate_exp = false;
    validation.set_issuer(&[expected_iss]);
    validation.set_audience(&[expected_aud]);
    validation.set_required_spec_claims(&["iss", "aud", "iat", "jti"]);
    validation.leeway = clock_skew_seconds;

    let mut claims: Option<RawClaims> = None;
    for jwk in keys {
        let Ok(key) = DecodingKey::from_jwk(jwk) else {
            continue;
        };
        match toolkit_security::jwt::decode::<RawClaims>(raw, &key, &validation) {
            Ok(data) => {
                claims = Some(data.claims);
                break;
            }
            Err(error) if matches!(error.kind(), ErrorKind::Provider(_)) => {
                return Err("crypto_unavailable");
            }
            Err(_) => {}
        }
    }
    let claims = claims.ok_or("signature_verification_failed")?;

    if claims.iat > now + clock_skew_seconds {
        return Err("iat_in_future");
    }
    if now.saturating_sub(claims.iat) > max_age_seconds + clock_skew_seconds {
        return Err("token_too_old");
    }

    let has_event = claims
        .events
        .as_ref()
        .and_then(|e| e.as_object())
        .is_some_and(|o| o.contains_key(LOGOUT_EVENT));
    if !has_event {
        return Err("missing_backchannel_event");
    }

    // A logout token MUST NOT carry a nonce (it would be an id_token replay).
    if claims.nonce.is_some() {
        return Err("nonce_present");
    }

    if claims.sub.is_none() && claims.sid.is_none() {
        return Err("no_sub_or_sid");
    }

    let _ = claims.iss;
    let _ = claims.aud;

    Ok(LogoutClaims {
        sub: claims.sub,
        sid: claims.sid,
        jti: claims.jti,
        iat: claims.iat,
    })
}

#[must_use]
pub fn replay_guard_ttl(iat: u64, now: u64, clock_skew_seconds: u64, max_age_seconds: u64) -> u64 {
    (iat + max_age_seconds + clock_skew_seconds)
        .saturating_sub(now)
        .max(1)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use p256::SecretKey;
    use p256::elliptic_curve::Generate as _;
    use p256::elliptic_curve::sec1::ToSec1Point as _;
    use p256::pkcs8::{EncodePrivateKey as _, LineEnding};

    const ISS: &str = "https://idp.example";
    const AUD: &str = "insight-authenticator";
    const NOW: u64 = 1_000_000;

    fn material() -> (EncodingKey, JwkSet) {
        use base64::Engine as _;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
        let secret = SecretKey::generate();
        let pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap();
        let enc = EncodingKey::from_ec_pem(pem.as_bytes()).unwrap();
        let point = secret.public_key().to_sec1_point(false);
        let jwks: JwkSet = serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": "k1",
                "x": B64.encode(point.x().unwrap()),
                "y": B64.encode(point.y().unwrap()),
            }]
        }))
        .unwrap();
        (enc, jwks)
    }

    fn sign(enc: &EncodingKey, claims: &serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("k1".to_owned());
        encode(&header, claims, enc).unwrap()
    }

    fn base_claims() -> serde_json::Value {
        serde_json::json!({
            "iss": ISS, "aud": AUD, "iat": NOW - 10, "jti": "jti-1",
            "sub": "user-1", "sid": "idp-sid-1",
            "events": { "http://schemas.openid.net/event/backchannel-logout": {} },
        })
    }

    fn validate(jwks: &JwkSet, raw: &str) -> Result<LogoutClaims, &'static str> {
        validate_logout_token(jwks, raw, ISS, AUD, NOW, 60, 300)
    }

    #[test]
    fn accepts_a_valid_logout_token() {
        let (enc, jwks) = material();
        let token = sign(&enc, &base_claims());
        let claims = validate(&jwks, &token).unwrap();
        assert_eq!(claims.sub.as_deref(), Some("user-1"));
        assert_eq!(claims.sid.as_deref(), Some("idp-sid-1"));
        assert_eq!(claims.jti, "jti-1");
    }

    #[test]
    fn rejects_wrong_issuer_audience_and_signature() {
        let (enc, jwks) = material();
        let mut c = base_claims();
        c["iss"] = "https://evil.example".into();
        assert!(validate(&jwks, &sign(&enc, &c)).is_err());

        let mut c = base_claims();
        c["aud"] = "someone-else".into();
        assert!(validate(&jwks, &sign(&enc, &c)).is_err());

        let (other_enc, _) = material();
        assert_eq!(
            validate(&jwks, &sign(&other_enc, &base_claims())).unwrap_err(),
            "signature_verification_failed"
        );
    }

    #[test]
    fn rejects_missing_event_and_present_nonce() {
        let (enc, jwks) = material();
        let mut c = base_claims();
        c["events"] = serde_json::json!({ "urn:other": {} });
        assert_eq!(
            validate(&jwks, &sign(&enc, &c)).unwrap_err(),
            "missing_backchannel_event"
        );

        let mut c = base_claims();
        c["nonce"] = "n-1".into();
        assert_eq!(
            validate(&jwks, &sign(&enc, &c)).unwrap_err(),
            "nonce_present"
        );
    }

    #[test]
    fn rejects_stale_and_future_iat() {
        let (enc, jwks) = material();
        let mut c = base_claims();
        c["iat"] = (NOW - 1000).into();
        assert_eq!(
            validate(&jwks, &sign(&enc, &c)).unwrap_err(),
            "token_too_old"
        );

        let mut c = base_claims();
        c["iat"] = (NOW + 500).into();
        assert_eq!(
            validate(&jwks, &sign(&enc, &c)).unwrap_err(),
            "iat_in_future"
        );
    }

    #[test]
    fn requires_sub_or_sid() {
        let (enc, jwks) = material();
        let mut c = base_claims();
        c.as_object_mut().unwrap().remove("sub");
        c.as_object_mut().unwrap().remove("sid");
        assert_eq!(
            validate(&jwks, &sign(&enc, &c)).unwrap_err(),
            "no_sub_or_sid"
        );

        let mut c = base_claims();
        c.as_object_mut().unwrap().remove("sid");
        assert!(validate(&jwks, &sign(&enc, &c)).is_ok());
        let mut c = base_claims();
        c.as_object_mut().unwrap().remove("sub");
        assert!(validate(&jwks, &sign(&enc, &c)).is_ok());
    }

    #[test]
    fn replay_ttl_covers_the_acceptability_window() {
        assert_eq!(replay_guard_ttl(NOW - 10, NOW, 60, 300), 350);
        assert_eq!(replay_guard_ttl(0, NOW, 60, 300), 1);
    }
}
