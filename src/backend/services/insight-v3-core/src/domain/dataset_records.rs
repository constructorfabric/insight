//! Looking at what a dataset holds: one page of its records, ordered by
//! the instant they arrived or by a field the dataset declares.

use super::datasets::{self, Datasets, Reads};
use super::definition::DefinitionName;
use super::kinds::dataset::declaration::{Declaration, Source};
use super::kinds::dataset::read::{Form, PAYLOAD_COLUMN, read};
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Page, Record};
use crate::store::relations::{RelationError, Relations};

/// The column every record sent into a dataset carries, whatever the dataset
/// declares. A relation the warehouse builds has none.
pub(crate) const ARRIVED: &str = "received_at";

/// Reads a dataset's own records, for a reader deciding whether what arrives
/// is what they meant to send.
#[derive(Debug)]
pub(crate) struct DatasetRecords<'a> {
    datasets: &'a dyn Datasets,
    tables: &'a DatasetTables,
    relations: &'a Relations,
    /// How many records one look shows, whatever the dataset holds.
    cap: u64,
}

impl<'a> DatasetRecords<'a> {
    pub(crate) fn new(
        datasets: &'a dyn Datasets,
        tables: &'a DatasetTables,
        relations: &'a Relations,
        cap: u64,
    ) -> Self {
        Self {
            datasets,
            tables,
            relations,
            cap,
        }
    }

    /// One page of a dataset's records, and how many it holds in all.
    ///
    /// A dataset mid-create or mid-removal answers nothing: there is no table
    /// to look in, or there is about to be none.
    pub(crate) async fn page(
        &self,
        name: &DefinitionName,
        look: &Look,
    ) -> Result<Preview, PreviewError> {
        let ready = datasets::ready(self.datasets, name.as_str())
            .await
            .map_err(PreviewError::Store)?
            .ok_or_else(|| PreviewError::NotReady(name.as_str().to_owned()))?;

        let order = ordering(&ready.declaration, look)?;
        let limit = shown(look.limit, self.cap)?;

        // Two reads, because the two hold their rows differently: a record
        // sent in is a payload stamped with when it arrived, and a row of a
        // relation is columns and nothing else. Both are shown through the
        // fields the dataset declares, and both page the same way.
        let Reads::Ours(ours) = ready.reads else {
            let (database, table) = ready.reads.at("");
            return self
                .of_a_relation(database, table, &ready.declaration, &order, look, limit)
                .await;
        };

        let page = Page {
            limit,
            offset: look.offset,
            order: &order,
        };

        let looked = async {
            let records = self.tables.page(&ours, page).await?;
            let total = self.tables.count(&ours).await?;
            Ok::<_, DatasetTableError>(Preview {
                records,
                total,
                limit,
            })
        };

        match looked.await {
            Ok(preview) => Ok(preview),
            // The dataset went while this read was in flight, which is the
            // same answer as its never having been there.
            Err(DatasetTableError::Vanished) => {
                Err(PreviewError::NotReady(name.as_str().to_owned()))
            }
            Err(error) => Err(PreviewError::Table(error)),
        }
    }

    /// One page of a relation the warehouse builds, through the fields the
    /// dataset declares over it.
    async fn of_a_relation(
        &self,
        database: &str,
        table: &str,
        declaration: &Declaration,
        order: &str,
        look: &Look,
        limit: u64,
    ) -> Result<Preview, PreviewError> {
        let reads: Vec<(String, String)> = declaration
            .fields
            .iter()
            .map(|field| {
                (
                    field.name.clone(),
                    read(field, Form::Presented, PAYLOAD_COLUMN),
                )
            })
            .collect();

        let page = crate::store::relations::Page {
            limit,
            offset: look.offset,
            order,
        };
        let rows = self.relations.rows(database, table, &reads, page).await?;
        let total = self.relations.count(database, table).await?;

        Ok(Preview {
            records: rows
                .into_iter()
                .map(|raw_data| Record {
                    id: None,
                    received_at: None,
                    raw_data,
                })
                .collect(),
            total,
            limit,
        })
    }
}

/// What a reader asked one look for.
#[derive(Debug)]
pub(crate) struct Look {
    pub(crate) limit: Option<u64>,
    pub(crate) offset: u64,
    /// A declared field, or `received_at`. Absent means the order records
    /// arrived in.
    pub(crate) order_by: Option<String>,
    pub(crate) descending: bool,
}

