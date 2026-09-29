use toolkit_canonical_errors::CanonicalError;

use super::super::error::UsageError;

const MAX_SEARCH_BYTES: usize = 200;

#[derive(Debug)]
pub(super) struct PersonSearch(String);

impl PersonSearch {
    pub(super) fn parse(raw: Option<&str>) -> Result<Option<Self>, CanonicalError> {
        let Some(needle) = raw.map(str::trim).filter(|needle| !needle.is_empty()) else {
            return Ok(None);
        };

        if needle.len() > MAX_SEARCH_BYTES {
            return Err(UsageError::invalid_argument()
                .with_field_violation(
                    "search",
                    format!("search must be at most {MAX_SEARCH_BYTES} bytes"),
                    "INVALID",
                )
                .create());
        }

        Ok(Some(Self(needle.to_owned())))
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests;
