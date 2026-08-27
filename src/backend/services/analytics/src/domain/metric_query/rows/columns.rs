//! What a compiled page projects, as a caller reads it: which columns reach the
//! wire, what each of them is called, and where the next position comes from.
//!
//! INVARIANT: the ordering columns feed the cursor alone and never reach the
//! wire, so a served page discloses no position of its own.

use serde_json::Value;

use crate::domain::compiler::drilldown::{Contribution, DrilldownColumn, DrilldownColumnKind};
use crate::domain::field_catalog::model::DisplayRole;

use super::dto::{ColumnKind, RowColumn};

/// The alias a value is read from, beside the column it is reported as.
#[derive(Debug)]
struct Projected {
    alias: String,
    column: RowColumn,
}

/// How one compiled page is read: the columns it reports, and the aliases the
/// position of its last row is taken from.
#[derive(Debug)]
pub(super) struct PageShape {
    projected: Vec<Projected>,
    /// The ordering aliases in the order the page sorts by.
    sort_aliases: Vec<String>,
}

impl PageShape {
    /// INVARIANT: the ordering aliases keep the compiler's sort order, because
    /// a position read back in another order resumes at an unrelated row.
    pub(super) fn of(columns: &[DrilldownColumn], contribution: Contribution) -> Self {
        let mut ordering: Vec<(usize, String)> = Vec::new();
        let mut projected = Vec::with_capacity(columns.len());

        for column in columns {
            if let DrilldownColumnKind::SortKey(index) = column.kind {
                ordering.push((index, column.alias.clone()));
                continue;
            }
            let Some((key, kind)) = reported(&column.kind, contribution) else {
                continue;
            };
            projected.push(Projected {
                alias: column.alias.clone(),
                column: RowColumn {
                    label: humanized(&key),
                    key,
                    kind,
                },
            });
        }

        ordering.sort_by_key(|(index, _)| *index);
        Self {
            projected,
            sort_aliases: ordering.into_iter().map(|(_, alias)| alias).collect(),
        }
    }

    pub(super) fn columns(&self) -> Vec<RowColumn> {
        self.projected
            .iter()
            .map(|projected| projected.column.clone())
            .collect()
    }

    /// One row's reported values, in the columns' order. A column the read
    /// answered nothing for is unknown rather than absent.
    pub(super) fn row(&self, read: &serde_json::Map<String, Value>) -> Vec<Value> {
        self.projected
            .iter()
            .map(|projected| read.get(&projected.alias).cloned().unwrap_or(Value::Null))
            .collect()
    }

    /// Where a page ends, as the next one resumes from it.
    pub(super) fn position(&self, read: &serde_json::Map<String, Value>) -> Vec<String> {
        self.sort_aliases
            .iter()
            .map(|alias| match read.get(alias) {
                Some(Value::String(value)) => value.clone(),
                Some(value) => value.to_string(),
                None => String::new(),
            })
            .collect()
    }
}

/// What a projected column is called on the wire, and how its values read.
///
/// Three kinds are reported as none, each because the page already says it
/// better elsewhere: the input role, which the answer names once beside the
/// rows; the ordering values, which the cursor carries; and the contribution of
/// a row that was itself what the fold counted. A dimension is reported twice —
/// its label under the dimension's own key, because that is what a reader sees,
/// and its value under a suffixed one, because that is what the metric grouped
/// by.
fn reported(
    kind: &DrilldownColumnKind,
    contribution: Contribution,
) -> Option<(String, ColumnKind)> {
    match kind {
        DrilldownColumnKind::EntityId => Some(("subject".to_owned(), ColumnKind::Text)),
        DrilldownColumnKind::InputRole | DrilldownColumnKind::SortKey(_) => None,
        DrilldownColumnKind::Date => Some(("date".to_owned(), ColumnKind::Date)),
        DrilldownColumnKind::ObservedAt => Some(("observed_at".to_owned(), ColumnKind::Timestamp)),
        DrilldownColumnKind::Contribution => match contribution {
            Contribution::CountedRow => None,
            Contribution::MeasuredValue => Some(("value".to_owned(), ColumnKind::Number)),
        },
        DrilldownColumnKind::Subject => Some(("subject_key".to_owned(), ColumnKind::Text)),
        DrilldownColumnKind::Display(role) => Some((display_key(*role), ColumnKind::Text)),
        DrilldownColumnKind::DimensionLabel(key) => Some((key.clone(), ColumnKind::Text)),
        DrilldownColumnKind::DimensionValue(key) => {
            Some((format!("{key}_value"), ColumnKind::Text))
        }
    }
}

fn display_key(role: DisplayRole) -> String {
    match role {
        DisplayRole::Title => "title",
        DisplayRole::Reference => "reference",
        DisplayRole::Actor => "actor",
        DisplayRole::Location => "location",
        DisplayRole::Link => "link",
    }
    .to_owned()
}

