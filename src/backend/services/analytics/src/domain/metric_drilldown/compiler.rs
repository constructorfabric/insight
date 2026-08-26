use std::fmt::Write;

use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_definitions::ComputationSpec;
use crate::domain::metric_definitions::definition::{MetricInputRole, RatioDenominatorAggregation};

use super::cursor::CursorKey;
use super::dto::{
    EvidenceInput, EvidenceQueryRow, MetricDrilldownFilter, ValidatedMetricDrilldown,
};
use super::error::config_error;

/// Person evidence uses the canonical person id resolved by the gold build;
/// tenant evidence repeats its tenant key as the entity id.
pub fn compile_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
    if matches!(req.plan.definition.spec, ComputationSpec::Ratio { .. }) {
        return compile_ratio_query(req);
    }
    Ok(compile_value_query(req))
}

fn compile_value_query(req: &ValidatedMetricDrilldown) -> (String, Vec<String>) {
    let (database, table) = req.plan.relation.table_ref();
    let mut params = Vec::new();
    let measures = req
        .plan
        .inputs
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let role_expr = role_expression(&req.plan.inputs);
    for input in &req.plan.inputs {
        params.push(input.measure_key.clone());
        params.push(input.role.as_db().to_owned());
    }
    params.extend([
        req.tenant_id.to_string(),
        req.plan.source_key.clone(),
        req.selection.entity.entity_type().to_owned(),
    ]);
    params.extend(req.selection.entity.person_ids().iter().cloned());
    params.extend([req.from.to_string(), req.to.to_string()]);
    params.extend(
        req.plan
            .inputs
            .iter()
            .map(|input| input.measure_key.clone()),
    );
    let filter_sql = filter_predicate(&req.selection.filters, &mut params);
    let cursor_sql = cursor_predicate(
        req.cursor.as_ref(),
        "AND",
        "role, toString(evidence.metric_date), ifNull(toString(evidence.observed_at), ''), evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, ifNull(evidence.subject_key, '')",
        &mut params,
    );
    let limit = req.limit + 1;
    let tenant =
        crate::domain::metric_results::compiler::tenant_predicate(req.enforce_tenant_scope);
    let sql = format!(
        "WITH {role_expr} AS role \
         SELECT role, toString(evidence.metric_date) AS metric_date, ifNull(toString(evidence.observed_at), '') AS observed_at, \
                evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, \
                evidence.contribution, CAST(NULL AS Nullable(Float64)) AS numerator, \
                CAST(NULL AS Nullable(Float64)) AS denominator, \
                ifNull(evidence.subject_key, '') AS subject_key, \
                toJSONString(evidence.dimensions) AS dimensions_json, evidence.details \
         FROM {database}.{table} AS evidence \
         WHERE {tenant} AND evidence.source_key = ? AND evidence.entity_type = ? AND {entity} \
           AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
           AND evidence.measure_key IN ({measures}){filter_sql}{cursor_sql} \
         ORDER BY role, metric_date, ifNull(toString(observed_at), ''), source_key, measure_key, record_id, record_kind, ifNull(subject_key, '') \
         LIMIT {limit}",
        entity = entity_predicate(&req.selection.entity),
    );
    (sql, params)
}

