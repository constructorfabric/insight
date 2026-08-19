use std::fmt::Write;

use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_definitions::ComputationSpec;
use crate::domain::metric_definitions::definition::MetricInputRole;

use super::cursor::CursorKey;
use super::dto::{
    EvidenceInput, EvidenceQueryRow, MetricDrilldownFilter, ValidatedMetricDrilldown,
};
use super::error::config_error;

/// Evidence is keyed by the source identity, so a person's rows are the rows of
/// every identity the live map resolves to them.
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
        req.selection.entity.r#type.clone(),
        req.selection.entity.id.clone(),
        req.from.to_string(),
        req.to.to_string(),
    ]);
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
        "role, toString(evidence.metric_date), ifNull(toString(evidence.observed_at), ''), evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, ifNull(evidence.subject_key, ''), evidence.entity_id",
        &mut params,
    );
    let limit = req.limit + 1;
    let tenant =
        crate::domain::metric_results::compiler::tenant_predicate(req.enforce_tenant_scope);
    let sql = format!(
        "WITH {role_expr} AS role \
         SELECT role, evidence.entity_id AS entity_id, toString(evidence.metric_date) AS metric_date, ifNull(toString(evidence.observed_at), '') AS observed_at, \
                evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, \
                evidence.contribution, CAST(NULL AS Nullable(Float64)) AS numerator, \
                CAST(NULL AS Nullable(Float64)) AS denominator, \
                ifNull(evidence.subject_key, '') AS subject_key, \
                toJSONString(evidence.dimensions) AS dimensions_json, evidence.details \
         FROM {database}.{table} AS evidence \
         WHERE {tenant} AND evidence.source_key = ? AND evidence.entity_type = ? \
           AND evidence.entity_id IN (SELECT email FROM identity.person_map WHERE person_id = ?) \
           AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
           AND evidence.measure_key IN ({measures}){filter_sql}{cursor_sql} \
         ORDER BY role, metric_date, ifNull(toString(observed_at), ''), source_key, measure_key, record_id, record_kind, ifNull(subject_key, ''), entity_id \
         LIMIT {limit}"
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
    let mut params = vec![
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
        req.tenant_id.to_string(),
        req.plan.source_key.clone(),
        req.selection.entity.r#type.clone(),
        req.selection.entity.id.clone(),
        req.from.to_string(),
        req.to.to_string(),
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
    ];
    let filter_sql = filter_predicate(&req.selection.filters, &mut params);
    let cursor_sql = cursor_predicate(
        req.cursor.as_ref(),
        "WHERE",
        "role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key, entity_id",
        &mut params,
    );
    let limit = req.limit + 1;
    let tenant =
        crate::domain::metric_results::compiler::tenant_predicate(req.enforce_tenant_scope);
    // INVARIANT: a flagged measure collapses identities before the daily rollup
    // sums it, or the drilldown explains a ratio the tile never showed.
    let collapsed = collapsed_evidence_value(numerator, denominator);
    let sql = format!(
        "SELECT * FROM (\
            SELECT 'value' AS role, '' AS entity_id, toString(collapsed.metric_date) AS metric_date, \
                   '' AS observed_at, \
                   any(collapsed.source_key) AS source_key, '' AS measure_key, \
                   toString(collapsed.metric_date) AS record_id, 'daily_ratio' AS record_kind, \
                   CAST(NULL AS Nullable(Float64)) AS contribution, \
                   sumIf(collapsed.contribution, collapsed.measure_key = ?) AS numerator, \
                   sumIf(collapsed.contribution, collapsed.measure_key = ?) AS denominator, \
                   '' AS subject_key, any(collapsed.dimensions_json) AS dimensions_json, \
                   CAST(map() AS Map(String, String)) AS details \
            FROM (\
                SELECT evidence.metric_date AS metric_date, \
                       any(evidence.source_key) AS source_key, \
                       evidence.measure_key AS measure_key, \
                       toJSONString(evidence.dimensions) AS dimensions_json, \
                       {collapsed} AS contribution \
                FROM {database}.{table} AS evidence \
                WHERE {tenant} AND evidence.source_key = ? AND evidence.entity_type = ? \
                  AND evidence.entity_id IN (SELECT email FROM identity.person_map WHERE person_id = ?) \
                  AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
                  AND evidence.measure_key IN (?, ?){filter_sql} \
                GROUP BY evidence.metric_date, evidence.measure_key, \
                         toJSONString(evidence.dimensions), ifNull(evidence.subject_key, '')\
            ) AS collapsed \
            GROUP BY collapsed.metric_date\
         ){cursor_sql} \
         ORDER BY role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key, entity_id \
         LIMIT {limit}"
    );
    Ok((sql, params))
}