/// How the page is ordered: by a declared field, read as the declaration says
/// it is read, or by the instant the record arrived.
fn ordering(declaration: &Declaration, look: &Look) -> Result<String, PreviewError> {
    let direction = if look.descending { "DESC" } else { "ASC" };
    let Some(named) = look.order_by.as_deref() else {
        return Ok(unasked(declaration, direction));
    };

    // A declared field wins a name it shares with the arrival column: the
    // reader is ordering by the column they can see, and nothing stops a
    // declaration naming a field `received_at`.
    let declared = declaration.fields.iter().find(|field| field.name == named);
    if declared.is_none() && named == ARRIVED && declaration.source == Source::Stream {
        return Ok(format!("{ARRIVED} {direction}"));
    }

    let field = declared.ok_or_else(|| PreviewError::NoSuchField {
        named: named.to_owned(),
        declared: declaration
            .fields
            .iter()
            .map(|field| field.name.clone())
            .collect(),
    })?;

    // A record that does not carry the field reads as NULL, and ClickHouse
    // sorts NULL above every value: without this the first page of a sparse
    // field is nothing but blanks.
    Ok(format!(
        "{} {direction} NULLS LAST",
        read(field, Form::Raw, PAYLOAD_COLUMN)
    ))
}

/// How a page is ordered when the reader names nothing.
///
/// A record sent into a dataset arrived at an instant, and the latest few
/// are what a reader wants. A relation has no such instant, so the order has
/// to come from the declaration — its main date where it names one, and its
/// first field otherwise. Arbitrary, but the same every time: a page read by
/// offset with no order at all would show a reader the same row twice and
/// skip another, and neither would be visible.
fn unasked(declaration: &Declaration, direction: &str) -> String {
    if declaration.source == Source::Stream {
        return format!("{ARRIVED} {direction}");
    }

    let field = declaration
        .default_clock()
        .or_else(|| declaration.fields.first());

    field.map_or_else(
        || "1".to_owned(),
        |field| {
            format!(
                "{} {direction} NULLS LAST",
                read(field, Form::Raw, PAYLOAD_COLUMN)
            )
        },
    )
}

/// What one look at a dataset shows: a page of its records, how many there
/// are in all, and how wide the page was.
#[derive(Debug)]
pub(crate) struct Preview {
    pub(crate) records: Vec<Record>,
    pub(crate) total: u64,
    /// How many records this page could hold. A reader paging by offset needs
    /// it: the cap is an installation's setting, not a number a caller knows.
    pub(crate) limit: u64,
}

/// How many records a look shows: what was asked for, or the cap when nothing
/// was asked.
///
/// SAFETY: a size outside the range is refused rather than quietly brought
/// into it. A caller paging by the size it asked for would step over the
/// records a smaller page left behind, and nothing in the answer would say so.
fn shown(wanted: Option<u64>, cap: u64) -> Result<u64, PreviewError> {
    match wanted {
        None => Ok(cap),
        Some(asked) if asked >= 1 && asked <= cap => Ok(asked),
        Some(asked) => Err(PreviewError::PageSize { asked, cap }),
    }
}

#[cfg(test)]
mod tests {
    use super::{ARRIVED, Declaration, Look, ordering, shown};

