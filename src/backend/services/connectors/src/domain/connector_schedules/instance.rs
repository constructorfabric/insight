use super::{Error, validation::Violation};

#[derive(Debug, Clone)]
pub(crate) struct Instance {
    pub(super) connector: Slug,
    pub(super) source_id: Slug,
}

#[derive(Debug, Clone)]
pub(super) struct Slug(String);

impl Slug {
    fn parse(raw: &str) -> Option<Self> {
        let valid = !raw.is_empty()
            && raw.len() <= 64
            && !raw.starts_with('-')
            && !raw.ends_with('-')
            && raw
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-');
        valid.then(|| Self(raw.to_owned()))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Instance {
    pub(crate) fn parse(connector: &str, source_id: &str) -> Result<Self, Error> {
        Ok(Self {
            connector: Slug::parse(connector)
                .ok_or_else(|| Violation::new("connector", "invalid connector name"))?,
            source_id: Slug::parse(source_id)
                .ok_or_else(|| Violation::new("source_id", "invalid source_id"))?,
        })
    }

    pub(crate) fn workflow_name(&self, tenant: &str) -> Result<String, Error> {
        let connector = self.connector.as_str();
        let source = self.source_id.as_str();
        let label = source
            .strip_prefix(connector)
            .and_then(|s| s.strip_prefix('-'))
            .unwrap_or(source);
        let prefix: String = tenant.chars().take(8).collect();
        let prefix = prefix.trim_end_matches(['-', '.']);
        let name = format!("{connector}-{label}-{prefix}-sync");
        if prefix.is_empty() || name.len() > 52 {
            return Err(Error::WorkflowName);
        }
        Ok(name)
    }
}
