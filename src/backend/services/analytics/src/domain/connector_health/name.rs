#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectorName(String);

const MAX_LEN: usize = 64;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TenantId(String);

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
