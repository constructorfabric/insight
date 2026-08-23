use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_definitions::ComputationSpec;
use crate::domain::metric_definitions::EvidenceGranularity;
use crate::domain::metric_definitions::definition::MetricInputRole;

use super::cursor::encode_cursor;
use super::dto::{
    EvidencePlan, EvidencePresentation, EvidenceQueryRow, MetricDrilldownColumn,
    MetricDrilldownColumnType, MetricDrilldownFilter, MetricDrilldownResponse, MetricDrilldownRow,
    ValidatedMetricDrilldown,
};

#[derive(Debug, Deserialize)]
struct EvidenceDimension {
    key: String,
    value: String,
    label: Option<String>,
}

use super::error::config_error;

pub fn build_response(
    req: &ValidatedMetricDrilldown,
    mut rows: Vec<EvidenceQueryRow>,
) -> Result<MetricDrilldownResponse, CanonicalError> {
    let next_cursor = if rows.len() > req.limit {
        rows.truncate(req.limit);
        rows.last()
            .map(|row| encode_cursor(&req.fingerprint, &req.snapshot_id, row))
            .transpose()?
    } else {
        None
    };
    let (columns, rows) = presentation(
        &rows,
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
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
) -> Result<(Vec<MetricDrilldownColumn>, Vec<MetricDrilldownRow>), CanonicalError> {
    let details = rows
        .iter()
        .map(|row| row.details.as_object().ok_or_else(config_error))
        .collect::<Result<Vec<_>, _>>()?;
    let ratio = matches!(plan.definition.spec, ComputationSpec::Ratio { .. });
    let display_dimensions = if ratio { &[] } else { display_dimensions };
    let dimensions = presentation_dimensions(rows)?;
    let mut detail_keys = if ratio {
        BTreeSet::new()
    } else {
        plan.inputs
            .iter()
            .flat_map(|input| input.presentation.detail_keys)
            .map(|key| (*key).to_owned())
            .collect::<BTreeSet<_>>()
    };
    let dimension_keys = filters
        .iter()
        .filter(|filter| filter.values.len() == 1)
        .map(|filter| filter.dimension.clone())
        .chain(display_dimensions.iter().cloned())
        .collect::<BTreeSet<_>>();
    detail_keys.extend(dimension_keys);
    let include_value = !ratio
        && plan
            .inputs
            .iter()
            .any(|input| input.presentation.show_value);
    let mut ordered_keys = Vec::new();
    if detail_keys.remove("ref") {
        ordered_keys.push("ref".to_owned());
    }
    if detail_keys.remove("title") {
        ordered_keys.push("title".to_owned());
    }
    for key in ["repository", "author"] {
        if detail_keys.remove(key) {
            ordered_keys.push(key.to_owned());
        }
    }
    ordered_keys.extend(detail_keys);
    ordered_keys.push("date".to_owned());
    if ratio {
        ordered_keys.push("numerator".to_owned());
        ordered_keys.push("denominator".to_owned());
    } else if include_value {
        ordered_keys.push("value".to_owned());
    }

    let columns: Vec<MetricDrilldownColumn> = ordered_keys
        .iter()
        .map(|key| presentation_column(key, plan))
        .collect();
    let column_types: BTreeMap<&str, MetricDrilldownColumnType> = columns
        .iter()
        .map(|column| (column.key.as_str(), column.r#type))
        .collect();
    let projected_rows = rows
        .iter()
        .zip(details)
        .zip(dimensions)
        .map(|((row, details), dimensions)| {
            project_row(row, details, &dimensions, &ordered_keys, &column_types)
        })
        .collect::<Result<Vec<_>, CanonicalError>>()?;

    Ok((columns, projected_rows))
}

fn project_row(
    row: &EvidenceQueryRow,
    details: &serde_json::Map<String, serde_json::Value>,
    dimensions: &[EvidenceDimension],
    ordered_keys: &[String],
    column_types: &BTreeMap<&str, MetricDrilldownColumnType>,
) -> Result<MetricDrilldownRow, CanonicalError> {
    let mut values = BTreeMap::new();
    for key in ordered_keys {
        let value = match key.as_str() {
            "date" => row.metric_date.clone().into(),
            "value" => serde_json::to_value(row.contribution).map_err(|_| config_error())?,
            "numerator" => serde_json::to_value(row.numerator).map_err(|_| config_error())?,
            "denominator" => serde_json::to_value(row.denominator).map_err(|_| config_error())?,
            _ => details
                .get(key)
                .filter(|value| visible_value(value))
                .cloned()
                .or_else(|| {
                    dimensions
                        .iter()
                        .find(|dimension| dimension.key == *key)
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

        let r#type = column_types
            .get(key.as_str())
            .copied()
            .unwrap_or(MetricDrilldownColumnType::String);
        values.insert(key.clone(), coerce_to_column_type(r#type, value));
    }
    Ok(MetricDrilldownRow { values })
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

fn presentation_column(key: &str, plan: &EvidencePlan) -> MetricDrilldownColumn {
    let (label, r#type) = match key {
        "ref" => ("Ref".to_owned(), MetricDrilldownColumnType::String),
        "title" => ("Title".to_owned(), MetricDrilldownColumnType::String),
        "repository" => ("Repository".to_owned(), MetricDrilldownColumnType::String),
        "author" => ("Author".to_owned(), MetricDrilldownColumnType::String),
        "date" => ("Date".to_owned(), MetricDrilldownColumnType::Date),
        "value" => ("Value".to_owned(), MetricDrilldownColumnType::Number),
        "numerator" => (
            input_label(plan, MetricInputRole::Numerator),
            MetricDrilldownColumnType::Number,
        ),
        "denominator" => (
            input_label(plan, MetricInputRole::Denominator),
            MetricDrilldownColumnType::Number,
        ),
        "lines_added" => ("Lines added".to_owned(), MetricDrilldownColumnType::Number),
        "lines_removed" => (
            "Lines removed".to_owned(),
            MetricDrilldownColumnType::Number,
        ),
        "billing_month" => (
            "Billing month".to_owned(),
            MetricDrilldownColumnType::String,
        ),
        "ceiling_usd" => ("Ceiling".to_owned(), MetricDrilldownColumnType::Number),
        _ => (humanize_field_name(key), MetricDrilldownColumnType::String),
    };
    MetricDrilldownColumn {
        key: key.to_owned(),
        label,
        r#type,
    }
}

pub(super) fn evidence_presentation(
    source_key: &str,
    measure_key: &str,
    granularity: EvidenceGranularity,
) -> EvidencePresentation {
    match (source_key, measure_key) {
        ("git", "commit_count" | "commit_change_size") => EvidencePresentation {
            detail_keys: &[
                "ref",
                "title",
                "repository",
                "author",
                "lines_added",
                "lines_removed",
            ],
            show_value: false,
        },
        ("git", "pr_created" | "pr_created_merged" | "pr_merged") => EvidencePresentation {
            detail_keys: &["ref", "title", "repository", "author"],
            show_value: false,
        },
        (
            "git",
            "pr_cycle_hours"
            | "pr_change_size"
            | "pr_first_review_hours"
            | "pr_review_wait_share"
            | "pr_review_to_merge_hours"
            | "pr_approval_to_merge_hours",
        ) => EvidencePresentation {
            detail_keys: &["ref", "title", "repository", "author"],
            show_value: true,
        },
        // A counting measure needs no value column — the row IS the one it
        // counted; a duration or a page count is only readable with its number.
        (
            "task",
            "tasks_closed" | "bugs_fixed" | "closed_non_bug" | "due_date_on_time"
            | "due_date_with_due" | "late_count",
        )
        | ("wiki", "pages_created") => EvidencePresentation {
            detail_keys: &["ref", "title"],
            show_value: false,
        },
        ("task", _) if granularity == EvidenceGranularity::Event => EvidencePresentation {
            detail_keys: &["ref", "title"],
            show_value: true,
        },
        // A seat-month row is dated at the day its snapshot was last read, not
        // at the month it bills for, so the month has to be a column of its own
        // or the reader cannot tell which month the row is. The ceiling is what
        // the amount is judged against; blank means none was set, which is why
        // the ratio metric withholds a value for that seat.
        ("ai_cost", _) => EvidencePresentation {
            detail_keys: &["billing_month", "ceiling_usd"],
            show_value: true,
        },
        _ => EvidencePresentation {
            detail_keys: &[],
            show_value: granularity != EvidenceGranularity::Event,
        },
    }
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

    use crate::domain::metric_definitions::RatioDenominatorAggregation;
    use crate::domain::metric_drilldown::cursor::decode_cursor;
    use crate::domain::metric_drilldown::dto::EvidenceInput;
    use crate::domain::metric_drilldown::test_support::{input, plan, row, validated};

    #[test]
    fn event_presentation_projects_human_fields_and_dimensions() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let (columns, rows) = presentation(&[row()], &plan, &[], &["category".to_owned()])
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
                "category",
                "lines_added",
                "lines_removed",
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
                EvidenceInput {
                    role: MetricInputRole::Numerator,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    measure_key: denominator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
            ],
        );
        let mut ratio_row = row();
        ratio_row.numerator = Some(6.0);
        ratio_row.denominator = Some(8.0);
        ratio_row.details = serde_json::json!({});
        let (columns, rows) = presentation(&[ratio_row], &plan, &[], &[])
            .unwrap_or_else(|error| panic!("ratio presentation must build: {error}"));
        assert_eq!(columns[1].label, "Focus hours");
        assert_eq!(columns[2].label, "Work hours");
        assert_eq!(rows[0].values["numerator"], 6.0);
        assert_eq!(rows[0].values["denominator"], 8.0);
    }

    #[test]
    fn response_pages_with_snapshot_bound_cursor() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let request = validated(plan);
        let response = build_response(&request, vec![row(), row()])
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
    fn presentation_rejects_invalid_warehouse_json() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                measure_key: value.measure_key,
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        );
        let mut invalid_details = row();
        invalid_details.details = serde_json::json!("invalid");
        assert!(presentation(&[invalid_details], &plan, &[], &[]).is_err());
        let mut invalid_dimensions = row();
        invalid_dimensions.dimensions_json = "invalid".to_owned();
        assert!(presentation(&[invalid_dimensions], &plan, &[], &[]).is_err());
    }

    #[test]
    fn evidence_presentations_cover_domain_shapes() {
        assert!(!evidence_presentation("git", "pr_merged", EvidenceGranularity::Event).show_value);
        assert!(
            evidence_presentation(
                "task",
                "average_slip",
                EvidenceGranularity::DerivedPopulation
            )
            .detail_keys
            .is_empty()
        );
        assert!(evidence_presentation("task", "custom", EvidenceGranularity::Event).show_value);
        assert!(
            !evidence_presentation("wiki", "pages_created", EvidenceGranularity::Event).show_value
        );
        assert!(
            evidence_presentation("collab", "messages", EvidenceGranularity::SourceSummary)
                .show_value
        );
    }

    #[test]
    fn git_pull_request_values_keep_their_numeric_column() {
        for measure_key in [
            "pr_cycle_hours",
            "pr_change_size",
            "pr_first_review_hours",
            "pr_review_wait_share",
            "pr_review_to_merge_hours",
            "pr_approval_to_merge_hours",
        ] {
            let presentation =
                evidence_presentation("git", measure_key, EvidenceGranularity::Event);

            assert!(presentation.show_value, "{measure_key}");
            assert_eq!(
                presentation.detail_keys,
                &["ref", "title", "repository", "author"],
                "{measure_key}"
            );
        }
    }

    #[test]
    fn a_seat_month_row_names_its_billing_month_and_ceiling() {
        let presentation = evidence_presentation(
            "ai_cost",
            "extra_usage_usd",
            EvidenceGranularity::SourceSummary,
        );
        assert_eq!(presentation.detail_keys, &["billing_month", "ceiling_usd"]);
        assert!(presentation.show_value);
    }
}
