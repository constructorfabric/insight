use std::fmt::Write as _;

use super::MetricQueryError;
use super::people::PersonHandle;
use crate::domain::kinds::dataset::declaration::{Declaration, Field, FieldType};
use crate::domain::kinds::dataset::read::{Form, PAYLOAD_COLUMN, collapsed, read};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Over<'a> {
    pub(crate) declaration: &'a Declaration,
    pub(crate) database: &'a str,
    pub(crate) table: &'a str,
}

impl<'a> Over<'a> {
    pub(crate) fn relation(self, qualifier: Option<&str>) -> String {
        let mut from = collapsed(self.declaration, self.database, self.table);

        if let Some(alias) = qualifier {
            let _ = write!(from, " AS `{alias}`");
        }

        from
    }

    pub(crate) fn read(
        self,
        named: Option<&str>,
        at: &str,
        qualifier: Option<&str>,
    ) -> Result<String, MetricQueryError> {
        self.read_as(named, at, qualifier, Form::Presented)
    }

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

    pub(crate) fn type_of(
        self,
        named: Option<&str>,
        at: &str,
    ) -> Result<FieldType, MetricQueryError> {
        Ok(self.declared(named, at)?.r#type)
    }

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

    fn payload(qualifier: Option<&str>) -> String {
        match qualifier {
            Some(alias) => format!("`{alias}`.{PAYLOAD_COLUMN}"),
            None => PAYLOAD_COLUMN.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests;
