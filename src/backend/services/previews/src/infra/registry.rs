//! OCI registry read access: list the tags of the fixed FE image repository
//! (`/v2/<repo>/tags/list`, `Link`-header pagination). Read-only — nothing
//! here can push or delete.

use std::time::Duration;

use anyhow::{Context, anyhow};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, LINK};

use crate::domain::objects::IMAGE_REPOSITORY;

const REGISTRY_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_SIZE: usize = 1000;
const MAX_PAGES: usize = 20;

/// A client for the one registry the FE image repository lives in, carrying
/// the read credential on every request.
pub struct Registry {
    http: reqwest::Client,
    base_url: String,
}

impl Registry {
    /// Build the client. Fails when the token cannot form a header value.
    pub fn connect(base_url: &str, token: &str) -> anyhow::Result<Self> {
        let mut bearer = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("registry token is not a valid header value")?;
        bearer.set_sensitive(true);

        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, bearer);
        let http = reqwest::Client::builder()
            .timeout(REGISTRY_TIMEOUT)
            .default_headers(headers)
            .build()
            .context("registry http client")?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    /// Every tag of the FE image repository, following pagination up to
    /// [`MAX_PAGES`]; a repository larger than that is truncated and logged.
    pub async fn list_tags(&self) -> anyhow::Result<Vec<String>> {
        let mut url = format!(
            "{}/v2/{}/tags/list?n={PAGE_SIZE}",
            self.base_url,
            repository_path()
        );

        let mut tags = Vec::new();
        for _ in 0..MAX_PAGES {
            let response = self
                .http
                .get(&url)
                .send()
                .await
                .context("registry tags request")?;
            let status = response.status();
            if !status.is_success() {
                return Err(anyhow!("registry answered {status} listing tags"));
            }

            let link = response
                .headers()
                .get(LINK)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let page: TagsPage = response.json().await.context("registry tags body")?;
            tags.extend(page.tags.unwrap_or_default());

            match next_page_url(link.as_deref(), &self.base_url) {
                Some(next) => url = next,
                None => return Ok(tags),
            }
        }

        tracing::warn!(
            pages = MAX_PAGES,
            "registry tag listing hit the page cap; the rest is dropped"
        );
        Ok(tags)
    }
}

/// The tags-list body of the OCI distribution spec; `tags` is null for an
/// empty repository on some registries.
#[derive(Debug, serde::Deserialize)]
struct TagsPage {
    tags: Option<Vec<String>>,
}

/// The repository path within the registry: [`IMAGE_REPOSITORY`] minus its
/// registry-host segment.
fn repository_path() -> &'static str {
    IMAGE_REPOSITORY
        .split_once('/')
        .map_or(IMAGE_REPOSITORY, |(_host, path)| path)
}

/// The follow-up URL out of an OCI `Link` header (`<url>; rel="next"`),
/// resolved against the registry base when the URL is host-relative.
fn next_page_url(link: Option<&str>, base_url: &str) -> Option<String> {
    let target = link?
        .split(',')
        .find(|part| part.contains("rel=\"next\""))?
        .trim();
    let url = target.strip_prefix('<')?.split_once('>')?.0;
    if url.starts_with('/') {
        Some(format!("{base_url}{url}"))
    } else {
        Some(url.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_path_drops_the_registry_host() {
        assert_eq!(repository_path(), "constructorfabric/insight-frontend");
    }

    #[test]
    fn pagination_follows_only_a_rel_next_link() {
        let base = "https://registry.example.com";
        for (case, link, expected) in [
            ("no header", None, None),
            ("no next rel", Some("</v2/x>; rel=\"prev\""), None),
            (
                "relative next",
                Some("</v2/app/tags/list?last=preview-a&n=1000>; rel=\"next\""),
                Some(
                    "https://registry.example.com/v2/app/tags/list?last=preview-a&n=1000"
                        .to_owned(),
                ),
            ),
            (
                "absolute next",
                Some("<https://other.example.com/v2/app/tags/list?last=b>; rel=\"next\""),
                Some("https://other.example.com/v2/app/tags/list?last=b".to_owned()),
            ),
            (
                "next among several",
                Some("</v2/x>; rel=\"prev\", </v2/y>; rel=\"next\""),
                Some("https://registry.example.com/v2/y".to_owned()),
            ),
            ("malformed target", Some("v2/y; rel=\"next\""), None),
        ] {
            assert_eq!(next_page_url(link, base), expected, "for: {case}");
        }
    }
}
