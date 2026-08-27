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

impl ConnectorName {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || raw.len() > MAX_LEN {
            return None;
        }
        if raw.starts_with('-') || raw.ends_with('-') {
            return None;
        }
        let allowed = raw
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        allowed.then(|| Self(raw.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}
