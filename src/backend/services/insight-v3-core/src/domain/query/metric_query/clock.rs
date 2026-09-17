//! The timestamp a metric windows and buckets by.

use serde::Deserialize;

use super::MetricQueryError;
use super::field::{FieldType, Source, is_identifier};
use super::filter::FilterBind;
use crate::domain::query::time_window::{Bounds, Grain, Window};

#[derive(Debug, Deserialize)]
pub(super) struct TimeField {
    /// The declared field of the dataset the window selects by.
    #[serde(default, rename = "field")]
    pub(super) declared: Option<String>,
    #[serde(default)]
    pub(super) json: Option<String>,
    #[serde(default)]
    pub(super) column: Option<String>,
    #[serde(default = "datetime_type")]
    pub(super) r#type: String,
}

fn datetime_type() -> String {
    "datetime".to_owned()
}

impl TimeField {
    /// The declared field this windows by, when it names one.
    pub(super) fn reads(&self) -> Option<&str> {
        self.declared.as_deref()
    }

    pub(super) fn source(&self) -> Result<Source<'_>, MetricQueryError> {
        if self.r#type != "datetime" {
            return Err(MetricQueryError::ClockType(self.r#type.clone()));
        }
        let source = Source::resolve(self.json.as_deref(), self.column.as_deref())
            .ok_or(MetricQueryError::ClockSource)?;
        if !is_identifier(source.name()) {
            return Err(MetricQueryError::Identifier(source.name().to_owned()));
        }

        Ok(source)
    }
}

/// How a clock is read out of a row. A payload key that is absent or
/// unparseable reads as no clock, not as a failed query.
pub(super) fn clock_expression(
    source: Source<'_>,
    qualifier: Option<&str>,
) -> Result<String, MetricQueryError> {
    let read = source.sql(FieldType::String, qualifier)?;

    Ok(match source {
        Source::Json { .. } => format!("parseDateTimeBestEffortOrNull({read})"),
        Source::Column(_) => read,
    })
}

pub(super) fn bucket_expression(grain: Grain, clock: &str) -> String {
    match grain {
        Grain::Hour => format!("toStartOfHour({clock}, 'UTC')"),
        Grain::Day => format!("toStartOfDay({clock}, 'UTC')"),
        Grain::Week => format!("toStartOfWeek({clock}, 1, 'UTC')"),
        Grain::Month => format!("toStartOfMonth({clock}, 'UTC')"),
    }
}

pub(super) fn add_window_predicates(
    window: &Window,
    time_expression: Option<&str>,
    where_parts: &mut Vec<String>,
    binds: &mut Vec<FilterBind>,
) -> Result<(), MetricQueryError> {
    let Window::Requested { bounds, .. } = window else {
        return Ok(());
    };
    let Some(clock) = time_expression else {
        return Err(MetricQueryError::ClocklessWindow);
    };

    match *bounds {
        Bounds::Finite { from, to } => {
            where_parts.push(format!("{clock} >= fromUnixTimestamp64Milli(?, 'UTC')"));
            where_parts.push(format!("{clock} < fromUnixTimestamp64Milli(?, 'UTC')"));
            binds.push(FilterBind::Int(from.timestamp_millis()));
            binds.push(FilterBind::Int(to.timestamp_millis()));
        }
        Bounds::Unbounded => where_parts.push(format!("{clock} IS NOT NULL")),
    }

    Ok(())
}
