//! The queryable surface of a dataset, which is less than its declaration says.
//!
//! INVARIANT: where the rows live — database, relation, read discipline, tenancy
//! column, row identity — is never served here.

use serde::Serialize;
use utoipa::ToSchema;

use crate::domain::query::contract::dto::{
    DEFAULT_ROW_LIMIT, MAX_AGGREGATES, MAX_FILTER_VALUES, MAX_FILTERS, MAX_GROUP_AXES,
    MAX_NAME_CHARS, MAX_ORDER_TERMS, MAX_ROW_LIMIT,
};

use super::declaration::{Dataset, Dimension, Measurable, TimeField};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryDatasetList)]
pub struct DatasetListDescription {
    pub datasets: Vec<DatasetDescription>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryDataset)]
pub struct DatasetDescription {
    pub key: String,
    /// The columns a query may bound its window by, and bucket on.
    pub time_fields: Vec<TimeFieldDescription>,
    /// The axes a query may group by, and filter on beside the measurables.
    pub dimensions: Vec<DimensionDescription>,
    /// The columns an aggregate may fold.
    pub measurables: Vec<MeasurableDescription>,
    /// The bounds a request against this dataset must stay inside.
    pub limits: QueryLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryDatasetTimeField)]
pub struct TimeFieldDescription {
    pub field: String,
    /// The field a query's window binds to when it names none.
    pub default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryDatasetDimension)]
pub struct DimensionDescription {
    pub field: String,
    /// The value absent rows group under, and a filter matches them by.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absent_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryDatasetMeasurable)]
pub struct MeasurableDescription {
    pub field: String,
}

/// The contract's request bounds, identical for every dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[schema(as = QueryLimits)]
pub struct QueryLimits {
    pub max_filters: usize,
    pub max_filter_values: usize,
    pub max_aggregates: usize,
    pub max_group_axes: usize,
    pub max_order_terms: usize,
    /// Longest an aggregate's name may be, in characters.
    pub max_name_chars: usize,
    /// Rows a query gets when it sets no ceiling of its own.
    pub default_limit: u32,
    /// Rows a query may ask for at most; over it the query is refused, not clipped.
    pub max_limit: u32,
}

impl QueryLimits {
    const fn contract() -> Self {
        Self {
            max_filters: MAX_FILTERS,
            max_filter_values: MAX_FILTER_VALUES,
            max_aggregates: MAX_AGGREGATES,
            max_group_axes: MAX_GROUP_AXES,
            max_order_terms: MAX_ORDER_TERMS,
            max_name_chars: MAX_NAME_CHARS,
            default_limit: DEFAULT_ROW_LIMIT,
            max_limit: MAX_ROW_LIMIT,
        }
    }
}

pub fn describe(dataset: &Dataset) -> DatasetDescription {
    DatasetDescription {
        key: dataset.key.to_string(),
        time_fields: dataset.time_fields.iter().map(time_field).collect(),
        dimensions: dataset.dimensions.iter().map(dimension).collect(),
        measurables: dataset.measurables.iter().map(measurable).collect(),
        limits: QueryLimits::contract(),
    }
}

pub fn describe_all(datasets: &[Dataset]) -> DatasetListDescription {
    DatasetListDescription {
        datasets: datasets.iter().map(describe).collect(),
    }
}

fn time_field(field: &TimeField) -> TimeFieldDescription {
    TimeFieldDescription {
        field: field.field.clone(),
        default: field.default,
    }
}

fn dimension(dimension: &Dimension) -> DimensionDescription {
    DimensionDescription {
        field: dimension.field.clone(),
        absent_value: dimension.absent_value.clone(),
    }
}

fn measurable(measurable: &Measurable) -> MeasurableDescription {
    MeasurableDescription {
        field: measurable.field.clone(),
    }
}

impl toolkit::api::api_dto::ResponseApiDto for DatasetListDescription {}
impl toolkit::api::api_dto::ResponseApiDto for DatasetDescription {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::{dataset, product_datasets};
    use super::*;

