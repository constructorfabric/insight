//! Experiment domain values: the validated name and image tag, TTL, the
//! record served by the API, and the status derivation. Pure — no I/O.

use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use regex_lite::Regex;

/// Resource-name prefix; `preview-` is 8 chars, so a 55-char name never
/// trunc-collides at the 63-char Kubernetes name limit.
pub const RESOURCE_PREFIX: &str = "preview-";
const MAX_NAME_LEN: usize = 55;
const MAX_TAG_LEN: usize = 128;

/// A validated experiment slug: a DNS-1123 label of at most 55 characters. It
/// forms the URL segment (`/exp/<name>`) and the resource name
/// (`preview-<name>`) — the same rule and failure text as the manual chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentName(String);

impl ExperimentName {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if !is_dns1123_label(raw) {
            return Err(format!(
                "experiment {raw:?} must be a DNS-1123 label: lowercase alphanumerics and '-', \
                 starting/ending alphanumeric"
            ));
        }
        if raw.len() > MAX_NAME_LEN {
            return Err(format!(
                "experiment {raw:?} is too long: max {MAX_NAME_LEN} characters (the resource \
                 name preview-<experiment> must fit the 63-char limit)"
            ));
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name shared by all three Kubernetes objects of this experiment.
    #[must_use]
    pub fn resource_name(&self) -> String {
        format!("{RESOURCE_PREFIX}{}", self.0)
    }
}

fn is_dns1123_label(s: &str) -> bool {
    let bytes = s.as_bytes();
    let edge_ok = |b: &u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    bytes.first().is_some_and(edge_ok)
        && bytes.last().is_some_and(edge_ok)
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// A validated FE image tag. The repository is hardcoded
/// ([`super::objects::IMAGE_REPOSITORY`]); the API accepts a tag only, and only
/// in the two shapes CI publishes — a `preview-…` tag or a
/// `YYYY.MM.DD.HH.MM-<sha7>[.branch]` build tag — so there is no arbitrary
/// image reference surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageTag(String);

impl ImageTag {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw.len() > MAX_TAG_LEN {
            return Err(format!("tag is too long: max {MAX_TAG_LEN} characters"));
        }
        if !(is_preview_tag(raw) || is_build_tag(raw)) {
            return Err(format!(
                "tag {raw:?} must be a `preview-…` tag or a CI build tag \
                 (YYYY.MM.DD.HH.MM-<sha7> with an optional branch suffix)"
            ));
        }
        Ok(Self(raw.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

static PREVIEW_TAG: LazyLock<Regex> = LazyLock::new(|| tag_pattern(r"^preview-[A-Za-z0-9._-]+$"));

/// `YYYY.MM.DD.HH.MM-<sha7>` optionally followed by `.<sanitized-branch>`.
static BUILD_TAG: LazyLock<Regex> = LazyLock::new(|| {
    tag_pattern(r"^\d{4}\.\d{2}\.\d{2}\.\d{2}\.\d{2}-[0-9a-fA-F]{7}(\.[A-Za-z0-9._-]+)?$")
});

fn tag_pattern(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|e| panic!("static tag pattern {pattern:?}: {e}"))
}

fn is_preview_tag(s: &str) -> bool {
    PREVIEW_TAG.is_match(s)
}

fn is_build_tag(s: &str) -> bool {
    BUILD_TAG.is_match(s)
}

/// The registry tags the create form may offer: only `preview-…` tags a
/// create would accept, deduped and sorted.
#[must_use]
pub fn preview_tags(tags: Vec<String>) -> Vec<String> {
    let mut offered: Vec<String> = tags
        .into_iter()
        .filter(|tag| tag.len() <= MAX_TAG_LEN && is_preview_tag(tag))
        .collect();
    offered.sort();
    offered.dedup();
    offered
}

/// A validated time-to-live in whole days, clamped nowhere: a request outside
/// `1..=max` is refused, not adjusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtlDays(u32);

impl TtlDays {
    pub fn parse(requested: Option<u32>, default: u32, max: u32) -> Result<Self, String> {
        let days = requested.unwrap_or(default);
        if days == 0 || days > max {
            return Err(format!("ttlDays must be between 1 and {max}, got {days}"));
        }
        Ok(Self(days))
    }

    #[must_use]
    pub fn expires_at(self, created_at: DateTime<Utc>) -> DateTime<Utc> {
        created_at + chrono::Duration::days(i64::from(self.0))
    }
}

/// One live experiment as the API serves it, read back from the annotations
/// and status of its Deployment.
#[derive(Debug, Clone)]
pub struct Experiment {
    pub name: String,
    pub tag: String,
    pub creator: String,
    pub created_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: ExperimentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    /// The FE pod is Ready and the route serves.
    Ready,
    /// Created but the FE pod is not Ready yet (or not anymore).
    Pending,
    /// Past `expiresAt`; the TTL sweep removes it on its next pass.
    Expired,
}

/// Derive the served status: expiry wins over readiness, and a missing
/// expiry annotation (an object not created by this service) never expires.
#[must_use]
pub fn status_of(
    expires_at: Option<DateTime<Utc>>,
    ready: bool,
    now: DateTime<Utc>,
) -> ExperimentStatus {
    match expires_at {
        Some(expiry) if expiry <= now => ExperimentStatus::Expired,
        _ if ready => ExperimentStatus::Ready,
        _ => ExperimentStatus::Pending,
    }
}

/// The URL the experiment serves under: `https://<host><base>/<name>/`, or a
/// host-relative path when no host is configured.
#[must_use]
pub fn experiment_url(host: &str, base_path: &str, name: &str) -> String {
    let base = base_path.trim_end_matches('/');
    if host.is_empty() {
        format!("{base}/{name}/")
    } else {
        format!("https://{host}{base}/{name}/")
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn a_valid_slug_forms_the_resource_name() -> Result<(), String> {
        let name = ExperimentName::parse("widget-alpha")?;

        assert_eq!(name.as_str(), "widget-alpha");
        assert_eq!(name.resource_name(), "preview-widget-alpha");
        Ok(())
    }

    #[test]
    fn slugs_follow_the_chart_rule() {
        // Same accept/reject table as deploy/preview/tests/test_render.py.
        for good in ["a", "a1", "widget-alpha", "0-a-0", &"a".repeat(55)] {
            assert!(
                ExperimentName::parse(good).is_ok(),
                "should accept: {good:?}"
            );
        }
        for bad in [
            "",
            "Widget_Bad",
            "UPPER",
            "-lead",
            "trail-",
            "a/b",
            "a b",
            "a.b",
        ] {
            let err = ExperimentName::parse(bad).map(|n| n.as_str().to_owned());
            assert!(err.is_err(), "should reject: {bad:?}");
            assert!(
                err.is_err_and(|e| e.contains("DNS-1123 label")),
                "failure text must name the rule for: {bad:?}"
            );
        }
    }

    #[test]
    fn a_slug_longer_than_55_chars_is_too_long() {
        let overlong = "a".repeat(56);

        let err = ExperimentName::parse(&overlong).map(|n| n.as_str().to_owned());

        assert!(
            err.as_ref().is_err_and(|e| e.contains("too long")),
            "{err:?}"
        );
    }

    #[test]
    fn tags_accept_only_the_two_published_shapes() {
        for good in [
            "preview-my-widget",
            "preview-2026.08.06",
            "2026.08.06.14.05-abc1234",
            "2026.08.06.14.05-abc1234.my-branch",
            "2026.08.06.14.05-0f0f0f0.feat-2372-previews-gear",
            "2026.08.06.14.05-ABC1234",
        ] {
            assert!(ImageTag::parse(good).is_ok(), "should accept: {good:?}");
        }
        for bad in [
            "",
            "latest",
            "preview-",
            "preview-a b",
            "preview-a:b",
            "preview-a/b",
            "2026.8.6.14.05-abc1234",
            "2026.08.06.14.05-abc123",
            "2026.08.06.14.05-abc12345",
            "2026.08.06.14.05-zzzzzzz",
            "2026.08.06.14.05-abc1234.",
            "2026.08.06.14.05abc1234",
            "main",
        ] {
            assert!(ImageTag::parse(bad).is_err(), "should reject: {bad:?}");
        }
    }

    #[test]
    fn the_form_is_offered_only_creatable_preview_tags_deduped_and_sorted() {
        let listed = vec![
            "preview-zeta".to_owned(),
            "latest".to_owned(),
            "2026.08.06.14.05-abc1234".to_owned(),
            "preview-alpha".to_owned(),
            "preview-alpha".to_owned(),
            "preview-a b".to_owned(),
            format!("preview-{}", "a".repeat(128)),
        ];

        assert_eq!(
            preview_tags(listed),
            vec!["preview-alpha".to_owned(), "preview-zeta".to_owned()]
        );
    }

    #[test]
    fn an_overlong_tag_is_rejected() {
        let overlong = format!("preview-{}", "a".repeat(128));

        assert!(ImageTag::parse(&overlong).is_err());
    }

    #[test]
    fn ttl_defaults_and_refuses_out_of_range_values() {
        for (requested, expected) in [(None, 7), (Some(1), 1), (Some(30), 30)] {
            let parsed = TtlDays::parse(requested, 7, 30).map(|t| t.0);
            assert_eq!(parsed, Ok(expected), "for: {requested:?}");
        }
        for out_of_range in [Some(0), Some(31), Some(u32::MAX)] {
            assert!(
                TtlDays::parse(out_of_range, 7, 30).is_err(),
                "should reject: {out_of_range:?}"
            );
        }
    }

    #[test]
    fn the_expiry_is_the_creation_plus_the_ttl() -> Result<(), String> {
        let created = Utc
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .ok_or("timestamp")?;

        let ttl = TtlDays::parse(Some(3), 7, 30)?;

        assert_eq!(
            ttl.expires_at(created),
            Utc.with_ymd_and_hms(2026, 1, 4, 0, 0, 0)
                .single()
                .ok_or("timestamp")?
        );
        Ok(())
    }

    #[test]
    fn expiry_wins_over_readiness() {
        let now = Utc
            .with_ymd_and_hms(2026, 1, 10, 0, 0, 0)
            .single()
            .unwrap_or_default();
        let past = now - chrono::Duration::seconds(1);
        let future = now + chrono::Duration::days(1);

        for (expires_at, ready, expected) in [
            (Some(past), true, ExperimentStatus::Expired),
            (Some(past), false, ExperimentStatus::Expired),
            (Some(now), true, ExperimentStatus::Expired),
            (Some(future), true, ExperimentStatus::Ready),
            (Some(future), false, ExperimentStatus::Pending),
            (None, true, ExperimentStatus::Ready),
            (None, false, ExperimentStatus::Pending),
        ] {
            assert_eq!(
                status_of(expires_at, ready, now),
                expected,
                "for: expires_at={expires_at:?} ready={ready}"
            );
        }
    }

    #[test]
    fn the_url_composes_host_base_and_slug() {
        for (host, base, expected) in [
            (
                "preview.example.com",
                "/exp",
                "https://preview.example.com/exp/widget/",
            ),
            (
                "preview.example.com",
                "/exp/",
                "https://preview.example.com/exp/widget/",
            ),
            ("", "/exp", "/exp/widget/"),
        ] {
            assert_eq!(
                experiment_url(host, base, "widget"),
                expected,
                "for: {host:?} {base:?}"
            );
        }
    }
}
