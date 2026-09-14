use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use toolkit_canonical_errors::CanonicalError;

use crate::domain::external_links::ExternalSourceRegistry;
use crate::domain::metric_definitions::ComputationSpec;
use crate::domain::metric_definitions::definition::MetricInputRole;
use crate::domain::metric_definitions::{EvidenceColumnType, EvidenceDetailColumn};

use super::cursor::encode_cursor;
use super::dto::{
    EvidencePlan, EvidenceQueryRow, MetricDrilldownColumn, MetricDrilldownColumnType,
    MetricDrilldownEntity, MetricDrilldownFilter, MetricDrilldownResponse, MetricDrilldownRow,
    PersonNames, ValidatedMetricDrilldown,
};
use super::error::config_error;
use super::sort::{PERSON_KEY, is_sortable};

#[derive(Debug, Deserialize)]
struct EvidenceDimension {
    key: String,
    value: String,
    label: Option<String>,
}

pub fn build_response(
    req: &ValidatedMetricDrilldown,
    mut rows: Vec<EvidenceQueryRow>,
    names: &PersonNames,
    external_links: &ExternalSourceRegistry,
) -> Result<MetricDrilldownResponse, CanonicalError> {
    let next_cursor = if rows.len() > req.limit {
        rows.truncate(req.limit);
        rows.last()
            .map(|row| encode_cursor(&req.fingerprint, &req.snapshot_id, row))
            .transpose()?
    } else {
        None
    };
    let (columns, rows) = present(
        &rows,
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
        &req.selection.entity,
        names,
        Some(external_links),
    )?;
    Ok(MetricDrilldownResponse {
        selection: req.selection.clone(),
        columns,
        rows,
        next_cursor,
    })
}

pub fn presentation(
    rows: &[EvidenceQueryRow],
    plan: &EvidencePlan,
    filters: &[MetricDrilldownFilter],
    display_dimensions: &[String],
    entity: &MetricDrilldownEntity,
    names: &PersonNames,
) -> Result<(Vec<MetricDrilldownColumn>, Vec<MetricDrilldownRow>), CanonicalError> {
    present(rows, plan, filters, display_dimensions, entity, names, None)
}

