use std::time::Duration;

use axum::extract::{FromRequestParts, Query};
use axum::http::HeaderMap;
use axum::http::request::Parts;
use serde::de::DeserializeOwned;

use crate::engine::key::CacheKey;
use crate::engine::page::PageToken;
use crate::engine::runner::GitCredentials;
use crate::engine::url::{CloneUrl, CloneUrlError, CloneUrlPolicy};

use super::error::ApiError;

pub const TENANT_HEADER: &str = "x-tenant-id";
pub const SOURCE_HEADER: &str = "x-source-id";
pub const GIT_USER_HEADER: &str = "x-git-username";
pub const GIT_TOKEN_HEADER: &str = "x-git-token";
pub const STALENESS_HEADER: &str = "x-max-staleness";

/// Ceiling and default are the same on purpose. Every memory bound on the
/// request path scales linearly with the page, so headroom above the default
/// is pure exposure: nothing has ever needed a larger page, `page_size` is a
/// service-to-connector knob rather than tenant configuration, and the row
/// caps cut an oversized page at emit time anyway — after the memory was
/// already spent reading it. Smaller pages remain available for debugging.
const MAX_PAGE_SIZE: usize = 1_000;
const DEFAULT_PAGE_SIZE: usize = 1_000;
const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PATCH_BYTES: usize = 1024 * 1024;

const MIN_SHA_PREFIX: usize = 7;
/// A full object id: 40 hex characters under SHA-1, 64 under SHA-256. Anything
/// longer is not a prefix of any object id and can only ever select nothing.
const MAX_SHA_PREFIX: usize = 64;

/// Explicit commit selection: full ids or hex prefixes of at least
/// [`MIN_SHA_PREFIX`] characters, comma separated (§4.2). A prefix selects
/// every commit it matches — it is not required to be unique, and the service
/// does not resolve it against the repository. A debugging and incident-review
/// affordance; the sync path pages by cursor instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaFilter {
    prefixes: Vec<String>,
}

impl ShaFilter {
    /// # Errors
    ///
    /// [`BadRequest`] when an entry is not hex or is too short to identify a
    /// commit.
    pub fn parse(raw: &str) -> Result<Self, BadRequest> {
        let prefixes: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_ascii_lowercase)
            .collect();

        if prefixes.is_empty() {
            return Err(BadRequest::MissingParam("sha"));
        }
        for prefix in &prefixes {
            if !(MIN_SHA_PREFIX..=MAX_SHA_PREFIX).contains(&prefix.len())
                || !prefix.chars().all(|c| c.is_ascii_hexdigit())
            {
                return Err(BadRequest::MalformedSha(prefix.clone()));
            }
        }
        Ok(Self { prefixes })
    }

    #[must_use]
    pub fn matches(&self, sha: &str) -> bool {
        let sha = sha.to_ascii_lowercase();
        self.prefixes.iter().any(|prefix| sha.starts_with(prefix))
    }

    /// The lowercase prefixes, for the index reader, whose query type cannot
    /// carry a borrow into `spawn_blocking`.
    #[must_use]
    pub fn prefixes(&self) -> Vec<String> {
        self.prefixes.clone()
    }
}