fn compile_ratio_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
    let (database, table) = req.plan.relation.table_ref();
    let numerator = req
        .plan
        .inputs
        .iter()
        .find(|input| input.role == MetricInputRole::Numerator)
        .ok_or_else(config_error)?;
    let denominator = req
        .plan
        .inputs
        .iter()
        .find(|input| input.role == MetricInputRole::Denominator)
        .ok_or_else(config_error)?;
    let ComputationSpec::Ratio {
        denominator_aggregation,
        ..
    } = &req.plan.definition.spec
    else {
        return Err(config_error());
    };
    let denominator_expr = match denominator_aggregation {
        RatioDenominatorAggregation::Sum => {
            "sumIf(ifNull(evidence.contribution, 0), evidence.measure_key = ?)"
        }
        RatioDenominatorAggregation::DistinctCount => {
            "toFloat64(uniqExactIf(evidence.subject_key, evidence.measure_key = ? AND evidence.subject_key IS NOT NULL))"
        }
    };
    let mut params = vec![
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
        req.tenant_id.to_string(),
        req.plan.source_key.clone(),
        req.selection.entity.entity_type().to_owned(),
    ];
    params.extend(req.selection.entity.person_ids().iter().cloned());
    params.extend([
        req.from.to_string(),
        req.to.to_string(),
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
    ]);
    let filter_sql = filter_predicate(&req.selection.filters, &mut params);
    let cursor_sql = cursor_predicate(
        req.cursor.as_ref(),
        "WHERE",
        "role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key",
        &mut params,
    );
    let limit = req.limit + 1;
    let tenant =
        crate::domain::metric_results::compiler::tenant_predicate(req.enforce_tenant_scope);
    let sql = format!(
        "SELECT * FROM (\
            SELECT 'value' AS role, toString(evidence.metric_date) AS metric_date, \
                   '' AS observed_at, \
                   any(evidence.source_key) AS source_key, '' AS measure_key, \
                   toString(evidence.metric_date) AS record_id, 'daily_ratio' AS record_kind, \
                   CAST(NULL AS Nullable(Float64)) AS contribution, \
                   sumIf(ifNull(evidence.contribution, 0), evidence.measure_key = ?) AS numerator, \
                   {denominator_expr} AS denominator, \
                   '' AS subject_key, any(toJSONString(evidence.dimensions)) AS dimensions_json, \
                   CAST(map() AS Map(String, String)) AS details \
            FROM {database}.{table} AS evidence \
            WHERE {tenant} AND evidence.source_key = ? AND evidence.entity_type = ? AND {entity} \
              AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
              AND evidence.measure_key IN (?, ?){filter_sql} \
            GROUP BY evidence.metric_date\
         ){cursor_sql} \
         ORDER BY role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key \
         LIMIT {limit}",
        entity = entity_predicate(&req.selection.entity),
    );
    Ok((sql, params))
}

fn entity_predicate(entity: &super::dto::MetricDrilldownEntity) -> String {
    match entity {
        super::dto::MetricDrilldownEntity::Person { .. } => "evidence.entity_id = ?".to_owned(),
        // One placeholder per person, bound in the same order the params are
        // pushed — an id is never interpolated into the SQL.
        super::dto::MetricDrilldownEntity::Persons { ids } if !ids.is_empty() => {
            format!(
                "evidence.entity_id IN ({})",
                vec!["?"; ids.len()].join(", ")
            )
        }
        super::dto::MetricDrilldownEntity::Tenant {} => {
            "evidence.entity_id = evidence.tenant_id".to_owned()
        }
        // An empty roster is rejected in validation, so it cannot arrive
        // through the API; matching no row beats emitting `IN ()`, which is a
        // syntax error rather than an empty result.
        super::dto::MetricDrilldownEntity::Persons { .. }
        | super::dto::MetricDrilldownEntity::Unknown => "1 = 0".to_owned(),
    }
}

fn filter_predicate(filters: &[MetricDrilldownFilter], params: &mut Vec<String>) -> String {
    let mut sql = String::new();
    for filter in filters {
        let placeholders = vec!["?"; filter.values.len()].join(", ");
        let _ = write!(
            sql,
            " AND indexOf(evidence.dimensions.1, ?) > 0 AND evidence.dimensions.2[indexOf(evidence.dimensions.1, ?)] IN ({placeholders})"
        );
        params.push(filter.dimension.clone());
        params.push(filter.dimension.clone());
        params.extend(filter.values.iter().cloned());
    }
    sql
}

fn cursor_predicate(
    cursor: Option<&CursorKey>,
    keyword: &str,
    key_tuple: &str,
    params: &mut Vec<String>,
) -> String {
    let Some(cursor) = cursor else {
        return String::new();
    };
    params.extend([
        cursor.role.clone(),
        cursor.metric_date.clone(),
        cursor.observed_at.clone(),
        cursor.source_key.clone(),
        cursor.measure_key.clone(),
        cursor.record_id.clone(),
        cursor.record_kind.clone(),
        cursor.subject_key.clone(),
    ]);
    format!(" {keyword} tuple({key_tuple}) > tuple(?, ?, ?, ?, ?, ?, ?, ?)")
}

fn role_expression(inputs: &[EvidenceInput]) -> String {
    let branches = inputs
        .iter()
        .map(|_| "evidence.measure_key = ?, ?")
        .collect::<Vec<_>>()
        .join(", ");
    format!("multiIf({branches}, 'value')")
}