/// Per-identity combination for the two halves of a ratio, at evidence grain.
/// `sum` needs no special case: summing identities then days is the same sum.
fn collapsed_evidence_value(numerator: &EvidenceInput, denominator: &EvidenceInput) -> String {
    let mut arms = String::new();
    for input in [numerator, denominator] {
        if !input.alias_collapse.needs_pre_collapse() {
            continue;
        }
        let aggregate = input.alias_collapse.aggregate_fn();
        let measure =
            crate::domain::metric_results::compiler::sql_string_literal(&input.measure_key);
        let _ = write!(
            arms,
            "evidence.measure_key = {measure}, {aggregate}(ifNull(evidence.contribution, 0)), "
        );
    }
    if arms.is_empty() {
        return "sum(ifNull(evidence.contribution, 0))".to_owned();
    }
    format!("multiIf({arms}sum(ifNull(evidence.contribution, 0)))")
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
    // INVARIANT: bound in the order `key_tuple` names the columns.
    params.extend([
        cursor.role.clone(),
        cursor.metric_date.clone(),
        cursor.observed_at.clone(),
        cursor.source_key.clone(),
        cursor.measure_key.clone(),
        cursor.record_id.clone(),
        cursor.record_kind.clone(),
        cursor.subject_key.clone(),
        cursor.entity_id.clone(),
    ]);
    format!(" {keyword} tuple({key_tuple}) > tuple(?, ?, ?, ?, ?, ?, ?, ?, ?)")
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
    use crate::domain::metric_definitions::EvidenceGranularity;
    use crate::domain::metric_definitions::definition::AliasCollapse;
    use crate::domain::metric_drilldown::dto::EvidencePresentation;
    use crate::domain::metric_drilldown::presentation::evidence_presentation;
    use crate::domain::metric_drilldown::test_support::{
        TEST_PERSON, TEST_TENANT, input, plan, validated,
    };

    #[test]
    fn value_query_binds_filters_and_cursor() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
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
            entity_id: "person@example.com".to_owned(),
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
                &TEST_PERSON.to_string(), // the person asked about; the map turns it into their identities
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
                "person@example.com", // source identity closes the ordering key
            ]
        );
        assert_eq!(sql.matches('?').count(), params.len());
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
            },
            vec![
                EvidenceInput {
                    role: MetricInputRole::Numerator,
                    alias_collapse: AliasCollapse::Sum,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    alias_collapse: AliasCollapse::Sum,
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
                &TEST_PERSON.to_string(), // the person asked about; the map turns it into their identities
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
    fn a_drilldown_scopes_to_every_identity_the_person_resolves_from() {
        let request = validated(plan(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
                measure_key: "commit_count".to_owned(),
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        ));
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(
            sql.contains(
                "AND evidence.entity_id IN (SELECT email FROM identity.person_map WHERE person_id = ?)"
            ),
            "scoped by the person's identity set, not one id"
        );
        assert!(
            params.contains(&TEST_PERSON.to_string()),
            "the person is still what the request binds"
        );
        assert_eq!(sql.matches('?').count(), params.len());
    }

    // INVARIANT: the identity closes the ordering key. Two identities of one
    // person can tie on every other column, and a page boundary between two
    // indistinguishable rows repeats or skips one.
    #[test]
    fn the_ordering_key_ends_with_the_source_identity() {
        let request = validated(plan(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
                measure_key: "commit_count".to_owned(),
                presentation: evidence_presentation(
                    "git",
                    "commit_count",
                    EvidenceGranularity::Event,
                ),
            }],
        ));
        let (sql, _) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("ORDER BY role, metric_date"));
        assert!(
            sql.trim_end()
                .contains("ifNull(subject_key, ''), entity_id"),
            "identity is the last ordering column"
        );
    }

    fn ratio_plan(denominator_collapse: AliasCollapse) -> ValidatedMetricDrilldown {
        let numerator = input(MetricInputRole::Numerator, "commit_count");
        let denominator = input(MetricInputRole::Denominator, "commit_day");
        validated(plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 1.0,
            },
            vec![
                EvidenceInput {
                    role: MetricInputRole::Numerator,
                    alias_collapse: AliasCollapse::Sum,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    alias_collapse: denominator_collapse,
                    measure_key: denominator.measure_key,
                    presentation: EvidencePresentation {
                        detail_keys: &[],
                        show_value: true,
                    },
                },
            ],
        ))
    }

    #[test]
    fn a_ratio_drilldown_collapses_a_flag_denominator_before_the_daily_rollup() {
        let (sql, params) = compile_query(&ratio_plan(AliasCollapse::Max))
            .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(
            sql.contains(
                "multiIf(evidence.measure_key = 'commit_day', max(ifNull(evidence.contribution, 0)), sum(ifNull(evidence.contribution, 0)))"
            ),
            "the flagged half collapses with max, the additive half still sums"
        );
        assert!(
            sql.contains("GROUP BY evidence.metric_date, evidence.measure_key"),
            "collapse happens at the evidence grain, under the daily rollup"
        );
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn a_ratio_drilldown_collapses_an_inverse_flag_denominator_with_min() {
        let (sql, _) = compile_query(&ratio_plan(AliasCollapse::Min))
            .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("min(ifNull(evidence.contribution, 0))"));
        assert!(!sql.contains("max(ifNull(evidence.contribution, 0))"));
    }

    #[test]
    fn an_all_additive_ratio_drilldown_has_no_collapse_branch() {
        let (sql, _) = compile_query(&ratio_plan(AliasCollapse::Sum))
            .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(!sql.contains("multiIf("));
        assert!(sql.contains("sum(ifNull(evidence.contribution, 0))"));
    }
}
