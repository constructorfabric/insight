//! The dataset a metric is compiled over.
//!
//! A metric reading a dataset names its fields; where each value sits in a
//! record, and which records count as one, are the declaration's to say. A
//! metric with none of this behind it addresses a warehouse relation itself,
//! which is how metrics read data before datasets.

use std::fmt::Write as _;

use super::MetricQueryError;
use super::people::PersonHandle;
use crate::domain::kinds::dataset::declaration::{Declaration, Field, FieldType};
use crate::domain::kinds::dataset::read::{Form, PAYLOAD_COLUMN, collapsed, read};

/// What a metric over a dataset is compiled against.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Over<'a> {
    pub(crate) declaration: &'a Declaration,
    /// The database this service keeps every dataset's records in.
    pub(crate) database: &'a str,
    /// The table holding them, as the dataset's row names it.
    pub(crate) table: &'a str,
}

impl<'a> Over<'a> {
    /// The relation a run reads: one record per identity.
    pub(crate) fn relation(self, qualifier: Option<&str>) -> String {
        let mut from = collapsed(self.declaration, self.database, self.table);

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
        self.read_as(named, at, qualifier, Form::Presented)
    }

    /// The same, in the form the caller needs: a join key reads the record's
    /// own value, so a substitute never stands in for a person nobody has.
    pub(crate) fn read_as(
        self,
        named: Option<&str>,
        at: &str,
        qualifier: Option<&str>,
        form: Form,
    ) -> Result<String, MetricQueryError> {
        let field = self.declared(named, at)?;

        Ok(read(field, form, &Self::payload(qualifier)))
    }

    /// What the declaration says this field holds, for a caller that has to
    /// know the type before it can bind a value against it.
    pub(crate) fn type_of(
        self,
        named: Option<&str>,
        at: &str,
    ) -> Result<FieldType, MetricQueryError> {
        Ok(self.declared(named, at)?.r#type)
    }

    /// Which handle this field carries a person under, when it carries one.
    ///
    /// The declaration answers it, not the metric: where a person sits in a
    /// record is what the dataset knows about its own records.
    pub(super) fn person_of(self, named: Option<&str>) -> Option<PersonHandle> {
        self.declaration
            .field(named?)?
            .person
            .map(PersonHandle::from)
    }

    fn declared(self, named: Option<&str>, at: &str) -> Result<&'a Field, MetricQueryError> {
        let named = named.ok_or_else(|| MetricQueryError::FieldSource(at.to_owned()))?;

        self.declaration
            .field(named)
            .ok_or_else(|| MetricQueryError::UnknownField(named.to_owned()))
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