pub fn decode_evidence_rows(bytes: &[u8]) -> Result<Vec<EvidenceQueryRow>, serde_json::Error> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(serde_json::from_slice)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_definitions::{EvidenceGranularity, RatioDenominatorAggregation};
    use crate::domain::metric_drilldown::dto::EvidencePresentation;
    use crate::domain::metric_drilldown::presentation::evidence_presentation;
    use crate::domain::metric_drilldown::test_support::{
        TEST_PERSON, TEST_TENANT, input, plan, validated,
    };
    use uuid::Uuid;

    #[test]
    fn value_query_binds_filters_and_cursor() {
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
        let mut request = validated(plan);
        request.cursor = Some(CursorKey {
            role: "value".to_owned(),
            metric_date: "2026-07-01".to_owned(),
            observed_at: String::new(),
            source_key: "git".to_owned(),
            measure_key: "commit_count".to_owned(),
            record_id: "abc".to_owned(),
            record_kind: "commit".to_owned(),
            subject_key: String::new(),
        });
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));
        assert!(sql.contains("insight.git_metric_evidence"));
        assert!(sql.contains("indexOf(evidence.dimensions.1, ?)"));
        assert!(sql.contains("LIMIT 2"));
        assert_eq!(
            sql.matches('?').count(),
            params.len(),
            "every placeholder needs exactly one bound param"
        );
        assert_eq!(
            params,
            [
                "commit_count", // role expression branch
                "value",
                &TEST_TENANT.to_string(), // tenant predicate leads the scope
                "git",                    // scope: source, entity type, entity id
                "person",
                &TEST_PERSON.to_string(), // canonical entity_id, resolved at build time
                "2026-07-01",             // period bounds
                "2026-07-31",
                "commit_count", // measure_key IN
                "repository",   // filter: indexOf twice, then values
                "repository",
                "org/repo",
                "value", // cursor tuple, complete ordering key
                "2026-07-01",
                "",
                "git",
                "commit_count",
                "abc",
                "commit",
                "",
            ]
        );
    }

    #[test]
    fn tenant_query_matches_the_entity_to_its_storage_partition() {
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
        let mut request = validated(plan);
        request.selection.entity = super::super::dto::MetricDrilldownEntity::Tenant {};

        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("evidence.entity_id = evidence.tenant_id"));
        assert!(!params.iter().any(|value| value == "default"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn roster_query_binds_one_placeholder_per_person() {
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
        let second = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_0002);
        let mut request = validated(plan);
        request.selection.entity = super::super::dto::MetricDrilldownEntity::Persons {
            ids: vec![TEST_PERSON.to_string(), second.to_string()],
        };

        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("evidence.entity_id IN (?, ?)"));
        // The roster reads as one query over people, not as a tenant total:
        // the entity type stays `person`, which is the partition the evidence
        // rows are keyed by.
        assert!(params.iter().any(|value| value == "person"));
        assert!(params.iter().any(|value| *value == TEST_PERSON.to_string()));
        assert!(params.iter().any(|value| *value == second.to_string()));
        assert_eq!(
            sql.matches('?').count(),
            params.len(),
            "every placeholder needs exactly one bound param"
        );
    }

    #[test]
    fn ratio_query_uses_named_inputs() {
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
        let request = validated(plan);
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));
        assert!(sql.contains("sumIf"));
        assert!(sql.contains("daily_ratio"));
        assert_eq!(
            sql.matches('?').count(),
            params.len(),
            "every placeholder needs exactly one bound param"
        );
        assert_eq!(
            params,
            [
                "focus_hours", // sumIf numerator, then denominator
                "work_hours",
                &TEST_TENANT.to_string(), // tenant predicate leads the scope
                "git",                    // scope: source, entity type, entity id
                "person",
                &TEST_PERSON.to_string(), // canonical entity_id, resolved at build time
                "2026-07-01",             // period bounds
                "2026-07-31",
                "focus_hours", // measure_key IN
                "work_hours",
                "repository", // filter: indexOf twice, then values
                "repository",
                "org/repo",
            ]
        );
    }

    #[test]
    fn ratio_query_uses_distinct_denominator_aggregation() {
        let numerator = input(MetricInputRole::Numerator, "commit_count");
        let denominator = input(MetricInputRole::Denominator, "commit_day");
        let plan = plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 1.0,
                denominator_aggregation: RatioDenominatorAggregation::DistinctCount,
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
        let request = validated(plan);

        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("uniqExactIf(evidence.subject_key"));
        assert_eq!(sql.matches('?').count(), params.len());
    }
}