/// One rule for every column: the key read as words, each of them capitalized.
fn humanized(key: &str) -> String {
    key.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut characters = word.chars();
            match characters.next() {
                Some(first) => first.to_uppercase().chain(characters).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn column(alias: &str, kind: DrilldownColumnKind) -> DrilldownColumn {
        DrilldownColumn {
            alias: alias.to_owned(),
            kind,
        }
    }

    fn every_kind() -> Vec<DrilldownColumn> {
        vec![
            column("entity_id", DrilldownColumnKind::EntityId),
            column("input_role", DrilldownColumnKind::InputRole),
            column("subject", DrilldownColumnKind::Subject),
            column("metric_date", DrilldownColumnKind::Date),
            column("observed_at", DrilldownColumnKind::ObservedAt),
            column("contribution", DrilldownColumnKind::Contribution),
            column(
                "display_title",
                DrilldownColumnKind::Display(DisplayRole::Title),
            ),
            column(
                "display_reference",
                DrilldownColumnKind::Display(DisplayRole::Reference),
            ),
            column(
                "display_actor",
                DrilldownColumnKind::Display(DisplayRole::Actor),
            ),
            column(
                "display_location",
                DrilldownColumnKind::Display(DisplayRole::Location),
            ),
            column(
                "display_link",
                DrilldownColumnKind::Display(DisplayRole::Link),
            ),
            column(
                "dim_0_value",
                DrilldownColumnKind::DimensionValue("repository".to_owned()),
            ),
            column(
                "dim_0_label",
                DrilldownColumnKind::DimensionLabel("repository".to_owned()),
            ),
            column("sort_1", DrilldownColumnKind::SortKey(1)),
            column("sort_0", DrilldownColumnKind::SortKey(0)),
        ]
    }

    fn keys(shape: &PageShape) -> Vec<String> {
        shape
            .columns()
            .into_iter()
            .map(|column| column.key)
            .collect()
    }

    #[test]
    fn every_projected_column_is_reported_under_the_name_the_contract_gives_it() {
        let shape = PageShape::of(&every_kind(), Contribution::MeasuredValue);

        assert_eq!(
            keys(&shape),
            [
                "subject",
                "subject_key",
                "date",
                "observed_at",
                "value",
                "title",
                "reference",
                "actor",
                "location",
                "link",
                "repository_value",
                "repository",
            ]
        );
    }

    #[test]
    fn a_row_that_was_itself_what_the_fold_counted_reports_no_number_of_its_own() {
        let counted = PageShape::of(&every_kind(), Contribution::CountedRow);

        assert!(
            !keys(&counted).contains(&"value".to_owned()),
            "every counted row contributes the same 1"
        );
    }

    #[test]
    fn the_ordering_columns_feed_the_position_and_never_the_page() {
        let shape = PageShape::of(&every_kind(), Contribution::MeasuredValue);
        let read = serde_json::json!({
            "sort_0": "row-7",
            "sort_1": "github",
            "entity_id": "person-1",
        });
        let read = read.as_object().expect("an object").clone();

        assert!(
            !keys(&shape).iter().any(|key| key.starts_with("sort")),
            "a page never reports what it is ordered by"
        );
        assert_eq!(shape.position(&read), ["row-7", "github"]);
    }

    #[test]
    fn a_column_kind_decides_how_its_values_read() {
        let shape = PageShape::of(&every_kind(), Contribution::MeasuredValue);
        let kinds: Vec<(String, ColumnKind)> = shape
            .columns()
            .into_iter()
            .map(|column| (column.key, column.kind))
            .collect();

        for (key, kind) in [
            ("subject", ColumnKind::Text),
            ("date", ColumnKind::Date),
            ("observed_at", ColumnKind::Timestamp),
            ("value", ColumnKind::Number),
            ("repository", ColumnKind::Text),
        ] {
            assert!(
                kinds.contains(&(key.to_owned(), kind)),
                "`{key}` reads as {kind:?}"
            );
        }
    }

    #[test]
    fn a_column_is_named_by_one_rule_rather_than_a_table_of_spellings() {
        let shape = PageShape::of(&every_kind(), Contribution::MeasuredValue);
        let labels: Vec<(String, String)> = shape
            .columns()
            .into_iter()
            .map(|column| (column.key, column.label))
            .collect();

        for (key, label) in [
            ("observed_at", "Observed At"),
            ("subject_key", "Subject Key"),
            ("repository_value", "Repository Value"),
            ("title", "Title"),
        ] {
            assert!(
                labels.contains(&(key.to_owned(), label.to_owned())),
                "`{key}` reads as `{label}`: {labels:?}"
            );
        }
    }

    #[test]
    fn a_row_reports_one_value_per_column_and_an_unanswered_one_as_unknown() {
        let shape = PageShape::of(
            &[
                column("entity_id", DrilldownColumnKind::EntityId),
                column("contribution", DrilldownColumnKind::Contribution),
                column("metric_date", DrilldownColumnKind::Date),
                column("sort_0", DrilldownColumnKind::SortKey(0)),
            ],
            Contribution::MeasuredValue,
        );
        let read = serde_json::json!({ "entity_id": "person-1", "contribution": 12.5 });
        let read = read.as_object().expect("an object").clone();

        assert_eq!(
            shape.row(&read),
            vec![
                serde_json::Value::from("person-1"),
                serde_json::Value::from(12.5),
                serde_json::Value::Null,
            ]
        );
    }
}
