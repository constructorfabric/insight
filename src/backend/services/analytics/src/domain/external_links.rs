use std::collections::HashMap;

use url::Url;

use crate::config::{ExternalSourceConfig, ExternalSourceProvider};

#[derive(Debug, Clone, Default)]
pub struct ExternalSourceRegistry {
    sources: HashMap<(ExternalSourceProvider, String), Url>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExternalSourceRegistryError {
    #[error("external source id must be non-empty and trimmed")]
    InvalidId,
    #[error("external source provider and id must be unique")]
    DuplicateProviderAndId,
    #[error("external source web_base_url is not a valid URL")]
    InvalidWebBaseUrl(#[from] url::ParseError),
    #[error("external source web_base_url must be an absolute HTTP(S) URL")]
    InvalidWebBaseUrlSchemeOrHost,
    #[error("external source web_base_url must not include userinfo")]
    WebBaseUrlIncludesUserInfo,
    #[error("external source web_base_url must not include a query or fragment")]
    WebBaseUrlIncludesQueryOrFragment,
}

impl ExternalSourceRegistry {
    pub fn new(sources: &[ExternalSourceConfig]) -> Result<Self, ExternalSourceRegistryError> {
        let mut registry = HashMap::with_capacity(sources.len());
        for source in sources {
            let id = source.id.trim();
            if id.is_empty() || id != source.id {
                return Err(ExternalSourceRegistryError::InvalidId);
            }

            let url = parse_web_base_url(&source.web_base_url)?;
            let key = (source.provider, id.to_owned());
            if registry.insert(key, url).is_some() {
                return Err(ExternalSourceRegistryError::DuplicateProviderAndId);
            }
        }
        Ok(Self { sources: registry })
    }

    pub fn evidence_links(
        &self,
        provider: &str,
        source_id: &str,
        record_kind: &str,
        repository: Option<&str>,
        record_ref: Option<&str>,
    ) -> ExternalRecordLinks {
        let Some(provider) = parse_provider(provider) else {
            return ExternalRecordLinks::default();
        };
        let Some(base) = self.sources.get(&(provider, source_id.to_owned())) else {
            return ExternalRecordLinks::default();
        };

        let repository = repository.and_then(repository_segments);
        let repository_href = repository
            .as_deref()
            .and_then(|segments| repository_url(base, provider, segments));
        let record_link = record_url(
            base,
            provider,
            record_kind,
            repository.as_deref(),
            record_ref,
        );

        ExternalRecordLinks {
            repository: repository_href,
            record: record_link,
        }
    }

    pub fn repository_href(
        &self,
        provider: Option<&str>,
        source_id: Option<&str>,
        repository: Option<&str>,
    ) -> Option<String> {
        let provider = parse_provider(provider?)?;
        let source_id = source_id?.trim();
        let base = self.sources.get(&(provider, source_id.to_owned()))?;
        let repository = repository_segments(repository?)?;
        repository_url(base, provider, &repository)
    }
}

#[derive(Debug, Default)]
pub struct ExternalRecordLinks {
    pub repository: Option<String>,
    pub record: Option<String>,
}

fn parse_web_base_url(value: &str) -> Result<Url, ExternalSourceRegistryError> {
    let mut url = Url::parse(value.trim())?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return Err(ExternalSourceRegistryError::InvalidWebBaseUrlSchemeOrHost);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ExternalSourceRegistryError::WebBaseUrlIncludesUserInfo);
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(ExternalSourceRegistryError::WebBaseUrlIncludesQueryOrFragment);
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}

fn parse_provider(value: &str) -> Option<ExternalSourceProvider> {
    match value.trim() {
        "github" => Some(ExternalSourceProvider::Github),
        "gitlab" => Some(ExternalSourceProvider::Gitlab),
        "bitbucket_cloud" => Some(ExternalSourceProvider::BitbucketCloud),
        "jira" => Some(ExternalSourceProvider::Jira),
        "youtrack" => Some(ExternalSourceProvider::Youtrack),
        _ => None,
    }
}

fn repository_url(
    base: &Url,
    provider: ExternalSourceProvider,
    repository: &[&str],
) -> Option<String> {
    match provider {
        ExternalSourceProvider::Github
        | ExternalSourceProvider::Gitlab
        | ExternalSourceProvider::BitbucketCloud => {
            append_segments(base, repository.iter().copied())
        }
        ExternalSourceProvider::Jira | ExternalSourceProvider::Youtrack => None,
    }
}

fn record_url(
    base: &Url,
    provider: ExternalSourceProvider,
    record_kind: &str,
    repository: Option<&[&str]>,
    record_ref: Option<&str>,
) -> Option<String> {
    let record_ref = record_ref?.trim();
    match provider {
        ExternalSourceProvider::Github => match record_kind {
            "commit" => append_segments(
                base,
                repository?.iter().copied().chain(["commit", record_ref]),
            ),
            "pull_request" => append_segments(
                base,
                repository?.iter().copied().chain(["pull", record_ref]),
            ),
            "issue" => issue_url(base, &["issues"], record_ref),
            _ => None,
        },
        ExternalSourceProvider::Gitlab => match record_kind {
            "commit" => append_segments(
                base,
                repository?
                    .iter()
                    .copied()
                    .chain(["-", "commit", record_ref]),
            ),
            "pull_request" => append_segments(
                base,
                repository?
                    .iter()
                    .copied()
                    .chain(["-", "merge_requests", record_ref]),
            ),
            "issue" => issue_url(base, &["-", "issues"], record_ref),
            _ => None,
        },
        ExternalSourceProvider::BitbucketCloud => match record_kind {
            "commit" => append_segments(
                base,
                repository?.iter().copied().chain(["commits", record_ref]),
            ),
            "pull_request" => append_segments(
                base,
                repository?
                    .iter()
                    .copied()
                    .chain(["pull-requests", record_ref]),
            ),
            "issue" => issue_url(base, &["issues"], record_ref),
            _ => None,
        },
        ExternalSourceProvider::Jira => match record_kind {
            "issue" => append_segments(base, ["browse", record_ref]),
            _ => None,
        },
        ExternalSourceProvider::Youtrack => match record_kind {
            "issue" => append_segments(base, ["issue", record_ref]),
            _ => None,
        },
    }
}

fn issue_url(base: &Url, suffix: &[&str], record_ref: &str) -> Option<String> {
    let (repository, number) = record_ref.rsplit_once('#')?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let repository = repository_segments(repository)?;
    append_segments(
        base,
        repository
            .into_iter()
            .chain(suffix.iter().copied())
            .chain([number]),
    )
}

fn repository_segments(value: &str) -> Option<Vec<&str>> {
    let value = value.trim();
    let segments = value.split('/').collect::<Vec<_>>();
    if segments.len() < 2 || segments.iter().any(|segment| !valid_segment(segment)) {
        return None;
    }
    Some(segments)
}

fn append_segments<'a>(base: &Url, segments: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut url = base.clone();
    let mut path = url.path_segments_mut().ok()?;
    for segment in segments {
        if !valid_segment(segment) {
            return None;
        }
        path.push(segment);
    }
    drop(path);
    Some(url.into())
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('/')
        && !segment.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources() -> Vec<ExternalSourceConfig> {
        vec![
            ExternalSourceConfig {
                id: "github-source".to_owned(),
                provider: ExternalSourceProvider::Github,
                web_base_url: "https://github.example.test/".to_owned(),
            },
            ExternalSourceConfig {
                id: "jira-source".to_owned(),
                provider: ExternalSourceProvider::Jira,
                web_base_url: "https://issues.example.test/jira/".to_owned(),
            },
        ]
    }

