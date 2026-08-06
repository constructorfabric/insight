use std::time::Duration;

use axum::http::HeaderMap;

use crate::engine::key::CacheKey;
use crate::engine::page::PageToken;
use crate::engine::runner::GitCredentials;

pub const TENANT_HEADER: &str = "x-tenant-id";
pub const SOURCE_HEADER: &str = "x-source-id";
pub const GIT_USER_HEADER: &str = "x-git-username";
pub const GIT_TOKEN_HEADER: &str = "x-git-token";
pub const STALENESS_HEADER: &str = "x-max-staleness";

const MAX_PAGE_SIZE: usize = 10_000;
const DEFAULT_PAGE_SIZE: usize = 1_000;
const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_PATCH_BYTES: usize = 1024 * 1024;

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
    /// [`BadRequest`] when an identity/credential header is absent or empty.
    pub fn from_parts(headers: &HeaderMap, clone_url: &str) -> Result<Self, BadRequest> {
        let tenant_id = required_header(headers, TENANT_HEADER)?;
        let source_id = required_header(headers, SOURCE_HEADER)?;
        let username = required_header(headers, GIT_USER_HEADER)?;
        let token = required_header(headers, GIT_TOKEN_HEADER)?;

        if clone_url.trim().is_empty() {
            return Err(BadRequest::MissingParam("repo"));
        }

        Ok(Self {
            key: CacheKey {
                tenant_id,
                source_id,
                clone_url: clone_url.to_owned(),
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

    #[test]
    fn builds_a_context_from_complete_headers() {
        let Ok(context) = RequestContext::from_parts(&full_headers(), "https://example.com/a.git")
        else {
            panic!("complete headers must parse")
        };
        assert_eq!(context.key.tenant_id, "tenant-1");
        assert_eq!(context.key.source_id, "source-1");
        assert_eq!(context.key.clone_url, "https://example.com/a.git");
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
            let outcome = RequestContext::from_parts(&headers, "https://example.com/a.git");
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
            RequestContext::from_parts(&headers, "https://example.com/a.git").is_err(),
            "whitespace is not an identity"
        );
    }

    #[test]
    fn empty_repo_parameter_is_rejected() {
        let outcome = RequestContext::from_parts(&full_headers(), "  ");
        match outcome {
            Err(BadRequest::MissingParam(name)) => assert_eq!(name, "repo"),
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("must reject an empty repo"),
        }
    }

    #[test]
    fn staleness_header_parses_and_validates() {
        let mut headers = full_headers();
        headers.insert(STALENESS_HEADER, HeaderValue::from_static("120"));
        let Ok(context) = RequestContext::from_parts(&headers, "url") else {
            panic!("numeric staleness must parse")
        };
        assert_eq!(context.max_staleness, Some(Duration::from_mins(2)));

        headers.insert(STALENESS_HEADER, HeaderValue::from_static("soon"));
        match RequestContext::from_parts(&headers, "url") {
            Err(BadRequest::NotANumber(name)) => assert_eq!(name, STALENESS_HEADER),
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("non-numeric staleness must be rejected"),
        }
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
            generation: 4,
            committed_date: "2026-08-01T00:00:00Z".to_owned(),
            sha: "aaa".to_owned(),
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
