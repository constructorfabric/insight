//! The connector name as it crosses the request boundary.

/// A connector name, parsed once at the edge so no raw path segment reaches a
/// query.
///
/// The vocabulary is what the descriptors use: lowercase letters, digits and
/// hyphens. Underscores are excluded deliberately — a connector name maps onto
/// a bronze schema name by replacing hyphens with underscores, so a name
/// carrying one already makes that mapping ambiguous.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectorName(String);

const MAX_LEN: usize = 64;

/// Lowercase letters, digits and hyphens; neither end a hyphen; bounded.
fn is_slug(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > MAX_LEN {
        return false;
    }
    if raw.starts_with('-') || raw.ends_with('-') {
        return false;
    }
    raw.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

impl ConnectorName {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        is_slug(raw).then(|| Self(raw.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// The tenant half of an instance identity, parsed at the edge like the name.
///
/// Its own type rather than a shared one: the two halves carry the same
/// vocabulary and mean different things, and a single type for both makes a
/// window narrowed by a source id passed as a tenant a call that compiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TenantId(String);

/// The source half — which installation of the connector, within that tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceId(String);

impl TenantId {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        is_slug(raw).then(|| Self(raw.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl SourceId {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        is_slug(raw).then(|| Self(raw.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}
