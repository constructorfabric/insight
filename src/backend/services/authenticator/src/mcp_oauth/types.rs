use serde::{Deserialize, Serialize};
use url::Url;

pub const MCP_SCOPE: &str = "mcp:query";
pub const GRANT_LIFETIME_SECONDS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Deserialize)]
pub struct ClientRegistrationRequest {
    pub client_name: Option<String>,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub grant_types: Vec<String>,
    #[serde(default)]
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegisteredClient {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    pub grant_types: Vec<String>,
    pub response_types: Vec<String>,
    pub token_endpoint_auth_method: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub request_id: Option<String>,
    pub response_type: Option<String>,
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub state: Option<String>,
    pub resource: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PendingAuthorization {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub state: String,
    pub resource: String,
    pub scope: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizationDecision {
    pub request_id: String,
    pub decision: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthorizationCodeGrant {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub session_id: String,
    pub resource: String,
    pub scope: String,
    pub expires_at: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RefreshGrant {
    pub client_id: String,
    pub grant_id: String,
    pub person_id: String,
    pub tenant_id: String,
    pub resource: String,
    pub scope: String,
    pub expires_at: u64,
}

impl RefreshGrant {
    pub fn remaining_seconds(&self, now: u64) -> Option<u64> {
        let remaining = self.expires_at.saturating_sub(now);
        (remaining > 0).then_some(remaining)
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub client_id: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    pub resource: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevocationRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub refresh_token: String,
    pub scope: String,
}

#[derive(Debug, Serialize)]
pub struct OAuthError {
    pub error: &'static str,
    pub error_description: String,
}

pub fn validated_registration(
    request: ClientRegistrationRequest,
    client_id: String,
) -> Result<RegisteredClient, &'static str> {
    if request.redirect_uris.is_empty() || request.redirect_uris.len() > 8 {
        return Err("redirect_uris must contain between 1 and 8 entries");
    }
    if request
        .redirect_uris
        .iter()
        .any(|redirect_uri| !valid_redirect_uri(redirect_uri))
    {
        return Err("redirect_uris contains an unsupported URI");
    }
    if !request.grant_types.is_empty()
        && request
            .grant_types
            .iter()
            .any(|grant| !matches!(grant.as_str(), "authorization_code" | "refresh_token"))
    {
        return Err("only authorization_code and refresh_token grants are supported");
    }
    if !request.response_types.is_empty()
        && request
            .response_types
            .iter()
            .any(|response| response != "code")
    {
        return Err("only the code response type is supported");
    }
    if request
        .token_endpoint_auth_method
        .as_deref()
        .is_some_and(|method| method != "none")
    {
        return Err("only public clients using token_endpoint_auth_method=none are supported");
    }

    let client_name = request
        .client_name
        .unwrap_or_else(|| "MCP client".to_owned())
        .trim()
        .chars()
        .take(100)
        .collect::<String>();
    if client_name.is_empty() {
        return Err("client_name must not be empty");
    }

    Ok(RegisteredClient {
        client_id,
        client_name,
        redirect_uris: request.redirect_uris,
        grant_types: vec!["authorization_code".to_owned(), "refresh_token".to_owned()],
        response_types: vec!["code".to_owned()],
        token_endpoint_auth_method: "none".to_owned(),
    })
}

pub fn redirect_uri_matches(registered: &str, presented: &str) -> bool {
    if registered == presented {
        return true;
    }
    let (Ok(registered), Ok(presented)) = (Url::parse(registered), Url::parse(presented)) else {
        return false;
    };
    is_loopback(&registered)
        && is_loopback(&presented)
        && registered.scheme() == presented.scheme()
        && registered.host_str() == presented.host_str()
        && registered.path() == presented.path()
        && registered.query() == presented.query()
}

pub fn valid_code_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
}

fn valid_redirect_uri(value: &str) -> bool {
    let Ok(uri) = Url::parse(value) else {
        return false;
    };
    uri.fragment().is_none()
        && uri.username().is_empty()
        && uri.password().is_none()
        && (uri.scheme() == "https" || is_loopback(&uri))
}

fn is_loopback(uri: &Url) -> bool {
    uri.scheme() == "http" && matches!(uri.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::{
        ClientRegistrationRequest, redirect_uri_matches, valid_code_verifier,
        validated_registration,
    };

    type R = Result<(), Box<dyn Error>>;

    #[test]
    fn registration_applies_supported_public_client_metadata() -> R {
        let request = ClientRegistrationRequest {
            client_name: Some(" Codex ".to_owned()),
            redirect_uris: vec!["http://127.0.0.1:3000/callback".to_owned()],
            grant_types: Vec::new(),
            response_types: Vec::new(),
            token_endpoint_auth_method: None,
        };

        let client = validated_registration(request, "client-id".to_owned())
            .map_err(std::io::Error::other)?;

        assert_eq!(client.client_name, "Codex");
        assert_eq!(client.grant_types, ["authorization_code", "refresh_token"]);
        assert_eq!(client.response_types, ["code"]);
        assert_eq!(client.token_endpoint_auth_method, "none");
        Ok(())
    }

    #[test]
    fn registration_rejects_insecure_non_loopback_redirect() {
        let request = ClientRegistrationRequest {
            client_name: Some("MCP client".to_owned()),
            redirect_uris: vec!["http://client.example/callback".to_owned()],
            grant_types: Vec::new(),
            response_types: Vec::new(),
            token_endpoint_auth_method: None,
        };

        assert!(matches!(
            validated_registration(request, "client-id".to_owned()),
            Err("redirect_uris contains an unsupported URI")
        ));
    }

    #[test]
    fn loopback_redirect_allows_dynamic_port_only() {
        assert!(redirect_uri_matches(
            "http://127.0.0.1:3000/callback?flow=mcp",
            "http://127.0.0.1:49152/callback?flow=mcp"
        ));
        assert!(!redirect_uri_matches(
            "http://127.0.0.1:3000/callback?flow=mcp",
            "http://127.0.0.1:49152/other?flow=mcp"
        ));
    }

    #[test]
    fn pkce_verifier_accepts_only_rfc_unreserved_boundary_values() {
        assert!(valid_code_verifier(&"a".repeat(43)));
        assert!(valid_code_verifier(&"~".repeat(128)));
        assert!(!valid_code_verifier(&"a".repeat(42)));
        assert!(!valid_code_verifier(&"a".repeat(129)));
        assert!(!valid_code_verifier(&format!("{}+", "a".repeat(42))));
    }
}