/// Parse the optional `sha` query parameter.
///
/// # Errors
///
/// [`BadRequest`] when the value is present but malformed.
pub fn parse_sha_filter(raw: Option<&str>) -> Result<Option<ShaFilter>, BadRequest> {
    match raw.filter(|value| !value.trim().is_empty()) {
        Some(value) => ShaFilter::parse(value).map(Some),
        None => Ok(None),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BadRequest {
    #[error("missing or empty header: {0}")]
    MissingHeader(&'static str),
    #[error("missing or empty query parameter: {0}")]
    MissingParam(&'static str),
    #[error("malformed page token")]
    MalformedToken,
    #[error("{0} is not a valid number")]
    NotANumber(&'static str),
    #[error("`sha` entry is not a hex commit id of at least 7 characters: {0}")]
    MalformedSha(String),
    #[error(transparent)]
    BadRepoUrl(#[from] CloneUrlError),
    #[error("query parameters could not be parsed")]
    MalformedQuery,
}

/// `Query<T>`, but a rejection leaves through [`ApiError`] like every other
/// failure this API has.
///
/// Axum's own rejection is `text/plain` and escapes the RFC 9457 envelope the
/// contract promises — and it happens BEFORE the handler runs, so nothing
/// downstream can repair it.
pub struct ValidatedQuery<T>(pub T);

impl<T, S> FromRequestParts<S> for ValidatedQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // The rejection's own message is not forwarded: it quotes the caller's
        // query string back at them, and this envelope is not the place for it.
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| BadRequest::MalformedQuery.into())
    }
}

/// A query parameter the route cannot proceed without.
///
/// # Errors
///
/// [`BadRequest::MissingParam`] naming `name` when it is absent or blank.
pub fn required_param<'a>(
    value: Option<&'a str>,
    name: &'static str,
) -> Result<&'a str, BadRequest> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or(BadRequest::MissingParam(name))
}

/// Everything a data request carries besides its own filters: who is asking,
/// which repository, and how fresh the snapshot must be.
#[derive(Debug)]
pub struct RequestContext {
    pub key: CacheKey,
    pub creds: GitCredentials,
    pub max_staleness: Option<Duration>,
}

impl RequestContext {
    /// # Errors
    ///
    /// [`BadRequest`] when an identity/credential header is absent or empty, or
    /// when `repo` is not an origin this service will clone from.
    pub fn from_parts(
        headers: &HeaderMap,
        clone_url: &str,
        policy: CloneUrlPolicy,
    ) -> Result<Self, BadRequest> {
        let tenant_id = required_header(headers, TENANT_HEADER)?;
        let source_id = required_header(headers, SOURCE_HEADER)?;
        let username = required_header(headers, GIT_USER_HEADER)?;
        let token = required_header(headers, GIT_TOKEN_HEADER)?;

        Ok(Self {
            key: CacheKey {
                tenant_id,
                source_id,
                clone_url: CloneUrl::parse(clone_url, policy)?,
            },
            creds: GitCredentials { username, token },
            max_staleness: staleness(headers)?,
        })
    }
}

/// Paging inputs shared by the cursor-paginated endpoints.
#[derive(Debug)]
pub struct Paging {
    pub token: Option<PageToken>,
    pub page_size: usize,
}

impl Paging {
    /// # Errors
    ///
    /// [`BadRequest`] when the token is not one this service issued.
    pub fn parse(page_token: Option<&str>, page_size: Option<u32>) -> Result<Self, BadRequest> {
        let token = match page_token.filter(|raw| !raw.is_empty()) {
            Some(raw) => Some(PageToken::decode(raw).map_err(|_| BadRequest::MalformedToken)?),
            None => None,
        };
        Ok(Self {
            token,
            page_size: clamp_page_size(page_size),
        })
    }
}

#[must_use]
pub fn clamp_page_size(requested: Option<u32>) -> usize {
    match requested {
        Some(0) | None => DEFAULT_PAGE_SIZE,
        Some(value) => (value as usize).min(MAX_PAGE_SIZE),
    }
}

#[must_use]
pub fn clamp_patch_bytes(requested: Option<u64>) -> usize {
    match requested {
        Some(0) | None => DEFAULT_PATCH_BYTES,
        Some(value) => usize::try_from(value)
            .unwrap_or(MAX_PATCH_BYTES)
            .min(MAX_PATCH_BYTES),
    }
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Result<String, BadRequest> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or(BadRequest::MissingHeader(name))
}

fn staleness(headers: &HeaderMap) -> Result<Option<Duration>, BadRequest> {
    let Some(raw) = headers
        .get(STALENESS_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let seconds: u64 = raw
        .parse()
        .map_err(|_| BadRequest::NotANumber(STALENESS_HEADER))?;
    Ok(Some(Duration::from_secs(seconds)))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn full_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(TENANT_HEADER, HeaderValue::from_static("tenant-1"));
        headers.insert(SOURCE_HEADER, HeaderValue::from_static("source-1"));
        headers.insert(GIT_USER_HEADER, HeaderValue::from_static("oauth2"));
        headers.insert(GIT_TOKEN_HEADER, HeaderValue::from_static("pat"));
        headers
    }

    const HTTP_ONLY: CloneUrlPolicy = CloneUrlPolicy::http_only();

    #[test]
    fn builds_a_context_from_complete_headers() {
        let Ok(context) =
            RequestContext::from_parts(&full_headers(), "https://example.com/a.git", HTTP_ONLY)
        else {
            panic!("complete headers must parse")
        };
        assert_eq!(context.key.tenant_id, "tenant-1");
        assert_eq!(context.key.source_id, "source-1");
        assert_eq!(context.key.clone_url.as_str(), "https://example.com/a.git");
        assert_eq!(context.creds.username, "oauth2");
        assert_eq!(
            context.max_staleness, None,
            "absent header means no override"
        );
    }

    #[test]
    fn each_missing_identity_header_is_rejected() {
        for name in [
            TENANT_HEADER,
            SOURCE_HEADER,
            GIT_USER_HEADER,
            GIT_TOKEN_HEADER,
        ] {
            let mut headers = full_headers();
            headers.remove(name);
            let outcome =
                RequestContext::from_parts(&headers, "https://example.com/a.git", HTTP_ONLY);
            match outcome {
                Err(BadRequest::MissingHeader(missing)) => assert_eq!(missing, name),
                Err(e) => panic!("wrong error for {name}: {e}"),
                Ok(_) => panic!("must reject a request without {name}"),
            }
        }
    }

    #[test]
    fn blank_headers_count_as_missing() {
        let mut headers = full_headers();
        headers.insert(TENANT_HEADER, HeaderValue::from_static("   "));
        assert!(
            RequestContext::from_parts(&headers, "https://example.com/a.git", HTTP_ONLY).is_err(),
            "whitespace is not an identity"
        );
    }

    #[test]
    fn empty_repo_parameter_is_rejected() {
        let outcome = RequestContext::from_parts(&full_headers(), "  ", HTTP_ONLY);
        match outcome {
            Err(BadRequest::BadRepoUrl(CloneUrlError::Empty)) => {}
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("must reject an empty repo"),
        }
    }

    #[test]
    fn a_non_http_repo_never_reaches_the_cache_key() {
        for raw in ["ext::sh -c id", "file:///tmp/x", "/etc/passwd", "-u/tmp/x"] {
            let outcome = RequestContext::from_parts(&full_headers(), raw, HTTP_ONLY);
            assert!(
                matches!(outcome, Err(BadRequest::BadRepoUrl(_))),
                "should reject: {raw:?}"
            );
        }
    }

    #[test]
    fn staleness_header_parses_and_validates() {
        let mut headers = full_headers();
        headers.insert(STALENESS_HEADER, HeaderValue::from_static("120"));
        let repo = "https://example.com/a.git";
        let Ok(context) = RequestContext::from_parts(&headers, repo, HTTP_ONLY) else {
            panic!("numeric staleness must parse")
        };
        assert_eq!(context.max_staleness, Some(Duration::from_mins(2)));

        headers.insert(STALENESS_HEADER, HeaderValue::from_static("soon"));
        match RequestContext::from_parts(&headers, repo, HTTP_ONLY) {
            Err(BadRequest::NotANumber(name)) => assert_eq!(name, STALENESS_HEADER),
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("non-numeric staleness must be rejected"),
        }
    }

    #[test]
    fn sha_filter_accepts_full_ids_and_prefixes() {
        let Ok(filter) = ShaFilter::parse("ABC1234, def5678901234567890123456789012345678901")
        else {
            panic!("a comma-separated list must parse")
        };
        assert!(
            filter.matches("abc1234def"),
            "prefix match, case-insensitive"
        );
        assert!(filter.matches("DEF5678901234567890123456789012345678901"));
        assert!(!filter.matches("999999999"), "unrelated sha must not match");
    }

    #[test]
    fn sha_filter_rejects_unusable_entries() {
        let long = "a".repeat(65);
        let cases = vec![
            ("too short", "abc123"),
            ("not hex", "zzzzzzzz"),
            ("only separators", ",,"),
            ("empty", ""),
            // Longer than any object id: it can never prefix-match, so it
            // used to be accepted and then silently return no rows.
            ("longer than an object id", long.as_str()),
        ];
        for (name, raw) in cases {
            assert!(ShaFilter::parse(raw).is_err(), "must reject: {name}");
        }

        let sha256 = "b".repeat(64);
        assert!(
            ShaFilter::parse(&sha256).is_ok(),
            "a full SHA-256 id is exactly at the bound"
        );
    }

    #[test]
    fn absent_sha_parameter_is_not_an_error() {
        assert!(matches!(parse_sha_filter(None), Ok(None)));
        assert!(matches!(parse_sha_filter(Some("   ")), Ok(None)));
        assert!(parse_sha_filter(Some("abc1234")).is_ok_and(|f| f.is_some()));
    }

    #[test]
    fn page_size_is_clamped_to_the_documented_bounds() {
        let cases = vec![
            ("absent", None, DEFAULT_PAGE_SIZE),
            ("zero", Some(0), DEFAULT_PAGE_SIZE),
            ("in range", Some(250), 250),
            ("above max", Some(50_000), MAX_PAGE_SIZE),
        ];
        for (name, requested, expected) in cases {
            assert_eq!(clamp_page_size(requested), expected, "case: {name}");
        }
    }

    #[test]
    fn patch_budget_is_clamped_to_the_documented_bounds() {
        let cases = vec![
            ("absent", None, DEFAULT_PATCH_BYTES),
            ("zero", Some(0), DEFAULT_PATCH_BYTES),
            ("in range", Some(4096), 4096),
            ("above max", Some(u64::MAX), MAX_PATCH_BYTES),
        ];
        for (name, requested, expected) in cases {
            assert_eq!(clamp_patch_bytes(requested), expected, "case: {name}");
        }
    }

    #[test]
    fn paging_rejects_foreign_tokens_but_accepts_its_own() {
        let token = PageToken {
            incarnation: "inc0".to_owned(),
            entry: String::new(),
            generation: 4,
            primary: "2026-08-01T00:00:00Z".to_owned(),
            secondary: "aaa".to_owned(),
        };
        let encoded = token.encode();
        let Ok(paging) = Paging::parse(Some(&encoded), Some(10)) else {
            panic!("own token must parse")
        };
        assert_eq!(paging.token, Some(token));
        assert_eq!(paging.page_size, 10);

        assert!(
            Paging::parse(Some("!!!"), None).is_err(),
            "garbage rejected"
        );
        let Ok(empty) = Paging::parse(Some(""), None) else {
            panic!("an empty token means first page")
        };
        assert_eq!(empty.token, None);
    }
}
