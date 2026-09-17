//! The dataset a metric is compiled over.
//!
//! A metric reading a dataset names its fields; where each value sits in a
//! record, and which records count as one, are the declaration's to say. A
//! metric with none of this behind it addresses a warehouse relation itself,
//! which is how metrics read data before datasets.

use std::fmt::Write as _;

use super::MetricQueryError;
use crate::domain::kinds::dataset::declaration::Declaration;
use crate::domain::kinds::dataset::read::{Form, PAYLOAD_COLUMN, collapsed, read};

/// What a metric over a dataset is compiled against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Over<'a> {
    pub(crate) declaration: &'a Declaration,
    /// The table holding the records, as the dataset's row names it.
    pub(crate) table: &'a str,
}

impl Over<'_> {
    /// The relation a run reads: one record per identity.
    pub(crate) fn relation(self, qualifier: Option<&str>) -> String {
        let mut from = collapsed(self.declaration, self.table);

        if let Some(alias) = qualifier {
            let _ = write!(from, " AS `{alias}`");
        }

        from
    }

    /// The expression for one declared field, as a reader sees it.
    ///
    /// `at` says which part of the metric named it, so a body naming nothing
    /// is refused against the place that left it out.
    pub(crate) fn read(
        self,
        named: Option<&str>,
        at: &str,
        qualifier: Option<&str>,
    ) -> Result<String, MetricQueryError> {
        let named = named.ok_or_else(|| MetricQueryError::FieldSource(at.to_owned()))?;
        let field = self
            .declaration
            .field(named)
            .ok_or_else(|| MetricQueryError::UnknownField(named.to_owned()))?;

        Ok(read(field, Form::Presented, &Self::payload(qualifier)))
    }

    /// The column every record is stored in, qualified when a join makes a
    /// bare name ambiguous.
    fn payload(qualifier: Option<&str>) -> String {
        match qualifier {
            Some(alias) => format!("`{alias}`.{PAYLOAD_COLUMN}"),
            None => PAYLOAD_COLUMN.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests;
