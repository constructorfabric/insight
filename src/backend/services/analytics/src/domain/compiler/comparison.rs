//! Renders a comparison read: each target's own value beside the spread of the pool
//! it is compared against.
//!
//! A cohort names its members by person reference while a dataset keys its
//! rows by source identity, so the caller resolves each member's identities
//! and the statement joins the two: the pool is a bound (person, identity)
//! relation, the metric folds once per person over that person's rows, and the
//! distribution is taken over those per-person values. Everything stays one
//! statement — the fold, the pool, and the distribution are three stages of
//! one scan.
//!
//! Cohort membership is not a dataset. It is read from its own relation with
//! the same tenancy, entity type, and cohort key the request resolved, and
//! nothing here repairs it: a pool that tie-breaks its own input would hide
//! the input being wrong.

use std::fmt::Write;

use crate::domain::definitions::definition::MetricDefinition;
use crate::domain::field_catalog::model::CatalogDataset;

use super::error::CompileError;
use super::fold::{Fold, transform_in_place};
use super::request::{
    ComparisonMember, ComparisonPopulation, ComparisonView, EntityScope, MetricQuery,
};
use super::sql::{
    CompiledMeasureQuery, QueryParam, ReadScope, from_clause, placeholders, read_predicates,
};

/// Where cohort membership is declared.
const COHORT_RELATION: &str = "insight.metric_entity_cohorts_current";

/// Fewest observed peers a distribution is disclosed for. Quartiles over a
/// handful of people are noise presented as signal — someone is always the
/// bottom quarter of three — and at two the median discloses the one
/// colleague's value. Below the floor the pool size is still reported and
/// every other statistic reads NULL.
const MIN_PEER_N: u32 = 5;

pub(super) fn compile(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    view: &ComparisonView,
) -> Result<CompiledMeasureQuery, CompileError> {
    if view.targets.is_empty() {
        return Err(CompileError::EmptySelection {
            selection: "the comparison targets".to_owned(),
        });
    }

    let mut params = Vec::new();
    let head = match &view.population {
        ComparisonPopulation::DeclaredCohort { cohort_key } => {
            declare_cohort_ctes(metric, query, view, cohort_key, &mut params)?
        }
        ComparisonPopulation::Tenant => tenant_targets_cte(view, &mut params),
    };

    let pool = pool_cte(&view.pool, &mut params)?;
    let member_values = member_values_cte(dataset, metric, fold, query, &mut params)?;
    let carried = transform_in_place(metric.transform.as_ref(), "member_values.value");

    let mut sql = head;
    sql.push_str(&pool);
    sql.push_str(&member_values);
    match &view.population {
        ComparisonPopulation::DeclaredCohort { .. } => push_cohort_distribution(&mut sql, &carried),
        ComparisonPopulation::Tenant => push_tenant_distribution(&mut sql, &carried),
    }
    params.push(QueryParam::UInt(query.row_limit));

    Ok(CompiledMeasureQuery { sql, params })
}

