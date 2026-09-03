use serde::{Deserialize, Serialize};
use url::Url;

pub const MCP_SCOPE: &str = "mcp:query";

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
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RefreshGrant {
    pub client_id: String,
    pub session_id: String,
    pub resource: String,
    pub scope: String,
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