    fn description(key: &str) -> DatasetDescription {
        describe(dataset(key).unwrap_or_else(|| panic!("{key} is declared")))
    }

    #[test]
    fn a_description_reports_every_field_its_declaration_admits_and_no_other() {
        for key in ["git_commits", "git_file_changes"] {
            let declared = dataset(key).unwrap_or_else(|| panic!("{key} is declared"));
            let described = description(key);

            assert_eq!(described.key, key);
            assert_eq!(
                described
                    .time_fields
                    .iter()
                    .map(|field| field.field.as_str())
                    .collect::<Vec<_>>(),
                declared.time_field_names(),
                "{key}: time fields"
            );
            assert_eq!(
                described
                    .dimensions
                    .iter()
                    .map(|dimension| dimension.field.as_str())
                    .collect::<Vec<_>>(),
                declared.dimension_names(),
                "{key}: dimensions"
            );
            assert_eq!(
                described
                    .measurables
                    .iter()
                    .map(|measurable| measurable.field.as_str())
                    .collect::<Vec<_>>(),
                declared.measurable_names(),
                "{key}: measurables"
            );
        }
    }

    #[test]
    fn a_described_dimension_names_absent_rows_exactly_as_its_declaration_does() {
        for key in ["git_commits", "git_file_changes"] {
            let declared = dataset(key).unwrap_or_else(|| panic!("{key} is declared"));
            for described in description(key).dimensions {
                let source = declared
                    .dimension(&described.field)
                    .unwrap_or_else(|| panic!("{key}: {} is declared", described.field));

                assert_eq!(
                    described.absent_value, source.absent_value,
                    "{key}: {} absent value",
                    described.field
                );
            }
        }
    }

    #[test]
    fn the_time_field_a_window_binds_to_by_default_is_the_one_flagged_default() {
        for key in ["git_commits", "git_file_changes"] {
            let declared = dataset(key).unwrap_or_else(|| panic!("{key} is declared"));
            let described = description(key);

            let defaults: Vec<&str> = described
                .time_fields
                .iter()
                .filter(|field| field.default)
                .map(|field| field.field.as_str())
                .collect();

            assert_eq!(
                defaults,
                [declared
                    .default_time_field()
                    .map(|field| field.field.as_str())
                    .expect("a declaration carries one default time field")],
                "{key}: default time field"
            );
        }
    }

    #[test]
    fn a_description_carries_nothing_about_where_the_rows_live() {
        let json = serde_json::to_value(description("git_commits"))
            .expect("a description serializes")
            .to_string();

        for internal in ["insight", "tenant_id", "commit_hash", "plain"] {
            assert!(
                !json.contains(internal),
                "the description leaks `{internal}`: {json}"
            );
        }
    }

    #[test]
    fn every_declared_dataset_is_listed_once_in_declaration_order() {
        let datasets = product_datasets().expect("the shipped declarations are admissible");
        let described = describe_all(datasets);
        let listed: Vec<&str> = described
            .datasets
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();

        assert_eq!(listed, ["git_commits", "git_file_changes"]);
    }

    #[test]
    fn the_limits_a_dataset_reports_are_the_ones_validation_enforces() {
        let limits = description("git_commits").limits;

        assert_eq!(limits.max_filters, MAX_FILTERS);
        assert_eq!(limits.max_filter_values, MAX_FILTER_VALUES);
        assert_eq!(limits.max_aggregates, MAX_AGGREGATES);
        assert_eq!(limits.max_group_axes, MAX_GROUP_AXES);
        assert_eq!(limits.max_order_terms, MAX_ORDER_TERMS);
        assert_eq!(limits.max_name_chars, MAX_NAME_CHARS);
        assert_eq!(limits.default_limit, DEFAULT_ROW_LIMIT);
        assert_eq!(limits.max_limit, MAX_ROW_LIMIT);
    }
}
