use super::datasets::{self, Datasets};
use super::definition::DefinitionId;
use super::kinds::dataset::declaration::Declaration;
use super::kinds::dataset::read::{Form, PAYLOAD_COLUMN, read};
use crate::store::dataset_tables::{DatasetTableError, DatasetTables, Page, Record};

pub(crate) const ARRIVED: &str = "received_at";

#[derive(Debug)]
pub(crate) struct DatasetRecords<'a> {
    datasets: &'a dyn Datasets,
    tables: &'a DatasetTables,
    cap: u64,
}

impl<'a> DatasetRecords<'a> {
    pub(crate) fn new(datasets: &'a dyn Datasets, tables: &'a DatasetTables, cap: u64) -> Self {
        Self {
            datasets,
            tables,
            cap,
        }
    }

    pub(crate) async fn page(
        &self,
        name: &DefinitionId,
        look: &Look,
    ) -> Result<Preview, PreviewError> {
        let ready = datasets::ready(self.datasets, name.as_str())
            .await
            .map_err(PreviewError::Store)?
            .ok_or_else(|| PreviewError::NotReady(name.as_str().to_owned()))?;

        let order = ordering(&ready.declaration, look)?;
        let limit = shown(look.limit, self.cap)?;
        let page = Page {
            limit,
            offset: look.offset,
            order: &order,
        };

        let looked = async {
            let records = self.tables.page(&ready.table, page).await?;
            let total = self.tables.count(&ready.table).await?;
            Ok::<_, DatasetTableError>(Preview {
                records,
                total,
                limit,
            })
        };

        match looked.await {
            Ok(preview) => Ok(preview),
            Err(DatasetTableError::Vanished) => {
                Err(PreviewError::NotReady(name.as_str().to_owned()))
            }
            Err(error) => Err(PreviewError::Table(error)),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Look {
    pub(crate) limit: Option<u64>,
    pub(crate) offset: u64,
    pub(crate) order_by: Option<String>,
    pub(crate) descending: bool,
}

fn ordering(declaration: &Declaration, look: &Look) -> Result<String, PreviewError> {
    let direction = if look.descending { "DESC" } else { "ASC" };
    let Some(named) = look.order_by.as_deref() else {
        return Ok(format!("{ARRIVED} {direction}"));
    };

    let declared = declaration.fields.iter().find(|field| field.name == named);
    if declared.is_none() && named == ARRIVED {
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

    Ok(format!(
        "{} {direction} NULLS LAST",
        read(field, Form::Raw, PAYLOAD_COLUMN)
    ))
}

#[derive(Debug)]
pub(crate) struct Preview {
    pub(crate) records: Vec<Record>,
    pub(crate) total: u64,
    pub(crate) limit: u64,
}

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

    #[test]
    fn a_page_of_a_size_this_installation_does_not_serve_is_refused() {
        for asked in [0, 51, 10_000] {
            assert!(shown(Some(asked), 50).is_err(), "asked {asked}");
        }
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
    Store(crate::domain::datasets::DatasetStoreError),
    #[error(transparent)]
    Table(DatasetTableError),
}