    fn declaration() -> Declaration {
        serde_json::from_value(serde_json::json!({
            "title": "Commits",
            "fields": [
                {"name": "day", "path": "committed_at", "type": "datetime"},
                {"name": "lines", "path": "stats.lines", "type": "int"},
            ],
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
    }

    fn look(order_by: Option<&str>, descending: bool) -> Look {
        Look {
            limit: None,
            offset: 0,
            order_by: order_by.map(str::to_owned),
            descending,
        }
    }

    #[test]
    fn a_page_naming_no_field_is_ordered_by_the_instant_records_arrived() {
        assert_eq!(
            ordering(&declaration(), &look(None, true)).ok(),
            Some(format!("{ARRIVED} DESC"))
        );
    }

    /// A direction that is accepted, validated and then dropped is a lie the
    /// caller cannot see.
    #[test]
    fn a_direction_is_honoured_even_where_no_field_is_named() {
        assert_eq!(
            ordering(&declaration(), &look(None, false)).ok(),
            Some(format!("{ARRIVED} ASC"))
        );
        assert_eq!(
            ordering(&declaration(), &look(Some(ARRIVED), false)).ok(),
            Some(format!("{ARRIVED} ASC"))
        );
    }

    /// The declaration says how a field is read; ordering reads it the same
    /// way a metric would, so the page is ordered by the value, not the text.
    #[test]
    fn a_named_field_is_ordered_by_the_value_the_declaration_reads() {
        let Ok(ordered) = ordering(&declaration(), &look(Some("lines"), true)) else {
            panic!("`lines` is declared")
        };

        assert_eq!(
            ordered,
            "JSONExtract(raw_data, 'stats', 'lines', 'Nullable(Int64)') DESC NULLS LAST"
        );
    }

    /// A record that does not carry the field reads as NULL, and ClickHouse
    /// sorts NULL above every value: without NULLS LAST the first page of a
    /// sparse field is nothing but blanks.
    #[test]
    fn a_page_ordered_by_a_field_puts_the_records_missing_it_last() {
        for descending in [true, false] {
            let Ok(ordered) = ordering(&declaration(), &look(Some("day"), descending)) else {
                panic!("`day` is declared")
            };

            assert!(ordered.ends_with("NULLS LAST"), "got {ordered}");
        }
    }

    #[test]
    fn a_page_ordered_by_a_field_the_dataset_does_not_declare_is_refused() {
        assert!(ordering(&declaration(), &look(Some("nonsense"), true)).is_err());
    }

    #[test]
    fn a_page_holds_what_was_asked_for_or_the_cap_when_nothing_was() {
        assert_eq!(shown(None, 50).ok(), Some(50));
        assert_eq!(shown(Some(20), 50).ok(), Some(20));
        assert_eq!(shown(Some(50), 50).ok(), Some(50));
    }

    /// Cutting a page down quietly would let a caller paging by the size it
    /// asked for step over the records the smaller page left behind. A page
    /// of none is no page at all, and the catalogue refuses it too.
    #[test]
    fn a_page_of_a_size_this_installation_does_not_serve_is_refused() {
        for asked in [0, 51, 10_000] {
            assert!(shown(Some(asked), 50).is_err(), "asked {asked}");
        }
    }

    fn over_a_relation() -> Declaration {
        serde_json::from_value(serde_json::json!({
            "title": "Collaboration observations",
            "source": {
                "kind": "relation",
                "database": "insight",
                "table": "collab_metric_observations"
            },
            "fields": [
                { "name": "team", "column": "entity_id", "type": "string" },
                { "name": "day", "column": "metric_date", "type": "datetime", "default_clock": true }
            ]
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"))
    }

    /// A relation has no arrival instant, so a page over one that named no
    /// order would have none at all — and a reader stepping by offset would
    /// be shown one row twice and never shown another, with nothing saying
    /// so. The declaration's main date is the order instead.
    #[test]
    fn a_page_of_a_relation_naming_no_order_is_ordered_by_its_main_date() {
        let ordered = ordering(&over_a_relation(), &look(None, true));

        assert_eq!(
            ordered.ok(),
            Some("accurateCastOrNull(`metric_date`, 'DateTime64(3)') DESC NULLS LAST".to_owned())
        );
    }

    /// A relation that dates nothing still has to be ordered by something
    /// that does not move between pages.
    #[test]
    fn a_relation_that_names_no_main_date_is_ordered_by_its_first_field() {
        let undated: Declaration = serde_json::from_value(serde_json::json!({
            "title": "Collaboration observations",
            "source": { "kind": "relation", "database": "insight", "table": "collab" },
            "fields": [{ "name": "team", "column": "entity_id", "type": "string" }]
        }))
        .unwrap_or_else(|error| panic!("the fixture parses: {error}"));

        let ordered = ordering(&undated, &look(None, false));

        assert_eq!(
            ordered.ok(),
            Some("toString(`entity_id`) ASC NULLS LAST".to_owned())
        );
    }

    /// The arrival column is a column of a table this service made. A dataset
    /// over a relation has none, so naming it is naming a field the dataset
    /// does not declare.
    #[test]
    fn a_relation_has_no_arrival_column_to_be_ordered_by() {
        assert!(ordering(&over_a_relation(), &look(Some(ARRIVED), true)).is_err());
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum PreviewError {
    #[error("`{named}` is not a field of this dataset; it declares {}", declared.join(", "))]
    NoSuchField {
        named: String,
        declared: Vec<String>,
    },
    #[error("a page holds 1 to {cap} records; {asked} were asked for")]
    PageSize { asked: u64, cap: u64 },
    #[error("no dataset named `{0}` is ready to be read")]
    NotReady(String),
    #[error(transparent)]
    Relation(#[from] RelationError),
    #[error(transparent)]
    Store(crate::domain::datasets::DatasetStoreError),
    #[error(transparent)]
    Table(DatasetTableError),
}