/// The two reads of the cohort relation: the targets the view answers for, and
/// everyone who shares a cohort with one of them. Both scope by the same
/// tenancy, entity type, and cohort key, in that binding order.
fn declare_cohort_ctes(
    metric: &MetricDefinition,
    query: &MetricQuery,
    view: &ComparisonView,
    cohort_key: &str,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    if metric.cohort_key.as_deref() != Some(cohort_key) {
        return Err(CompileError::UndeclaredCohort {
            metric: metric.key.clone(),
            cohort_key: cohort_key.to_owned(),
        });
    }

    let mut sql = String::from("WITH targets AS (\n");
    push_cohort_scope(params, query, metric, cohort_key);
    params.extend(view.targets.iter().cloned().map(QueryParam::Text));
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        entity_id,");
    let _ = writeln!(sql, "        cohort_id");
    let _ = writeln!(sql, "    FROM {COHORT_RELATION}");
    let _ = writeln!(sql, "    WHERE tenant_id = ?");
    let _ = writeln!(sql, "      AND entity_type = ?");
    let _ = writeln!(sql, "      AND cohort_key = ?");
    let _ = writeln!(
        sql,
        "      AND entity_id IN ({})",
        placeholders(view.targets.len())
    );
    let _ = writeln!(sql, "),");

    push_cohort_scope(params, query, metric, cohort_key);
    let _ = writeln!(sql, "cohort AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        entity_id,");
    let _ = writeln!(sql, "        cohort_id");
    let _ = writeln!(sql, "    FROM {COHORT_RELATION}");
    let _ = writeln!(sql, "    WHERE tenant_id = ?");
    let _ = writeln!(sql, "      AND entity_type = ?");
    let _ = writeln!(sql, "      AND cohort_key = ?");
    let _ = writeln!(
        sql,
        "      AND cohort_id IN (SELECT cohort_id FROM targets)"
    );
    let _ = writeln!(sql, "),");

    Ok(sql)
}

// INVARIANT: tenancy leads a cohort read exactly as it leads a dataset read,
// bound from the request and never written into the statement.
fn push_cohort_scope(
    params: &mut Vec<QueryParam>,
    query: &MetricQuery,
    metric: &MetricDefinition,
    cohort_key: &str,
) {
    params.push(QueryParam::Text(query.tenant_id.clone()));
    params.push(QueryParam::Text(metric.entity_type.clone()));
    params.push(QueryParam::Text(cohort_key.to_owned()));
}

fn tenant_targets_cte(view: &ComparisonView, params: &mut Vec<QueryParam>) -> String {
    params.extend(view.targets.iter().cloned().map(QueryParam::Text));

    format!(
        "WITH targets AS (\n    SELECT arrayJoin([{}]) AS entity_id\n),\n",
        placeholders(view.targets.len())
    )
}

/// The caller's identity resolution as a bound relation: one row per
/// (person, identity) pair, so a person with several identities folds their
/// rows together and an identity is never attributed twice.
fn pool_cte(
    pool: &[ComparisonMember],
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    let pairs: usize = pool.iter().map(|member| member.identities.len()).sum();
    if pairs == 0 {
        return Err(CompileError::EmptySelection {
            selection: "the comparison pool".to_owned(),
        });
    }

    for member in pool {
        for identity in &member.identities {
            params.push(QueryParam::Text(member.person_ref.clone()));
            params.push(QueryParam::Text(identity.clone()));
        }
    }

    let tuples = vec!["(?, ?)"; pairs].join(", ");
    Ok(format!(
        "pool AS (\n    SELECT\n        member.1 AS person_ref,\n        member.2 AS identity\n    FROM (SELECT arrayJoin([{tuples}]) AS member)\n),\n"
    ))
}

/// The metric folded once per pool member. The pool join is what narrows this
/// read to the population, so the request's own entity scope is not a
/// predicate here — a comparison read answers about a target by comparing it to
/// people the target did not select.
fn member_values_cte(
    dataset: &CatalogDataset,
    metric: &MetricDefinition,
    fold: &Fold<'_>,
    query: &MetricQuery,
    params: &mut Vec<QueryParam>,
) -> Result<String, CompileError> {
    let population = EntityScope::Tenant;
    let scope = ReadScope::of_metric(query).over_every_entity(&population);

    let value = fold.value_expr(metric, params)?;
    let predicates = read_predicates(dataset, fold.grain, fold.where_filter, &scope, params)?;

    let mut sql = String::from("member_values AS (\n    SELECT\n");
    let _ = writeln!(sql, "        pool.person_ref AS entity_id,");
    let _ = writeln!(sql, "        {value} AS value");
    let _ = writeln!(sql, "    FROM {}", from_clause(dataset));
    let _ = writeln!(
        sql,
        "    INNER JOIN pool ON pool.identity = {}",
        fold.grain.entity
    );
    let _ = writeln!(sql, "    WHERE {}", predicates.join("\n      AND "));
    let _ = writeln!(sql, "    GROUP BY entity_id");
    let _ = writeln!(sql, "),");
    Ok(sql)
}