    #[test]
    fn resolves_git_repository_and_commit_links() -> anyhow::Result<()> {
        let registry = ExternalSourceRegistry::new(&sources())?;

        let links = registry.evidence_links(
            "github",
            "github-source",
            "commit",
            Some("group/repository"),
            Some("a1b2c3"),
        );

        assert_eq!(
            links.repository.as_deref(),
            Some("https://github.example.test/group/repository")
        );
        assert_eq!(
            links.record.as_deref(),
            Some("https://github.example.test/group/repository/commit/a1b2c3")
        );
        Ok(())
    }

    #[test]
    fn resolves_jira_issue_link() -> anyhow::Result<()> {
        let registry = ExternalSourceRegistry::new(&sources())?;

        let links = registry.evidence_links("jira", "jira-source", "issue", None, Some("OPS-42"));

        assert_eq!(
            links.record.as_deref(),
            Some("https://issues.example.test/jira/browse/OPS-42")
        );
        Ok(())
    }

    #[test]
    fn resolves_provider_specific_record_routes() -> anyhow::Result<()> {
        let entries = [
            ExternalSourceConfig {
                id: "github-source".to_owned(),
                provider: ExternalSourceProvider::Github,
                web_base_url: "https://github.example.test".to_owned(),
            },
            ExternalSourceConfig {
                id: "gitlab-source".to_owned(),
                provider: ExternalSourceProvider::Gitlab,
                web_base_url: "https://gitlab.example.test".to_owned(),
            },
            ExternalSourceConfig {
                id: "bitbucket-source".to_owned(),
                provider: ExternalSourceProvider::BitbucketCloud,
                web_base_url: "https://bitbucket.example.test".to_owned(),
            },
            ExternalSourceConfig {
                id: "youtrack-source".to_owned(),
                provider: ExternalSourceProvider::Youtrack,
                web_base_url: "https://tracker.example.test/youtrack".to_owned(),
            },
        ];
        let registry = ExternalSourceRegistry::new(&entries)?;

        for (provider, source_id, kind, repository, reference, expected) in [
            (
                "github",
                "github-source",
                "issue",
                None,
                "group/repository#42",
                "https://github.example.test/group/repository/issues/42",
            ),
            (
                "gitlab",
                "gitlab-source",
                "commit",
                Some("group/subgroup/repository"),
                "abc123",
                "https://gitlab.example.test/group/subgroup/repository/-/commit/abc123",
            ),
            (
                "gitlab",
                "gitlab-source",
                "pull_request",
                Some("group/subgroup/repository"),
                "42",
                "https://gitlab.example.test/group/subgroup/repository/-/merge_requests/42",
            ),
            (
                "gitlab",
                "gitlab-source",
                "issue",
                None,
                "group/subgroup/repository#42",
                "https://gitlab.example.test/group/subgroup/repository/-/issues/42",
            ),
            (
                "bitbucket_cloud",
                "bitbucket-source",
                "commit",
                Some("workspace/repository"),
                "abc123",
                "https://bitbucket.example.test/workspace/repository/commits/abc123",
            ),
            (
                "bitbucket_cloud",
                "bitbucket-source",
                "pull_request",
                Some("workspace/repository"),
                "42",
                "https://bitbucket.example.test/workspace/repository/pull-requests/42",
            ),
            (
                "bitbucket_cloud",
                "bitbucket-source",
                "issue",
                None,
                "workspace/repository#42",
                "https://bitbucket.example.test/workspace/repository/issues/42",
            ),
            (
                "youtrack",
                "youtrack-source",
                "issue",
                None,
                "APP-42",
                "https://tracker.example.test/youtrack/issue/APP-42",
            ),
        ] {
            let links =
                registry.evidence_links(provider, source_id, kind, repository, Some(reference));

            assert_eq!(
                links.record.as_deref(),
                Some(expected),
                "should resolve: {provider}"
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_duplicate_provider_and_id() {
        let mut entries = sources();
        entries.push(ExternalSourceConfig {
            id: "github-source".to_owned(),
            provider: ExternalSourceProvider::Github,
            web_base_url: "https://alternate.example.test".to_owned(),
        });

        assert!(matches!(
            ExternalSourceRegistry::new(&entries),
            Err(ExternalSourceRegistryError::DuplicateProviderAndId)
        ));
    }

    #[test]
    fn reports_invalid_ids_and_unparseable_urls() {
        let invalid_id = ExternalSourceConfig {
            id: " github-source".to_owned(),
            provider: ExternalSourceProvider::Github,
            web_base_url: "https://github.example.test".to_owned(),
        };
        assert!(matches!(
            ExternalSourceRegistry::new(&[invalid_id]),
            Err(ExternalSourceRegistryError::InvalidId)
        ));

        let invalid_url = ExternalSourceConfig {
            id: "github-source".to_owned(),
            provider: ExternalSourceProvider::Github,
            web_base_url: "://".to_owned(),
        };
        assert!(matches!(
            ExternalSourceRegistry::new(&[invalid_url]),
            Err(ExternalSourceRegistryError::InvalidWebBaseUrl(_))
        ));
    }

    #[test]
    fn rejects_unsafe_web_base_urls() {
        for web_base_url in [
            "github.example.test",
            "ftp://github.example.test",
            "https://user@github.example.test",
            "https://github.example.test?query=yes",
            "https://github.example.test#fragment",
        ] {
            let entry = ExternalSourceConfig {
                id: "source".to_owned(),
                provider: ExternalSourceProvider::Github,
                web_base_url: web_base_url.to_owned(),
            };

            assert!(
                ExternalSourceRegistry::new(&[entry]).is_err(),
                "should reject: {web_base_url}"
            );
        }
    }

    #[test]
    fn refuses_links_without_valid_context() -> anyhow::Result<()> {
        let registry = ExternalSourceRegistry::new(&sources())?;

        for (provider, source_id, kind, repository, reference) in [
            (
                "github",
                "missing-source",
                "commit",
                Some("group/repository"),
                Some("a1b2c3"),
            ),
            (
                "github",
                "github-source",
                "commit",
                Some("source-a:group/repository"),
                Some("a1b2c3"),
            ),
            (
                "github",
                "github-source",
                "commit",
                Some("repository"),
                Some("a1b2c3"),
            ),
            (
                "github",
                "github-source",
                "commit",
                Some("group/../repository"),
                Some("a1b2c3"),
            ),
            (
                "github",
                "github-source",
                "commit",
                Some("group/./repository"),
                Some("a1b2c3"),
            ),
            (
                "github",
                "github-source",
                "commit",
                Some("group/repository"),
                Some(""),
            ),
            (
                "github",
                "github-source",
                "issue",
                None,
                Some("not-an-issue-ref"),
            ),
            (
                "github",
                "github-source",
                "unsupported",
                Some("group/repository"),
                Some("42"),
            ),
        ] {
            let links = registry.evidence_links(provider, source_id, kind, repository, reference);

            assert!(links.record.is_none(), "should reject: {kind}");
        }
        Ok(())
    }
}
