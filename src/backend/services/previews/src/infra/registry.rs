//! OCI registry read access: list the tags of the FE image repository.

use std::time::Duration;

use anyhow::{Context, anyhow};
use reqwest::StatusCode;
use reqwest::header::{AUTHORIZATION, HeaderValue, LINK, WWW_AUTHENTICATE};

use crate::domain::objects::IMAGE_REPOSITORY;

const REGISTRY_TIMEOUT: Duration = Duration::from_secs(10);
const PAGE_SIZE: usize = 1000;
const MAX_PAGES: usize = 20;

pub struct Registry {
    http: reqwest::Client,
    base_url: String,
    static_bearer: Option<HeaderValue>,
}

impl Registry {
    pub fn connect(base_url: &str, token: &str) -> anyhow::Result<Self> {
        let static_bearer = if token.is_empty() {
            None
        } else {
            Some(bearer(token)?)
        };
        let http = reqwest::Client::builder()
            .timeout(REGISTRY_TIMEOUT)
            .build()
            .context("registry http client")?;

        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            static_bearer,
        })
    }

    /// All tags, following `Link` pagination; past [`MAX_PAGES`] the rest is
    /// dropped and logged.
    pub async fn list_tags(&self) -> anyhow::Result<Vec<String>> {
        let mut url = format!(
            "{}/v2/{}/tags/list?n={PAGE_SIZE}",
            self.base_url,
            repository_path()
        );
        let mut auth = self.static_bearer.clone();

        let mut tags = Vec::new();
        for _ in 0..MAX_PAGES {
            let mut response = self.get(&url, auth.as_ref()).await?;
            if response.status() == StatusCode::UNAUTHORIZED && auth.is_none() {
                auth = Some(self.anonymous_bearer(&response).await?);
                response = self.get(&url, auth.as_ref()).await?;
            }
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

    async fn get(
        &self,
        url: &str,
        auth: Option<&HeaderValue>,
    ) -> anyhow::Result<reqwest::Response> {
        let mut request = self.http.get(url);
        if let Some(auth) = auth {
            request = request.header(AUTHORIZATION, auth);
        }
        request.send().await.context("registry request")
    }

    /// Even public repositories answer 401 anonymously; the challenge names
    /// the endpoint that mints a pull token with no credential.
    async fn anonymous_bearer(&self, refused: &reqwest::Response) -> anyhow::Result<HeaderValue> {
        let challenge = refused
            .headers()
            .get(WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| anyhow!("registry 401 without a bearer challenge"))?;
        let token_url = token_url(challenge, repository_path())
            .ok_or_else(|| anyhow!("registry challenge names no token endpoint"))?;

        let minted: MintedToken = self
            .http
            .get(&token_url)
            .send()
            .await
            .context("registry token request")?
            .error_for_status()
            .context("registry token endpoint")?
            .json()
            .await
            .context("registry token body")?;
        bearer(&minted.token)
    }
}

fn bearer(token: &str) -> anyhow::Result<HeaderValue> {
    let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
        .context("registry token is not a valid header value")?;
    value.set_sensitive(true);
    Ok(value)
}

#[derive(Debug, serde::Deserialize)]
struct MintedToken {
    token: String,
}

/// `Bearer realm="…",service="…"` →
/// `{realm}?service={service}&scope=repository:{repo}:pull`.
fn token_url(challenge: &str, repo: &str) -> Option<String> {
    let params = challenge.strip_prefix("Bearer ")?;
    let realm = challenge_field(params, "realm")?;
    let service = challenge_field(params, "service").unwrap_or_default();
    Some(format!(
        "{realm}?service={service}&scope=repository:{repo}:pull"
    ))
}

fn challenge_field(params: &str, name: &str) -> Option<String> {
    params.split(',').find_map(|part| {
        part.trim()
            .strip_prefix(name)?
            .strip_prefix("=\"")?
            .strip_suffix('"')
            .map(str::to_owned)
    })
}

/// WORKAROUND: some registries serve `tags: null` for an empty repository.
#[derive(Debug, serde::Deserialize)]
struct TagsPage {
    tags: Option<Vec<String>>,
}

fn repository_path() -> &'static str {
    IMAGE_REPOSITORY
        .split_once('/')
        .map_or(IMAGE_REPOSITORY, |(_host, path)| path)
}

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
    fn the_token_endpoint_comes_from_the_bearer_challenge() {
        for (case, challenge, expected) in [
            (
                "ghcr-shaped challenge",
                "Bearer realm=\"https://registry.example.com/token\",service=\"registry.example.com\",scope=\"repository:example/app:pull\"",
                Some(
                    "https://registry.example.com/token?service=registry.example.com&scope=repository:example/app:pull"
                        .to_owned(),
                ),
            ),
            (
                "no service",
                "Bearer realm=\"https://registry.example.com/token\"",
                Some(
                    "https://registry.example.com/token?service=&scope=repository:example/app:pull"
                        .to_owned(),
                ),
            ),
            ("basic challenge", "Basic realm=\"registry\"", None),
            ("no realm", "Bearer service=\"registry.example.com\"", None),
        ] {
            assert_eq!(
                token_url(challenge, "example/app"),
                expected,
                "for: {case}"
            );
        }
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