// WORKAROUND: ClickHouse inlines a `WITH` body once per reference, so a second
// reference to `entity_values` would re-scan the whole window. The target's own
// value is extracted from the peer rows with a conditional aggregate instead.
fn push_cohort_distribution(sql: &mut String, carried: &str) {
    let _ = writeln!(sql, "entity_values AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        cohort.entity_id AS entity_id,");
    let _ = writeln!(sql, "        cohort.cohort_id AS cohort_id,");
    let _ = writeln!(sql, "        {carried} AS value");
    let _ = writeln!(sql, "    FROM cohort");
    let _ = writeln!(
        sql,
        "    LEFT JOIN member_values ON member_values.entity_id = cohort.entity_id"
    );
    let _ = writeln!(sql, ")");
    let _ = writeln!(sql, "SELECT");
    let _ = writeln!(sql, "    targets.entity_id AS entity_id,");
    // INVARIANT: at most one peer row matches a target — the cohort relation
    // is one row per (tenant, entity, cohort key) and `member_values` groups by
    // person — so `maxIf` returns exactly that row's value, NULL when the
    // target was never observed.
    let _ = writeln!(
        sql,
        "    maxIf(peer.value, peer.entity_id = targets.entity_id) AS target_value,"
    );
    push_distribution_selects(sql, "    ");
    let _ = writeln!(sql, "FROM targets");
    let _ = writeln!(sql, "LEFT JOIN entity_values AS peer");
    let _ = writeln!(sql, "    ON peer.cohort_id = targets.cohort_id");
    let _ = writeln!(sql, "GROUP BY targets.entity_id");
    let _ = writeln!(sql, "LIMIT ?");
    let _ = write!(sql, "SETTINGS join_use_nulls = 1");
}

