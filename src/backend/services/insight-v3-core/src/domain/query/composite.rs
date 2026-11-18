//! A metric that combines the numbers of two to four other metrics, each over
//! its own dataset or table, on the keys they all group by.
//!
//! INVARIANT: every metric a composite combines answers exactly one row per
//! key, so stacking their rows and folding them by key never counts a fact
//! twice. [`Composite::admit`] is what holds each one to that.

mod admission;
mod compiler;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::Deserialize;
use thiserror::Error;

use crate::domain::definition::DefinitionId;
use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::query::metric_query::{MetricQueryError, OrderBy, imposed_limit, is_identifier};

pub(crate) use admission::{Admitted, Candidate};
pub(crate) use compiler::Stacked;

const FEWEST_COMBINED: usize = 2;
const MOST_COMBINED: usize = 4;

/// The start of the names a composite stacks its children's values under,
/// which none of its own fields may take.
const RESERVED_PREFIX: &str = "__";

/// The name a composite gives one of the metrics it combines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Alias(String);

impl Alias {
    fn parse(value: String) -> Result<Self, CompositeError> {
        if is_identifier(&value) && value != BUCKET_COLUMN {
            return Ok(Self(value));
        }

        Err(CompositeError::Alias(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

/// One metric a composite combines, under the name the composite gives it.
#[derive(Debug)]
pub(crate) struct Combined {
    pub(crate) alias: Alias,
    pub(crate) metric: DefinitionId,
}

/// A stored composite, read whole: every name in it resolves to a metric it
/// combines or to a column it produces.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawComposite")]
pub(crate) struct Composite {
    /// In the order the body's map holds them, which decides the order the
    /// children are stacked in and which one's clock a run reports.
    combine: Vec<Combined>,
    group_by: Vec<String>,
    fields: Vec<Field>,
    order_by: Option<OrderBy>,
    limit: Option<u32>,
}

/// One value column of a composite.
#[derive(Debug, Deserialize)]
#[serde(try_from = "RawField")]
enum Field {
    /// A value column of one combined metric, as it reads there.
    Pick {
        as_name: String,
        alias: String,
        column: String,
    },
    /// Two earlier fields of this composite, divided.
    Ratio {
        as_name: String,
        numerator: String,
        denominator: String,
        percent: bool,
    },
}

impl Field {
    fn as_name(&self) -> &str {
        match self {
            Self::Pick { as_name, .. } | Self::Ratio { as_name, .. } => as_name,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawComposite {
    combine: BTreeMap<String, String>,
    #[serde(default)]
    group_by: Vec<String>,
    fields: Vec<Field>,
    #[serde(default)]
    order_by: Option<OrderBy>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawField {
    as_name: String,
    #[serde(default)]
    field: Option<String>,
    #[serde(default)]
    divide: Option<[String; 2]>,
    #[serde(default)]
    percent: bool,
}

impl TryFrom<RawField> for Field {
    type Error = CompositeError;

    fn try_from(raw: RawField) -> Result<Self, Self::Error> {
        match (raw.field, raw.divide) {
            (Some(read), None) if !raw.percent => {
                let Some((alias, column)) = read.split_once('.') else {
                    return Err(CompositeError::Pick(read));
                };
                if !is_identifier(column) {
                    return Err(CompositeError::Pick(read));
                }

                Ok(Self::Pick {
                    as_name: raw.as_name,
                    alias: alias.to_owned(),
                    column: column.to_owned(),
                })
            }
            (None, Some([numerator, denominator])) => Ok(Self::Ratio {
                as_name: raw.as_name,
                numerator,
                denominator,
                percent: raw.percent,
            }),
            (Some(_), None) => Err(CompositeError::Percent(raw.as_name)),
            (Some(_), Some(_)) | (None, None) => Err(CompositeError::FieldShape(raw.as_name)),
        }
    }
}

impl TryFrom<RawComposite> for Composite {
    type Error = CompositeError;

    fn try_from(raw: RawComposite) -> Result<Self, Self::Error> {
        let count = raw.combine.len();
        if !(FEWEST_COMBINED..=MOST_COMBINED).contains(&count) {
            return Err(CompositeError::Count(count));
        }

        let mut combine = Vec::with_capacity(count);
        for (alias, metric) in raw.combine {
            let Ok(id) = DefinitionId::parse(&metric) else {
                return Err(CompositeError::MetricId { alias, metric });
            };
            combine.push(Combined {
                alias: Alias::parse(alias)?,
                metric: id,
            });
        }
        check_names(&combine, &raw.group_by, &raw.fields, raw.order_by.as_ref())?;

        Ok(Self {
            combine,
            group_by: raw.group_by,
            fields: raw.fields,
            order_by: raw.order_by,
            limit: raw.limit,
        })
    }
}

/// Whether every name in the body resolves: each key and field is a column
/// of its own, each pick reads a combined metric, each ratio divides earlier
/// fields, every combined metric is read, and the order names a column.
fn check_names(
    combine: &[Combined],
    group_by: &[String],
    fields: &[Field],
    order_by: Option<&OrderBy>,
) -> Result<(), CompositeError> {
    let mut keys: Vec<&str> = Vec::with_capacity(group_by.len());
    for key in group_by {
        if !is_identifier(key) || key == BUCKET_COLUMN || keys.contains(&key.as_str()) {
            return Err(CompositeError::Key(key.clone()));
        }
        keys.push(key);
    }

    if fields.is_empty() {
        return Err(CompositeError::NoFields);
    }
    let mut values: Vec<&str> = Vec::with_capacity(fields.len());
    for field in fields {
        let as_name = field.as_name();
        let taken = keys.contains(&as_name) || values.contains(&as_name);
        if !is_identifier(as_name)
            || as_name == BUCKET_COLUMN
            || as_name.starts_with(RESERVED_PREFIX)
            || taken
        {
            return Err(CompositeError::AsName(as_name.to_owned()));
        }

        match field {
            Field::Pick { alias, .. } => {
                if !combine
                    .iter()
                    .any(|combined| combined.alias.as_str() == alias)
                {
                    return Err(CompositeError::UnknownAlias {
                        as_name: as_name.to_owned(),
                        alias: alias.clone(),
                    });
                }
            }
            Field::Ratio {
                numerator,
                denominator,
                ..
            } => {
                for name in [numerator, denominator] {
                    if !values.contains(&name.as_str()) {
                        return Err(CompositeError::Divide {
                            as_name: as_name.to_owned(),
                            name: name.clone(),
                        });
                    }
                }
            }
        }
        values.push(as_name);
    }

    for combined in combine {
        let read = fields.iter().any(
            |field| matches!(field, Field::Pick { alias, .. } if alias == combined.alias.as_str()),
        );
        if !read {
            return Err(CompositeError::Unread(combined.alias.as_str().to_owned()));
        }
    }

    if let Some(order) = order_by {
        let named = order.field.as_str();
        if named != BUCKET_COLUMN && !keys.contains(&named) && !values.contains(&named) {
            return Err(CompositeError::OrderBy(order.field.clone()));
        }
    }

    Ok(())
}

impl Composite {
    /// The metrics this composite combines.
    pub(crate) fn combine(&self) -> &[Combined] {
        &self.combine
    }

    /// The keys every child is folded on.
    pub(crate) fn groups(&self) -> &[String] {
        &self.group_by
    }

    pub(crate) fn imposed_limit(&self) -> Option<u32> {
        imposed_limit(self.limit)
    }

    /// The columns a result carries, in order: the bucket when a window has
    /// one, then the keys, then the fields.
    pub(crate) fn column_names(&self, clocked: bool) -> Vec<String> {
        let mut names = Vec::with_capacity(self.group_by.len() + self.fields.len() + 1);
        if clocked {
            names.push(BUCKET_COLUMN.to_owned());
        }
        names.extend(self.group_by.iter().cloned());
        names.extend(self.fields.iter().map(|field| field.as_name().to_owned()));

        names
    }
}

/// Why a composite cannot be stored or run.
#[derive(Debug, Error)]
pub(crate) enum CompositeError {
    #[error("a composite combines 2 to 4 metrics; this one combines {0}")]
    Count(usize),
    #[error(
        "`{0}` cannot name a combined metric: an alias is 1-128 letters, digits or \
         underscores, and not `bucket`"
    )]
    Alias(String),
    #[error("`{alias}` names `{metric}`, which is not a metric id")]
    MetricId { alias: String, metric: String },
    #[error(
        "`{0}` cannot be a key: a key is 1-128 letters, digits or underscores, named once, \
         and never `bucket`, which a windowed run adds itself"
    )]
    Key(String),
    #[error("a composite must have at least one field")]
    NoFields,
    #[error("`{0}` must name exactly one of `field` or `divide`")]
    FieldShape(String),
    #[error("`{0}` reads a combined metric's column as it is; only a `divide` is a percentage")]
    Percent(String),
    #[error("`{0}` must read a combined metric's column as `<alias>.<column>`")]
    Pick(String),
    #[error(
        "`{0}` cannot name a field: it must be 1-128 letters, digits or underscores, not begin \
         with `__`, not be `bucket`, and not repeat a key or another field"
    )]
    AsName(String),
    #[error("`{as_name}` reads `{alias}`, which this composite does not combine")]
    UnknownAlias { as_name: String, alias: String },
    #[error("`{as_name}` divides `{name}`, which no earlier field of this composite produces")]
    Divide { as_name: String, name: String },
    #[error("`{0}` is combined but no field reads it")]
    Unread(String),
    #[error("`{0}` cannot order the rows: it is not a column of this composite")]
    OrderBy(String),
    #[error("`{alias}` names metric `{metric}`, which is not there")]
    Missing { alias: String, metric: String },
    #[error("`{alias}` names a metric that cannot be read: {reason}")]
    Unreadable { alias: String, reason: String },
    #[error("`{0}` is a composite itself; a composite combines plain metrics only")]
    Nested(String),
    #[error(
        "`{0}` answers a row per record; a composite combines metrics that aggregate, one row \
         per key"
    )]
    Unaggregated(String),
    #[error(
        "`{0}` cuts its rows at a `limit` of its own, which would drop keys before they are \
         combined"
    )]
    Limited(String),
    #[error(
        "`{alias}` groups by {theirs}; every combined metric must group by exactly this \
         composite's `group_by`: {ours}"
    )]
    Keys {
        alias: String,
        theirs: String,
        ours: String,
    },
    #[error("`{key}` holds a different kind of value in `{alias}` than in `{first}`")]
    KeyKind {
        key: String,
        alias: String,
        first: String,
    },
    #[error(
        "`{dated}` has a date to window by and `{undated}` has none; a composite combines \
         metrics that are all dated or all undated"
    )]
    Clocks { dated: String, undated: String },
    #[error("`{alias}.{column}` is not a value column of `{alias}`; its values are {values}")]
    NotAValue {
        alias: String,
        column: String,
        values: String,
    },
    #[error("`{alias}`: {source}")]
    Compile {
        alias: String,
        source: MetricQueryError,
    },
    #[error(
        "a composite metric has no records of its own to show; drill into one of the metrics \
         it combines instead"
    )]
    NoRecords,
}
