//! Holding each metric a composite combines to one row per key.

use std::collections::BTreeSet;

use crate::domain::kinds::dataset::declaration::BUCKET_COLUMN;
use crate::domain::query::metric_query::{ColumnKind, MetricQuery};

use super::{Alias, Composite, CompositeError, Field};

/// One combined metric, as admission weighs it.
#[derive(Debug)]
pub(crate) struct Candidate<'a> {
    pub(crate) alias: &'a Alias,
    pub(crate) query: &'a MetricQuery,
    /// What each of its result columns holds.
    pub(crate) kinds: Vec<(String, ColumnKind)>,
    /// Whether a window over it has a date to select by.
    pub(crate) dated: bool,
}

/// What a composite's columns hold once its children are admitted.
#[derive(Debug)]
pub(crate) struct Admitted {
    dated: bool,
    /// The keys, then the fields, each with what it holds.
    kinds: Vec<(String, ColumnKind)>,
}

impl Admitted {
    /// Whether every child has a date to window by; admission refuses a mix.
    pub(crate) fn dated(&self) -> bool {
        self.dated
    }

    /// What each result column holds, in result order.
    pub(crate) fn column_kinds(&self, clocked: bool) -> Vec<(String, ColumnKind)> {
        let mut kinds = Vec::with_capacity(self.kinds.len() + usize::from(clocked));
        if clocked {
            kinds.push((BUCKET_COLUMN.to_owned(), ColumnKind::Date));
        }
        kinds.extend(self.kinds.iter().cloned());

        kinds
    }
}

impl Composite {
    /// Whether these children can be combined without a fact being counted
    /// twice or a key being lost, given in the order [`Composite::combine`]
    /// names them. A refusal names the child and the rule it breaks.
    pub(crate) fn admit(&self, children: &[Candidate<'_>]) -> Result<Admitted, CompositeError> {
        let keys: BTreeSet<&str> = self.group_by.iter().map(String::as_str).collect();
        for child in children {
            one_row_per_key(child, &keys)?;
        }

        let Some((first, rest)) = children.split_first() else {
            return Err(CompositeError::Count(0));
        };
        for child in rest {
            for key in &self.group_by {
                let theirs = kind_of(&child.kinds, key);
                let ours = kind_of(&first.kinds, key);
                if theirs != ours {
                    return Err(CompositeError::KeyKind {
                        key: key.clone(),
                        alias: child.alias.as_str().to_owned(),
                        first: first.alias.as_str().to_owned(),
                    });
                }
            }
            if child.dated != first.dated {
                let (dated, undated) = if child.dated {
                    (child, first)
                } else {
                    (first, child)
                };
                return Err(CompositeError::Clocks {
                    dated: dated.alias.as_str().to_owned(),
                    undated: undated.alias.as_str().to_owned(),
                });
            }
        }

        let mut kinds = Vec::with_capacity(self.group_by.len() + self.fields.len());
        for key in &self.group_by {
            let Some(kind) = kind_of(&first.kinds, key) else {
                return Err(keys_differ(first, &keys));
            };
            kinds.push((key.clone(), kind));
        }
        for field in &self.fields {
            let kind = match field {
                Field::Pick { alias, column, .. } => value_kind(children, &keys, alias, column)?,
                Field::Ratio { .. } => ColumnKind::Number,
            };
            kinds.push((field.as_name().to_owned(), kind));
        }

        Ok(Admitted {
            dated: first.dated,
            kinds,
        })
    }
}

/// Rules one to three of a child: it aggregates, it keeps every key, and it
/// groups by exactly the composite's keys.
fn one_row_per_key(child: &Candidate<'_>, keys: &BTreeSet<&str>) -> Result<(), CompositeError> {
    let alias = child.alias.as_str();
    if !child.query.groups_its_rows() {
        return Err(CompositeError::Unaggregated(alias.to_owned()));
    }
    if child.query.names_a_limit() {
        return Err(CompositeError::Limited(alias.to_owned()));
    }

    if &keys_of(child.query) != keys {
        return Err(keys_differ(child, keys));
    }

    Ok(())
}

/// The columns a child groups by, leaving out the bucket a window adds.
fn keys_of(query: &MetricQuery) -> BTreeSet<&str> {
    query
        .groups()
        .iter()
        .map(String::as_str)
        .filter(|group| *group != BUCKET_COLUMN)
        .collect()
}

fn keys_differ(child: &Candidate<'_>, keys: &BTreeSet<&str>) -> CompositeError {
    CompositeError::Keys {
        alias: child.alias.as_str().to_owned(),
        theirs: listed(&keys_of(child.query)),
        ours: listed(keys),
    }
}

/// What a picked column holds, once it is known to be a value column of the
/// child it names rather than one of its keys.
fn value_kind(
    children: &[Candidate<'_>],
    keys: &BTreeSet<&str>,
    alias: &str,
    column: &str,
) -> Result<ColumnKind, CompositeError> {
    let Some(child) = children.iter().find(|child| child.alias.as_str() == alias) else {
        return Err(CompositeError::Unread(alias.to_owned()));
    };
    let values: BTreeSet<&str> = child
        .query
        .output_names()
        .into_iter()
        .filter(|name| !keys.contains(name))
        .collect();

    match kind_of(&child.kinds, column) {
        Some(kind) if values.contains(column) => Ok(kind),
        Some(_) | None => Err(CompositeError::NotAValue {
            alias: alias.to_owned(),
            column: column.to_owned(),
            values: listed(&values),
        }),
    }
}

fn kind_of(kinds: &[(String, ColumnKind)], column: &str) -> Option<ColumnKind> {
    kinds
        .iter()
        .find(|(name, _)| name == column)
        .map(|(_, kind)| *kind)
}

fn listed(names: &BTreeSet<&str>) -> String {
    if names.is_empty() {
        return "nothing".to_owned();
    }

    names
        .iter()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