// INVARIANT: the population is aggregated once over the whole pool and the
// requested targets' own values are captured in that same pass, so the cost is
// one scan and one aggregation however many targets the request carries.
fn push_tenant_distribution(sql: &mut String, carried: &str) {
    let _ = writeln!(sql, "entity_values AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(sql, "        entity_id,");
    let _ = writeln!(sql, "        {carried} AS value");
    let _ = writeln!(sql, "    FROM member_values");
    let _ = writeln!(sql, "),");
    let _ = writeln!(sql, "population_stats AS (");
    let _ = writeln!(sql, "    SELECT");
    let _ = writeln!(
        sql,
        "        groupArrayIf(tuple(peer.entity_id, peer.value), peer.entity_id IN (SELECT entity_id FROM targets)) AS target_rows,"
    );
    push_distribution_selects(sql, "        ");
    let _ = writeln!(sql, "    FROM entity_values AS peer");
    let _ = writeln!(sql, ")");
    let _ = writeln!(sql, "SELECT");
    let _ = writeln!(sql, "    targets.entity_id AS entity_id,");
    // SAFETY: `arrayFirst` yields the default tuple when a target was never
    // captured; its Nullable value element reads NULL, so an unobserved target
    // stays unobserved rather than reading a zero.
    let _ = writeln!(
        sql,
        "    tupleElement(arrayFirst(row -> row.1 = targets.entity_id, population_stats.target_rows), 2) AS target_value,"
    );
    for statistic in ["p25", "median", "p75", "min", "max"] {
        let _ = writeln!(sql, "    population_stats.{statistic} AS {statistic},");
    }
    let _ = writeln!(sql, "    population_stats.n AS n");
    let _ = writeln!(sql, "FROM targets");
    let _ = writeln!(sql, "CROSS JOIN population_stats");
    let _ = writeln!(sql, "LIMIT ?");
    let _ = write!(sql, "SETTINGS join_use_nulls = 1");
}

/// The pool's spread, withheld below the disclosure floor. The pool size is
/// reported whatever it is, so a consumer can say why the rest is absent.
fn push_distribution_selects(sql: &mut String, indent: &str) {
    let observed = "peer.value IS NOT NULL";
    let pool = format!("uniqExactIf(peer.entity_id, {observed})");
    let quantiles = format!("quantilesExactIf(0.25, 0.5, 0.75)(peer.value, {observed})");

    for (position, statistic) in ["p25", "median", "p75"].into_iter().enumerate() {
        let _ = writeln!(
            sql,
            "{indent}if({pool} >= {MIN_PEER_N}, toNullable({quantiles}[{}]), NULL) AS {statistic},",
            position + 1
        );
    }
    let _ = writeln!(
        sql,
        "{indent}if({pool} >= {MIN_PEER_N}, minIfOrNull(peer.value, {observed}), NULL) AS min,"
    );
    let _ = writeln!(
        sql,
        "{indent}if({pool} >= {MIN_PEER_N}, maxIfOrNull(peer.value, {observed}), NULL) AS max,"
    );
    let _ = writeln!(sql, "{indent}toUInt64({pool}) AS n");
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use crate::domain::compiler::error::CompileError;
    use crate::domain::compiler::fixtures::{
        compile, compile_err, direct, lines, measure, metric, percent_of_total, query, text,
    };
    use crate::domain::compiler::request::{
        ComparisonMember, ComparisonPopulation, ComparisonView, EntityScope, ViewKind,
    };
    use crate::domain::compiler::sql::QueryParam;
    use crate::domain::definitions::definition::MetricDefinition;

    fn cohort_metric() -> MetricDefinition {
        MetricDefinition {
            cohort_key: Some("org_unit".to_owned()),
            ..metric(direct("prs_merged"))
        }
    }

    fn pool() -> Vec<ComparisonMember> {
        vec![
            ComparisonMember {
                person_ref: "person-1".to_owned(),
                identities: vec![
                    "one@example.com".to_owned(),
                    "one.alt@example.com".to_owned(),
                ],
            },
            ComparisonMember {
                person_ref: "person-2".to_owned(),
                identities: vec!["two@example.com".to_owned()],
            },
        ]
    }

    fn view(population: ComparisonPopulation) -> ViewKind {
        ViewKind::Comparison(ComparisonView {
            population,
            targets: vec!["person-1".to_owned()],
            pool: pool(),
        })
    }

    fn declared() -> ComparisonPopulation {
        ComparisonPopulation::DeclaredCohort {
            cohort_key: "org_unit".to_owned(),
        }
    }

    #[test]
    fn a_declared_cohort_comparison_read_folds_the_pool_and_takes_its_spread_in_one_statement() {
        let compiled = compile(
            &cohort_metric(),
            &[measure("prs_merged", None)],
            &query(view(declared())),
        );

        assert_eq!(
            compiled.sql,
            lines(&[
                "WITH targets AS (",
                "    SELECT",
                "        entity_id,",
                "        cohort_id",
                "    FROM insight.metric_entity_cohorts_current",
                "    WHERE tenant_id = ?",
                "      AND entity_type = ?",
                "      AND cohort_key = ?",
                "      AND entity_id IN (?)",
                "),",
                "cohort AS (",
                "    SELECT",
                "        entity_id,",
                "        cohort_id",
                "    FROM insight.metric_entity_cohorts_current",
                "    WHERE tenant_id = ?",
                "      AND entity_type = ?",
                "      AND cohort_key = ?",
                "      AND cohort_id IN (SELECT cohort_id FROM targets)",
                "),",
                "pool AS (",
                "    SELECT",
                "        member.1 AS person_ref,",
                "        member.2 AS identity",
                "    FROM (SELECT arrayJoin([(?, ?), (?, ?), (?, ?)]) AS member)",
                "),",
                "member_values AS (",
                "    SELECT",
                "        pool.person_ref AS entity_id,",
                "        toFloat64(count()) AS value",
                "    FROM silver.class_git_pull_requests FINAL",
                "    INNER JOIN pool ON pool.identity = author_email",
                "    WHERE tenant_id = ?",
                "      AND toDate(closed_on) >= toDate(?)",
                "      AND toDate(closed_on) <= toDate(?)",
                "    GROUP BY entity_id",
                "),",
                "entity_values AS (",
                "    SELECT",
                "        cohort.entity_id AS entity_id,",
                "        cohort.cohort_id AS cohort_id,",
                "        member_values.value AS value",
                "    FROM cohort",
                "    LEFT JOIN member_values ON member_values.entity_id = cohort.entity_id",
                ")",
                "SELECT",
                "    targets.entity_id AS entity_id,",
                "    maxIf(peer.value, peer.entity_id = targets.entity_id) AS target_value,",
                "    if(uniqExactIf(peer.entity_id, peer.value IS NOT NULL) >= 5, toNullable(quantilesExactIf(0.25, 0.5, 0.75)(peer.value, peer.value IS NOT NULL)[1]), NULL) AS p25,",
                "    if(uniqExactIf(peer.entity_id, peer.value IS NOT NULL) >= 5, toNullable(quantilesExactIf(0.25, 0.5, 0.75)(peer.value, peer.value IS NOT NULL)[2]), NULL) AS median,",
                "    if(uniqExactIf(peer.entity_id, peer.value IS NOT NULL) >= 5, toNullable(quantilesExactIf(0.25, 0.5, 0.75)(peer.value, peer.value IS NOT NULL)[3]), NULL) AS p75,",
                "    if(uniqExactIf(peer.entity_id, peer.value IS NOT NULL) >= 5, minIfOrNull(peer.value, peer.value IS NOT NULL), NULL) AS min,",
                "    if(uniqExactIf(peer.entity_id, peer.value IS NOT NULL) >= 5, maxIfOrNull(peer.value, peer.value IS NOT NULL), NULL) AS max,",
                "    toUInt64(uniqExactIf(peer.entity_id, peer.value IS NOT NULL)) AS n",
                "FROM targets",
                "LEFT JOIN entity_values AS peer",
                "    ON peer.cohort_id = targets.cohort_id",
                "GROUP BY targets.entity_id",
                "LIMIT ?",
                "SETTINGS join_use_nulls = 1",
            ])
        );
        assert_eq!(
            compiled.params,
            vec![
                text("acme-tenant"),
                text("person"),
                text("org_unit"),
                text("person-1"),
                text("acme-tenant"),
                text("person"),
                text("org_unit"),
                text("person-1"),
                text("one@example.com"),
                text("person-1"),
                text("one.alt@example.com"),
                text("person-2"),
                text("two@example.com"),
                text("acme-tenant"),
                text("2026-01-01"),
                text("2026-01-31"),
                QueryParam::UInt(10_001),
            ]
        );
    }

    #[test]
    fn a_tenant_population_takes_its_spread_over_the_whole_pool_in_one_pass() {
        let compiled = compile(
            &cohort_metric(),
            &[measure("prs_merged", None)],
            &query(view(ComparisonPopulation::Tenant)),
        );

        assert!(compiled.sql.starts_with(&lines(&[
            "WITH targets AS (",
            "    SELECT arrayJoin([?]) AS entity_id",
            "),",
            "pool AS (",
        ])));
        assert!(
            !compiled
                .sql
                .contains("insight.metric_entity_cohorts_current")
        );
        assert!(compiled.sql.contains(&lines(&[
            "population_stats AS (",
            "    SELECT",
            "        groupArrayIf(tuple(peer.entity_id, peer.value), peer.entity_id IN (SELECT entity_id FROM targets)) AS target_rows,",
        ])));
        assert!(compiled.sql.contains(&lines(&[
            "SELECT",
            "    targets.entity_id AS entity_id,",
            "    tupleElement(arrayFirst(row -> row.1 = targets.entity_id, population_stats.target_rows), 2) AS target_value,",
            "    population_stats.p25 AS p25,",
            "    population_stats.median AS median,",
            "    population_stats.p75 AS p75,",
            "    population_stats.min AS min,",
            "    population_stats.max AS max,",
            "    population_stats.n AS n",
            "FROM targets",
            "CROSS JOIN population_stats",
            "LIMIT ?",
            "SETTINGS join_use_nulls = 1",
        ])));
        assert_eq!(compiled.sql.matches('?').count(), compiled.params.len());
    }

    #[test]
    fn every_disclosed_statistic_is_withheld_below_the_pool_floor() {
        let compiled = compile(
            &cohort_metric(),
            &[measure("prs_merged", None)],
            &query(view(declared())),
        );

        for statistic in ["p25", "median", "p75", "min", "max"] {
            let projection = compiled
                .sql
                .lines()
                .find(|line| line.ends_with(&format!(" AS {statistic},")))
                .unwrap_or_else(|| panic!("{statistic} is projected"));

            assert!(
                projection.starts_with(
                    "    if(uniqExactIf(peer.entity_id, peer.value IS NOT NULL) >= 5, "
                ),
                "{projection}"
            );
        }
        assert!(
            compiled
                .sql
                .contains("    toUInt64(uniqExactIf(peer.entity_id, peer.value IS NOT NULL)) AS n"),
            "the pool size is reported whatever it is"
        );
    }

    #[test]
    fn the_pool_join_narrows_the_read_rather_than_the_requests_own_entity_scope() {
        let mut request = query(view(declared()));
        request.entity_scope = EntityScope::Identities(vec!["someone@example.com".to_owned()]);

        let compiled = compile(&cohort_metric(), &[measure("prs_merged", None)], &request);

        assert!(
            compiled
                .sql
                .contains("    INNER JOIN pool ON pool.identity = author_email")
        );
        assert!(!compiled.sql.contains("author_email IN ("));
        assert!(!compiled.params.contains(&text("someone@example.com")));
    }

    #[test]
    fn the_transform_is_projected_over_each_members_value_before_the_spread_is_taken() {
        let mut metric = cohort_metric();
        metric.transform = Some(percent_of_total());

        let compiled = compile(
            &metric,
            &[measure("prs_merged", None)],
            &query(view(declared())),
        );

        assert!(compiled.sql.contains(
            "        if((100.0 * (member_values.value)) IS NULL, NULL, least(100.0, greatest(0.0, 100.0 * (member_values.value)))) AS value"
        ));
    }

    #[test]
    fn a_comparison_read_of_a_cohort_the_metric_does_not_declare_is_rejected() {
        assert_eq!(
            compile_err(
                &cohort_metric(),
                &[measure("prs_merged", None)],
                &query(view(ComparisonPopulation::DeclaredCohort {
                    cohort_key: "tenure_band".to_owned(),
                }))
            ),
            CompileError::UndeclaredCohort {
                metric: "git.merge_rate".to_owned(),
                cohort_key: "tenure_band".to_owned(),
            }
        );
    }

    #[test]
    fn a_comparison_read_with_no_target_or_no_resolved_identity_is_rejected() {
        let mut no_targets = query(view(declared()));
        if let ViewKind::Comparison(view) = &mut no_targets.view {
            view.targets.clear();
        }
        assert_eq!(
            compile_err(
                &cohort_metric(),
                &[measure("prs_merged", None)],
                &no_targets
            ),
            CompileError::EmptySelection {
                selection: "the comparison targets".to_owned(),
            }
        );

        let mut no_identities = query(view(declared()));
        if let ViewKind::Comparison(view) = &mut no_identities.view {
            for member in &mut view.pool {
                member.identities.clear();
            }
        }
        assert_eq!(
            compile_err(
                &cohort_metric(),
                &[measure("prs_merged", None)],
                &no_identities
            ),
            CompileError::EmptySelection {
                selection: "the comparison pool".to_owned(),
            }
        );
    }
}