fn present(
    rows: &[EvidenceQueryRow],
    plan: &EvidencePlan,
    filters: &[MetricDrilldownFilter],
    display_dimensions: &[String],
    entity: &MetricDrilldownEntity,
    names: &PersonNames,
    external_links: Option<&ExternalSourceRegistry>,
) -> Result<(Vec<MetricDrilldownColumn>, Vec<MetricDrilldownRow>), CanonicalError> {
    let details = rows
        .iter()
        .map(|row| row.details.as_object().ok_or_else(config_error))
        .collect::<Result<Vec<_>, _>>()?;
    let dimensions = presentation_dimensions(rows)?;
    let columns = presentation_columns(plan, filters, display_dimensions, entity);

    let projected_rows = rows
        .iter()
        .zip(details)
        .zip(dimensions)
        .map(|((row, details), dimensions)| {
            project_row(row, details, &dimensions, &columns, names, external_links)
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;

    Ok((columns, projected_rows))
}

/// Whether this selection presents the `person` column — the one read that has
/// to resolve names, and so the only one that should pay for it.
pub(crate) fn presents_person(req: &ValidatedMetricDrilldown) -> bool {
    presentation_columns(
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
        &req.selection.entity,
    )
    .iter()
    .any(|column| column.key == PERSON_KEY)
}

/// INVARIANT: the query compiler builds its ORDER BY and its search from this
/// same list, so a column the reader sees is a column the query can name.
pub(super) fn presentation_columns(
    plan: &EvidencePlan,
    filters: &[MetricDrilldownFilter],
    display_dimensions: &[String],
    entity: &MetricDrilldownEntity,
) -> Vec<MetricDrilldownColumn> {
    // A ratio row is one side of a division, so the records behind its two
    // measures are different things and neither measure's columns describe the
    // pair: the two numbers are what the row has to say.
    let ratio = matches!(plan.definition.spec, ComputationSpec::Ratio { .. });
    let declared = if ratio {
        Vec::new()
    } else {
        declared_columns(plan)
    };
    // Both sources of dimension columns drop together for a ratio: the outer
    // aggregate carries no cell the query can order or search, so a column
    // rendered from it would answer neither.
    let display_dimensions: &[String] = if ratio { &[] } else { display_dimensions };
    let filters: &[MetricDrilldownFilter] = if ratio { &[] } else { filters };

    let mut columns = Vec::new();
    // A roster's records come from many people, and which one is the first
    // thing the reader needs. One person's own drilldown already knows.
    if !ratio && matches!(entity, MetricDrilldownEntity::Persons { .. }) {
        columns.push(MetricDrilldownColumn {
            key: PERSON_KEY.to_owned(),
            label: "Who".to_owned(),
            r#type: MetricDrilldownColumnType::String,
            sortable: false,
        });
    }
    columns.extend(declared.iter().map(detail_column));
    columns.extend(
        dimension_keys(filters, display_dimensions, &declared)
            .into_iter()
            .map(dimension_column),
    );
    columns.push(MetricDrilldownColumn {
        key: "date".to_owned(),
        label: "Date".to_owned(),
        r#type: MetricDrilldownColumnType::Date,
        sortable: true,
    });

    if ratio {
        columns.push(input_column("numerator", plan, MetricInputRole::Numerator));
        columns.push(input_column(
            "denominator",
            plan,
            MetricInputRole::Denominator,
        ));
    } else if plan
        .inputs
        .iter()
        .any(|input| input.presentation.show_value)
    {
        columns.push(MetricDrilldownColumn {
            key: "value".to_owned(),
            label: "Value".to_owned(),
            r#type: MetricDrilldownColumnType::Number,
            sortable: true,
        });
    }
    for column in &mut columns {
        column.sortable = column.sortable && is_sortable(&column.key, column.r#type, ratio);
    }
    columns
}

/// The detail columns the plan's measures declare, in declaration order, each
/// key kept once — a metric reading two measures of the same shape shows that
/// shape once.
fn declared_columns(plan: &EvidencePlan) -> Vec<EvidenceDetailColumn> {
    let mut seen = BTreeSet::new();
    plan.inputs
        .iter()
        .flat_map(|input| &input.presentation.detail_columns)
        .filter(|column| seen.insert(column.key.as_str()))
        .cloned()
        .collect()
}

/// The dimensions the selection pinned to one value, plus the ones it asked to
/// display. A dimension a measure already declares as a detail is dropped here:
/// the declared column carries it, and the row projection falls back to the
/// dimension when the details map has nothing under that key.
fn dimension_keys(
    filters: &[MetricDrilldownFilter],
    display_dimensions: &[String],
    declared: &[EvidenceDetailColumn],
) -> BTreeSet<String> {
    let declared_keys = declared
        .iter()
        .map(|column| column.key.as_str())
        .collect::<BTreeSet<_>>();
    filters
        .iter()
        .filter(|filter| filter.values.len() == 1)
        .map(|filter| filter.dimension.clone())
        .chain(display_dimensions.iter().cloned())
        .filter(|key| !declared_keys.contains(key.as_str()))
        .collect()
}

fn detail_column(column: &EvidenceDetailColumn) -> MetricDrilldownColumn {
    MetricDrilldownColumn {
        key: column.key.clone(),
        label: column.label.clone(),
        r#type: match column.r#type {
            EvidenceColumnType::String => MetricDrilldownColumnType::String,
            EvidenceColumnType::Number => MetricDrilldownColumnType::Number,
            EvidenceColumnType::Date => MetricDrilldownColumnType::Date,
        },
        sortable: true,
    }
}

fn dimension_column(key: String) -> MetricDrilldownColumn {
    MetricDrilldownColumn {
        label: humanize_field_name(&key),
        key,
        r#type: MetricDrilldownColumnType::String,
        sortable: true,
    }
}

fn input_column(key: &str, plan: &EvidencePlan, role: MetricInputRole) -> MetricDrilldownColumn {
    MetricDrilldownColumn {
        key: key.to_owned(),
        label: input_label(plan, role),
        r#type: MetricDrilldownColumnType::Number,
        sortable: true,
    }
}

fn project_row(
    row: &EvidenceQueryRow,
    details: &serde_json::Map<String, serde_json::Value>,
    dimensions: &[EvidenceDimension],
    columns: &[MetricDrilldownColumn],
    names: &PersonNames,
    external_links: Option<&ExternalSourceRegistry>,
) -> Result<MetricDrilldownRow, CanonicalError> {
    let mut values = BTreeMap::new();
    for column in columns {
        let value = match column.key.as_str() {
            PERSON_KEY => person_value(&row.person_id, names),
            "date" => row.metric_date.clone().into(),
            "value" => serde_json::to_value(row.contribution).map_err(|_| config_error())?,
            "numerator" => serde_json::to_value(row.numerator).map_err(|_| config_error())?,
            "denominator" => serde_json::to_value(row.denominator).map_err(|_| config_error())?,
            key => details
                .get(key)
                .filter(|value| visible_value(value))
                .cloned()
                .or_else(|| {
                    dimensions
                        .iter()
                        .find(|dimension| dimension.key == key)
                        .map(|dimension| {
                            serde_json::Value::from(
                                dimension
                                    .label
                                    .as_deref()
                                    .filter(|label| !label.trim().is_empty())
                                    .unwrap_or(&dimension.value),
                            )
                        })
                })
                .unwrap_or(serde_json::Value::Null),
        };
        values.insert(
            column.key.clone(),
            coerce_to_column_type(column.r#type, value),
        );
    }
    let links = external_links
        .map(|registry| row_links(registry, row, details, dimensions, &values))
        .unwrap_or_default();
    Ok(MetricDrilldownRow { values, links })
}

fn row_links(
    registry: &ExternalSourceRegistry,
    row: &EvidenceQueryRow,
    details: &serde_json::Map<String, serde_json::Value>,
    dimensions: &[EvidenceDimension],
    values: &BTreeMap<String, serde_json::Value>,
) -> BTreeMap<String, String> {
    let Some(provider) = dimensions
        .iter()
        .find(|dimension| dimension.key == "source")
        .map(|dimension| dimension.value.as_str())
    else {
        return BTreeMap::new();
    };
    let Some(source_id) = details.get("source_id").and_then(serde_json::Value::as_str) else {
        return BTreeMap::new();
    };
    let repository = details
        .get("repository")
        .and_then(serde_json::Value::as_str);
    let record_ref = details.get("ref").and_then(serde_json::Value::as_str);
    let resolved = registry.evidence_links(
        provider,
        source_id,
        &row.record_kind,
        repository,
        record_ref,
    );
    let mut links = BTreeMap::new();
    if values.get("repository").is_some_and(visible_value)
        && let Some(href) = resolved.repository
    {
        links.insert("repository".to_owned(), href);
    }
    if let Some(href) = resolved.record {
        for key in ["ref", "title"] {
            if values.get(key).is_some_and(visible_value) {
                links.insert(key.to_owned(), href.clone());
            }
        }
    }
    links
}

fn presentation_dimensions(
    rows: &[EvidenceQueryRow],
) -> Result<Vec<Vec<EvidenceDimension>>, CanonicalError> {
    rows.iter()
        .map(|row| {
            serde_json::from_str::<Vec<EvidenceDimension>>(&row.dimensions_json)
                .map_err(|_| config_error())
        })
        .collect()
}

fn input_label(plan: &EvidencePlan, role: MetricInputRole) -> String {
    plan.inputs
        .iter()
        .find(|input| input.role == role)
        .map_or_else(
            || humanize_field_name(role.as_db()),
            |input| humanize_field_name(&input.measure_key),
        )
}

/// Who a row belongs to, in the words the reader knows them by.
///
/// A person the identity map could not resolve, or could not name, leaves the
/// cell empty: an id is not an answer to "who", and it would be exported as
/// one.
fn person_value(person_id: &str, names: &PersonNames) -> serde_json::Value {
    names
        .get(person_id.trim())
        .filter(|name| !name.trim().is_empty())
        .map_or(serde_json::Value::Null, |name| {
            serde_json::Value::from(name.as_str())
        })
}

fn visible_value(value: &serde_json::Value) -> bool {
    !value.is_null() && value.as_str().is_none_or(|value| !value.trim().is_empty())
}

fn coerce_to_column_type(
    r#type: MetricDrilldownColumnType,
    value: serde_json::Value,
) -> serde_json::Value {
    let MetricDrilldownColumnType::Number = r#type else {
        return value;
    };
    let Some(text) = value.as_str() else {
        return value;
    };
    if let Ok(number) = text.parse::<i64>() {
        return serde_json::Value::from(number);
    }
    if let Ok(number) = text.parse::<f64>() {
        return serde_json::Value::from(number);
    }
    value
}

fn humanize_field_name(key: &str) -> String {
    let label = key.replace('_', " ");
    let mut characters = label.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::definition::AliasCollapse;

    use crate::config::{ExternalSourceConfig, ExternalSourceProvider};
    use crate::domain::metric_definitions::EvidencePresentation;
    use crate::domain::metric_definitions::{EvidenceGranularity, RatioDenominatorAggregation};
    use crate::domain::metric_drilldown::cursor::decode_cursor;
    use crate::domain::metric_drilldown::cursor::selection_fingerprint;
    use crate::domain::metric_drilldown::dto::EvidenceInput;
    use crate::domain::metric_drilldown::test_support::{
        TEST_TENANT, commit_input, commit_presentation, input, plan, row, validated,
    };

    fn one_person() -> MetricDrilldownEntity {
        MetricDrilldownEntity::Person {
            id: "person".to_owned(),
        }
    }

    fn a_roster() -> MetricDrilldownEntity {
        MetricDrilldownEntity::Persons {
            ids: vec!["person".to_owned()],
        }
    }

    fn commit_plan() -> EvidencePlan {
        let value = input(MetricInputRole::Value, "commit_count");
        plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![commit_input(MetricInputRole::Value, &value.measure_key)],
        )
    }

    fn ratio_plan() -> EvidencePlan {
        let numerator = input(MetricInputRole::Numerator, "focus_hours");
        let denominator = input(MetricInputRole::Denominator, "work_hours");
        plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 100.0,
                denominator_aggregation: RatioDenominatorAggregation::Sum,
            },
            vec![
                commit_input(MetricInputRole::Numerator, &numerator.measure_key),
                commit_input(MetricInputRole::Denominator, &denominator.measure_key),
            ],
        )
    }

    #[test]
    fn declared_columns_lead_in_declaration_order_and_dimensions_follow() {
        let (columns, rows) = presentation(
            &[row()],
            &commit_plan(),
            &[],
            &["category".to_owned()],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(
            columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            [
                "ref",
                "title",
                "repository",
                "author",
                "lines_added",
                "lines_removed",
                "category",
                "date"
            ]
        );
        assert_eq!(rows[0].values["category"], "code");
        assert_eq!(rows[0].values["lines_added"], 12);
        assert!(
            rows[0].values["lines_added"].is_i64(),
            "line counts are whole lines, not floats: {:?}",
            rows[0].values["lines_added"]
        );
    }

    // The ordering key's SQL is decided by a column's declared TYPE, and no
    // snapshot pins the catalog that declares it. A cursor that survived such
    // a change would compare against a differently-shaped key.
    #[test]
    fn the_cursor_is_bound_to_the_shape_of_the_columns_it_was_issued_over() {
        let request = validated(commit_plan());
        let columns = presentation_columns(
            &request.plan,
            &request.selection.filters,
            &request.selection.display_dimensions,
            &request.selection.entity,
        );
        let mut retyped = columns.clone();
        for column in &mut retyped {
            if column.key == "ref" {
                column.r#type = MetricDrilldownColumnType::Number;
            }
        }

        let before = selection_fingerprint(TEST_TENANT, &request.selection, &columns)
            .unwrap_or_else(|error| panic!("fingerprint must build: {error}"));
        let after = selection_fingerprint(TEST_TENANT, &request.selection, &retyped)
            .unwrap_or_else(|error| panic!("fingerprint must build: {error}"));

        assert_ne!(before, after);
    }

    // Only the read that shows the column pays for resolving names.
    #[test]
    fn a_ratio_roster_presents_no_one_and_so_names_no_one() {
        let mut roster = validated(commit_plan());
        roster.selection.entity = a_roster();
        assert!(presents_person(&roster));

        let mut ratio = validated(ratio_plan());
        ratio.selection.entity = a_roster();
        assert!(!presents_person(&ratio));

        assert!(!presents_person(&validated(commit_plan())));
    }

    // A ratio row is two aggregates over a day; the outer query carries no cell
    // a dimension column could be ordered or searched by, from either source.
    #[test]
    fn a_ratio_presents_no_dimension_column_from_either_source() {
        let filters = vec![MetricDrilldownFilter {
            dimension: "repository".to_owned(),
            values: vec!["org/repo".to_owned()],
        }];
        let columns = presentation_columns(
            &ratio_plan(),
            &filters,
            &["category".to_owned()],
            &one_person(),
        );

        assert_eq!(
            columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            ["date", "numerator", "denominator"]
        );
    }

    #[test]
    fn a_roster_says_who_each_record_belongs_to() {
        let (roster, _) = presentation(
            &[row()],
            &commit_plan(),
            &[],
            &[],
            &a_roster(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        let who = roster
            .first()
            .unwrap_or_else(|| panic!("a roster leads with who"));
        assert_eq!(who.key, "person");
        assert!(!who.sortable, "the column carries an id, not a name");

        let (single, _) = presentation(
            &[row()],
            &commit_plan(),
            &[],
            &[],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert!(
            !single.iter().any(|column| column.key == "person"),
            "one person's own drilldown already knows whose records these are"
        );
    }

    // An id is not an answer to "who", least of all in an exported file.
    #[test]
    fn who_reads_as_a_name_or_as_nothing_at_all() {
        let mut evidence = row();
        evidence.person_id = "person-1".to_owned();
        let roster = PersonNames::from([("person-1".to_owned(), "Ada".to_owned())]);

        let (_, rows) = presentation(
            &[evidence.clone()],
            &commit_plan(),
            &[],
            &[],
            &a_roster(),
            &roster,
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(rows[0].values["person"], "Ada");

        let (_, unnamed) = presentation(
            &[evidence],
            &commit_plan(),
            &[],
            &[],
            &a_roster(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert!(unnamed[0].values["person"].is_null());
    }

    #[test]
    fn a_column_the_query_cannot_order_by_is_shown_but_not_sortable() {
        let (columns, _) = presentation(
            &[row()],
            &commit_plan(),
            &[],
            &[],
            &a_roster(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        for column in &columns {
            assert_eq!(
                column.sortable,
                column.key != "person",
                "every column but who is sortable: {}",
                column.key
            );
        }
    }

    #[test]
    fn declared_labels_and_types_reach_the_reader() {
        let (columns, _) = presentation(
            &[row()],
            &commit_plan(),
            &[],
            &[],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        let lines_added = columns
            .iter()
            .find(|column| column.key == "lines_added")
            .unwrap_or_else(|| panic!("declared column must be served"));
        assert_eq!(lines_added.label, "Lines added");
        assert_eq!(lines_added.r#type, MetricDrilldownColumnType::Number);
    }

    #[test]
    fn a_dimension_a_measure_already_declares_stays_one_column() {
        let (columns, rows) = presentation(
            &[row()],
            &commit_plan(),
            &[],
            &["repository".to_owned(), "category".to_owned()],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(
            columns
                .iter()
                .filter(|column| column.key == "repository")
                .count(),
            1
        );
        assert_eq!(rows[0].values["repository"], "org/repo");
    }

    #[test]
    fn an_undeclared_measure_serves_the_date_and_its_number() {
        let value = input(MetricInputRole::Value, "meeting_hours");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
                measure_key: value.measure_key,
                presentation: EvidencePresentation::undeclared(EvidenceGranularity::SourceSummary),
            }],
        );
        let (columns, _) = presentation(
            &[row()],
            &plan,
            &[],
            &[],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(
            columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            ["date", "value"]
        );
    }

    #[test]
    fn a_metric_reading_two_measures_of_one_shape_shows_that_shape_once() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![
                commit_input(MetricInputRole::Value, &value.measure_key),
                commit_input(MetricInputRole::Value, "commit_change_size"),
            ],
        );
        let (columns, _) = presentation(
            &[row()],
            &plan,
            &[],
            &[],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(
            columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            [
                "ref",
                "title",
                "repository",
                "author",
                "lines_added",
                "lines_removed",
                "date"
            ]
        );
    }

    #[test]
    fn a_measure_declaring_a_value_column_gets_one() {
        let value = input(MetricInputRole::Value, "pr_cycle_hours");
        let mut presentation_spec = commit_presentation();
        presentation_spec.show_value = true;
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
                measure_key: value.measure_key,
                presentation: presentation_spec,
            }],
        );
        let (columns, rows) = presentation(
            &[row()],
            &plan,
            &[],
            &[],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("presentation must build: {error}"));
        assert_eq!(
            columns
                .last()
                .unwrap_or_else(|| panic!("columns must not be empty"))
                .key,
            "value"
        );
        assert_eq!(rows[0].values["value"], 1.0);
    }

    #[test]
    fn number_columns_coerce_strings_integers_first_then_floats() {
        use serde_json::json;

        let number = MetricDrilldownColumnType::Number;
        assert_eq!(coerce_to_column_type(number, json!("12")), json!(12));
        assert!(
            coerce_to_column_type(number, json!("12")).is_i64(),
            "whole counts are integers, not 12.0"
        );
        assert_eq!(coerce_to_column_type(number, json!("-3")), json!(-3));
        assert_eq!(coerce_to_column_type(number, json!("2.5")), json!(2.5));
        assert_eq!(
            coerce_to_column_type(number, json!("many")),
            json!("many"),
            "unparseable text stays verbatim"
        );
        assert_eq!(
            coerce_to_column_type(number, json!(7)),
            json!(7),
            "already-numeric values pass through"
        );

        for r#type in [
            MetricDrilldownColumnType::String,
            MetricDrilldownColumnType::Date,
        ] {
            assert_eq!(
                coerce_to_column_type(r#type, json!("12")),
                json!("12"),
                "only Number columns coerce: {type:?}"
            );
        }
    }

    #[test]
    fn ratio_presentation_names_numerator_and_denominator() {
        let numerator = input(MetricInputRole::Numerator, "focus_hours");
        let denominator = input(MetricInputRole::Denominator, "work_hours");
        let plan = plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 100.0,
                denominator_aggregation: RatioDenominatorAggregation::Sum,
            },
            vec![
                commit_input(MetricInputRole::Numerator, &numerator.measure_key),
                commit_input(MetricInputRole::Denominator, &denominator.measure_key),
            ],
        );
        let mut ratio_row = row();
        ratio_row.numerator = Some(6.0);
        ratio_row.denominator = Some(8.0);
        ratio_row.details = serde_json::json!({});
        let (columns, rows) = presentation(
            &[ratio_row],
            &plan,
            &[],
            &[],
            &one_person(),
            &PersonNames::new(),
        )
        .unwrap_or_else(|error| panic!("ratio presentation must build: {error}"));
        assert_eq!(
            columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            ["date", "numerator", "denominator"],
            "the records behind the two measures are different things"
        );
        assert_eq!(columns[1].label, "Focus hours");
        assert_eq!(columns[2].label, "Work hours");
        assert_eq!(rows[0].values["numerator"], 6.0);
        assert_eq!(rows[0].values["denominator"], 8.0);
    }

    #[test]
    fn response_pages_with_snapshot_bound_cursor() {
        let request = validated(commit_plan());
        let response = build_response(
            &request,
            vec![row(), row()],
            &PersonNames::new(),
            &ExternalSourceRegistry::default(),
        )
        .unwrap_or_else(|error| panic!("response must build: {error}"));
        let cursor = response
            .next_cursor
            .unwrap_or_else(|| panic!("response must include a next cursor"));
        let envelope =
            decode_cursor(&cursor).unwrap_or_else(|error| panic!("cursor must decode: {error}"));
        assert_eq!(response.rows.len(), 1);
        assert_eq!(envelope.snapshot_id, "snapshot");
        assert_eq!(envelope.fingerprint, request.fingerprint);
        assert!(decode_cursor("invalid").is_err());
    }

    #[test]
    fn response_links_only_visible_columns_from_configured_source() -> anyhow::Result<()> {
        let request = validated(commit_plan());
        let mut evidence = row();
        evidence.dimensions_json = r#"[
            {"key":"source","value":"github","label":"GitHub"},
            {"key":"repository","value":"source-a:group/repository","label":"group/repository"}
        ]"#
        .to_owned();
        evidence.details["source_id"] = serde_json::json!("source-a");
        let registry = ExternalSourceRegistry::new(&[ExternalSourceConfig {
            id: "source-a".to_owned(),
            provider: ExternalSourceProvider::Github,
            web_base_url: "https://code.example.test".to_owned(),
        }])?;

        let response = build_response(&request, vec![evidence], &PersonNames::new(), &registry)?;

        assert_eq!(
            response.rows[0].links.get("repository").map(String::as_str),
            Some("https://code.example.test/org/repo")
        );
        assert_eq!(
            response.rows[0].links.get("ref").map(String::as_str),
            Some("https://code.example.test/org/repo/commit/abc123")
        );
        assert_eq!(
            response.rows[0].links.get("title").map(String::as_str),
            Some("https://code.example.test/org/repo/commit/abc123")
        );
        assert!(!response.rows[0].values.contains_key("source_id"));
        Ok(())
    }

    #[test]
    fn response_does_not_link_empty_record_cells() -> anyhow::Result<()> {
        let request = validated(commit_plan());
        let mut evidence = row();
        evidence.dimensions_json = r#"[
            {"key":"source","value":"github","label":"GitHub"},
            {"key":"repository","value":"source-a:group/repository","label":"group/repository"}
        ]"#
        .to_owned();
        evidence.details["source_id"] = serde_json::json!("source-a");
        evidence.details["title"] = serde_json::json!("");
        let registry = ExternalSourceRegistry::new(&[ExternalSourceConfig {
            id: "source-a".to_owned(),
            provider: ExternalSourceProvider::Github,
            web_base_url: "https://code.example.test".to_owned(),
        }])?;

        let response = build_response(&request, vec![evidence], &PersonNames::new(), &registry)?;

        assert!(response.rows[0].links.contains_key("ref"));
        assert!(!response.rows[0].links.contains_key("title"));
        Ok(())
    }

    #[test]
    fn presentation_rejects_invalid_warehouse_json() {
        let plan = commit_plan();
        let mut invalid_details = row();
        invalid_details.details = serde_json::json!("invalid");
        assert!(
            presentation(
                &[invalid_details],
                &plan,
                &[],
                &[],
                &one_person(),
                &PersonNames::new()
            )
            .is_err()
        );
        let mut invalid_dimensions = row();
        invalid_dimensions.dimensions_json = "invalid".to_owned();
        assert!(
            presentation(
                &[invalid_dimensions],
                &plan,
                &[],
                &[],
                &one_person(),
                &PersonNames::new()
            )
            .is_err()
        );
    }
}
