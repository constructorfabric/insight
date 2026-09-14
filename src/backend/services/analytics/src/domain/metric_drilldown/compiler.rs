use std::fmt::Write;

use toolkit_canonical_errors::CanonicalError;

use crate::domain::metric_definitions::ComputationSpec;
use crate::domain::metric_definitions::definition::{MetricInputRole, RatioDenominatorAggregation};

use super::cursor::CursorKey;
use super::dto::{
    EvidenceInput, EvidenceQueryRow, MetricDrilldownColumn, MetricDrilldownEntity,
    MetricDrilldownFilter, ValidatedMetricDrilldown,
};
use super::error::config_error;
use super::presentation::presentation_columns;
use super::sort::{
    MetricDrilldownSort, MetricDrilldownSortDirection, PERSON_KEY, column_sql, empty_flag,
};

/// Person evidence is keyed by the source identity, so a person's rows are the
/// rows of every identity the live map resolves to them; tenant evidence
/// repeats its tenant key as the entity id.
pub fn compile_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
    if matches!(req.plan.definition.spec, ComputationSpec::Ratio { .. }) {
        return compile_ratio_query(req);
    }
    compile_value_query(req)
}

fn compile_value_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
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
    let columns = presentation_columns(
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
        &req.selection.entity,
    );
    let order = OrderKey::build(&columns, &req.selection.sort, false)?;
    let person = person_projection(&req.selection.entity, &columns, &mut params);
    params.extend([
        req.tenant_id.to_string(),
        req.plan.source_key.clone(),
        req.selection.entity.entity_type().to_owned(),
    ]);
    let entity = entity_predicate(&req.selection.entity, &mut params);
    params.extend([req.from.to_string(), req.to.to_string()]);
    params.extend(
        req.plan
            .inputs
            .iter()
            .map(|input| input.measure_key.clone()),
    );
    let filter_sql = filter_predicate(&req.selection.filters, &mut params);
    let search = search_predicate(
        req.selection.search.as_deref(),
        &columns,
        false,
        &person,
        &req.search_person_ids,
        &mut params,
    );
    let cursor = order.cursor_predicate(
        req.cursor.as_ref(),
        "role, toString(evidence.metric_date), ifNull(toString(evidence.observed_at), ''), evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, ifNull(evidence.subject_key, ''), evidence.entity_id",
        &mut params,
    );
    let narrowing_sql = conjunction("AND", &[search, cursor]);
    let limit = req.limit + 1;
    let tenant =
        crate::domain::metric_results::compiler::tenant_predicate(req.enforce_tenant_scope);
    let order_by = order.order_by(&[
        "role",
        "metric_date",
        "ifNull(toString(observed_at), '')",
        "source_key",
        "measure_key",
        "record_id",
        "record_kind",
        "ifNull(subject_key, '')",
        "entity_id",
    ]);
    let sql = format!(
        "WITH {role_expr} AS role, {sort_source} \
         SELECT role, evidence.entity_id AS entity_id, toString(evidence.metric_date) AS metric_date, ifNull(toString(evidence.observed_at), '') AS observed_at, \
                evidence.source_key, evidence.measure_key, evidence.record_id, evidence.record_kind, \
                evidence.contribution, CAST(NULL AS Nullable(Float64)) AS numerator, \
                CAST(NULL AS Nullable(Float64)) AS denominator, \
                ifNull(evidence.subject_key, '') AS subject_key, \
                toJSONString(evidence.dimensions) AS dimensions_json, evidence.details, \
                {person} AS person_id, {projection} \
         FROM {database}.{table} AS evidence{person_join} \
         WHERE {tenant} AND evidence.source_key = ? AND evidence.entity_type = ? AND {entity} \
           AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
           AND evidence.measure_key IN ({measures}){filter_sql}{narrowing_sql} \
         ORDER BY {order_by} \
         LIMIT {limit}",
        person = person.expression,
        person_join = person.join,
        sort_source = order.source(),
        projection = order.projection(),
    );
    Ok((sql, params))
}

/// The `person` column's identity resolution, and nothing more: a selection
/// that does not present the column pays for no join.
struct PersonProjection {
    expression: String,
    join: String,
}

impl PersonProjection {
    /// A query that resolves nobody: the column is not presented, so there is
    /// no join to read and no identity to compare against.
    fn unresolved() -> Self {
        Self {
            expression: "''".to_owned(),
            join: String::new(),
        }
    }

    fn resolves(&self) -> bool {
        !self.join.is_empty()
    }
}

/// INVARIANT: both identity relations are views, so an unnarrowed join reads
/// every account and every email the tenant holds. The roster the selection
/// already binds is what bounds them.
fn person_projection(
    entity: &MetricDrilldownEntity,
    columns: &[MetricDrilldownColumn],
    params: &mut Vec<String>,
) -> PersonProjection {
    let MetricDrilldownEntity::Persons { ids } = entity else {
        return PersonProjection::unresolved();
    };
    if ids.is_empty() || !columns.iter().any(|column| column.key == PERSON_KEY) {
        return PersonProjection::unresolved();
    }

    let roster = vec!["?"; ids.len()].join(", ");
    params.extend(ids.iter().cloned());
    params.extend(ids.iter().cloned());

    PersonProjection {
        expression: resolved_person_expr().to_owned(),
        join: format!(
            " LEFT JOIN (SELECT source_type, source_id, account_id, person_id \
                  FROM {account_map} WHERE person_id IN ({roster})) AS account_map \
                  ON account_map.source_type = evidence.account_source_type \
                 AND account_map.source_id = {account_source_uuid} \
                 AND account_map.account_id = {account_id} \
              LEFT JOIN (SELECT email, person_id FROM {person_map} \
                  WHERE person_id IN ({roster})) AS person_map \
                  ON person_map.email = evidence.entity_id",
            account_map = crate::domain::metric_results::compiler::ACCOUNT_ASSIGNMENT_RELATION,
            person_map = crate::domain::metric_results::compiler::PERSON_MAP_RELATION,
            account_source_uuid =
                crate::domain::metric_results::compiler::account_source_uuid_expr("evidence"),
            account_id = crate::domain::metric_results::compiler::account_id_expr("evidence"),
        ),
    }
}

/// The two halves of the division and how the denominator adds up — a plan
/// missing any of the three describes a ratio nothing can compute.
fn ratio_halves(
    req: &ValidatedMetricDrilldown,
) -> Result<(&EvidenceInput, &EvidenceInput, &'static str), CanonicalError> {
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
            "sumIf(collapsed.contribution, collapsed.measure_key = ?)"
        }
        RatioDenominatorAggregation::DistinctCount => {
            "toFloat64(uniqExactIf(collapsed.subject_key, collapsed.measure_key = ? AND collapsed.subject_key IS NOT NULL))"
        }
    };
    Ok((numerator, denominator, denominator_expr))
}

fn compile_ratio_query(
    req: &ValidatedMetricDrilldown,
) -> Result<(String, Vec<String>), CanonicalError> {
    let (database, table) = req.plan.relation.table_ref();
    let (numerator, denominator, denominator_expr) = ratio_halves(req)?;
    let columns = presentation_columns(
        &req.plan,
        &req.selection.filters,
        &req.selection.display_dimensions,
        &req.selection.entity,
    );
    let order = OrderKey::build(&columns, &req.selection.sort, true)?;
    let mut params = vec![
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
        req.tenant_id.to_string(),
        req.plan.source_key.clone(),
        req.selection.entity.entity_type().to_owned(),
    ];
    let entity = entity_predicate(&req.selection.entity, &mut params);
    params.extend([
        req.from.to_string(),
        req.to.to_string(),
        numerator.measure_key.clone(),
        denominator.measure_key.clone(),
    ]);
    let filter_sql = filter_predicate(&req.selection.filters, &mut params);
    // Both terms read the aggregates the subquery emits, so both wait for the
    // outer level rather than joining the filters inside.
    let search = search_predicate(
        req.selection.search.as_deref(),
        &columns,
        true,
        &PersonProjection::unresolved(),
        &[],
        &mut params,
    );
    let cursor = order.cursor_predicate(
        req.cursor.as_ref(),
        "role, metric_date, observed_at, source_key, measure_key, record_id, record_kind, subject_key, entity_id",
        &mut params,
    );
    let outer_sql = where_clause(&[search, cursor]);
    let limit = req.limit + 1;
    let tenant =
        crate::domain::metric_results::compiler::tenant_predicate(req.enforce_tenant_scope);
    let order_by = order.order_by(&[
        "role",
        "metric_date",
        "observed_at",
        "source_key",
        "measure_key",
        "record_id",
        "record_kind",
        "subject_key",
        "entity_id",
    ]);
    // INVARIANT: a flagged measure collapses identities before the daily rollup
    // sums it, or the drilldown explains a ratio the tile never showed.
    let collapsed = collapsed_evidence_value(numerator, denominator);
    let sql = format!(
        "WITH {sort_source} \
         SELECT *, {projection} FROM (\
            SELECT 'value' AS role, '' AS entity_id, '' AS person_id, toString(collapsed.metric_date) AS metric_date, \
                   '' AS observed_at, \
                   any(collapsed.source_key) AS source_key, '' AS measure_key, \
                   toString(collapsed.metric_date) AS record_id, 'daily_ratio' AS record_kind, \
                   CAST(NULL AS Nullable(Float64)) AS contribution, \
                   sumIf(collapsed.contribution, collapsed.measure_key = ?) AS numerator, \
                   {denominator_expr} AS denominator, \
                   '' AS subject_key, any(collapsed.dimensions_json) AS dimensions_json, \
                   CAST(map() AS Map(String, String)) AS details \
            FROM (\
                SELECT evidence.metric_date AS metric_date, \
                       any(evidence.source_key) AS source_key, \
                       evidence.measure_key AS measure_key, \
                       evidence.subject_key AS subject_key, \
                       toJSONString(evidence.dimensions) AS dimensions_json, \
                       {collapsed} AS contribution \
                FROM {database}.{table} AS evidence \
                LEFT JOIN {account_map_relation} AS account_map \
                    ON account_map.source_type = evidence.account_source_type \
                   AND account_map.source_id = {account_source_uuid} \
                   AND account_map.account_id = {account_id} \
                LEFT JOIN {person_map_relation} AS person_map \
                    ON person_map.email = evidence.entity_id \
                WHERE {tenant} AND evidence.source_key = ? AND evidence.entity_type = ? AND {entity} \
                  AND evidence.metric_date >= toDate(?) AND evidence.metric_date <= toDate(?) \
                  AND evidence.measure_key IN (?, ?){filter_sql} \
                GROUP BY evidence.metric_date, evidence.measure_key, \
                         evidence.subject_key, toJSONString(evidence.dimensions), \
                         {resolved_person}\
            ) AS collapsed \
            GROUP BY collapsed.metric_date\
         ){outer_sql} \
         ORDER BY {order_by} \
         LIMIT {limit}",
        sort_source = order.source(),
        projection = order.projection(),
        account_map_relation = crate::domain::metric_results::compiler::ACCOUNT_ASSIGNMENT_RELATION,
        person_map_relation = crate::domain::metric_results::compiler::PERSON_MAP_RELATION,
        account_source_uuid =
            crate::domain::metric_results::compiler::account_source_uuid_expr("evidence"),
        account_id = crate::domain::metric_results::compiler::account_id_expr("evidence"),
        resolved_person = resolved_person_expr(),
    );
    Ok((sql, params))
}

/// INVARIANT: the collapse groups by the resolved PERSON, not the source
/// identity. Grouping by identity would leave one person's aliases uncollapsed;
/// omitting the person entirely would collapse a roster's people into each
/// other, so a flagged denominator would read `1` for a whole team.
fn resolved_person_expr() -> &'static str {
    "multiIf(coalesce(account_map.account_id, '') != '', \
      toString(assumeNotNull(account_map.person_id)), \
      coalesce(person_map.email, '') != '', \
      toString(assumeNotNull(person_map.person_id)), \
      evidence.entity_id)"
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

/// The evidence row's account-binding key, in the identity store's form —
/// shared with the observation reads so both surfaces resolve identically.
fn account_binding_key(alias: &str) -> String {
    format!(
        "({alias}.account_source_type, {source_uuid}, {account_id})",
        source_uuid = crate::domain::metric_results::compiler::account_source_uuid_expr(alias),
        account_id = crate::domain::metric_results::compiler::account_id_expr(alias),
    )
}

/// Person selections resolve account-first: a row whose account key is bound
/// decides by that binding alone — a binding to the requested person matches,
/// any other binding (the excluded person included) terminates — and only an
/// unbound row falls back to the person's email set. Tenant evidence keys on
/// the tenant itself. Pushes its own params where the predicate renders; a
/// person id is bound once per arm, never interpolated into the SQL.
fn entity_predicate(
    entity: &super::dto::MetricDrilldownEntity,
    params: &mut Vec<String>,
) -> String {
    let account_key = account_binding_key("evidence");
    match entity {
        super::dto::MetricDrilldownEntity::Person { id } => {
            params.push(id.clone());
            params.push(id.clone());
            format!(
                "({account_key} IN (SELECT source_type, source_id, account_id \
                 FROM {account_map} WHERE person_id = ?) \
                 OR ({account_key} NOT IN (SELECT source_type, source_id, account_id \
                 FROM {account_map}) \
                 AND evidence.entity_id IN (SELECT email FROM {person_map} WHERE person_id = ?)))",
                account_map = crate::domain::metric_results::compiler::ACCOUNT_ASSIGNMENT_RELATION,
                person_map = crate::domain::metric_results::compiler::PERSON_MAP_RELATION,
            )
        }
        super::dto::MetricDrilldownEntity::Persons { ids } if !ids.is_empty() => {
            params.extend(ids.iter().cloned());
            params.extend(ids.iter().cloned());
            let id_params = vec!["?"; ids.len()].join(", ");
            format!(
                "({account_key} IN (SELECT source_type, source_id, account_id \
                 FROM {account_map} WHERE person_id IN ({id_params})) \
                 OR ({account_key} NOT IN (SELECT source_type, source_id, account_id \
                 FROM {account_map}) \
                 AND evidence.entity_id IN (SELECT email FROM {person_map} WHERE person_id IN ({id_params}))))",
                account_map = crate::domain::metric_results::compiler::ACCOUNT_ASSIGNMENT_RELATION,
                person_map = crate::domain::metric_results::compiler::PERSON_MAP_RELATION,
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

/// The sorted cell, bound once. The flag, the key, the cursor comparison and
/// the ORDER BY all read it, and the fallback expression behind it is long
/// enough that repeating it would dominate the query text.
///
/// INVARIANT: the bound term carries no placeholder. It leads the query, so
/// one would take the first bound parameter and shift every other by one —
/// column keys reach it as inlined literals for exactly this reason.
const SORT_SOURCE: &str = "sort_source";

/// The order the rows travel in, and everything that has to agree with it: the
/// ORDER BY, the columns the cursor replays, and how a replayed key re-enters
/// the comparison.
///
/// INVARIANT: one direction for the whole key. Blank cells are pushed last by
/// the leading flag rather than by a second direction, so the next page stays a
/// single tuple comparison instead of a per-column chain.
struct OrderKey {
    source: String,
    flag: String,
    key: String,
    binding: &'static str,
    direction: MetricDrilldownSortDirection,
}

impl OrderKey {
    /// INVARIANT: an unresolved sort has no ordering key to fall back on. A
    /// constant one would still be replayed against a cursor minted by a real
    /// key, and the two leading tuple elements would decide the comparison
    /// before the tiebreakers were reached — serving a page twice or dropping
    /// the rest of the result set, silently. Validation refuses such a sort
    /// first, so reaching here is a fault, not a degradation.
    fn build(
        columns: &[MetricDrilldownColumn],
        sort: &MetricDrilldownSort,
        ratio: bool,
    ) -> Result<Self, CanonicalError> {
        let sql = columns
            .iter()
            .find(|column| column.key == sort.key)
            .and_then(|column| column_sql(&column.key, column.r#type, ratio))
            .ok_or_else(config_error)?;
        Ok(Self {
            flag: empty_flag(SORT_SOURCE, sort.direction),
            key: sql.order_key(SORT_SOURCE),
            binding: sql.cursor_binding(),
            source: sql.as_text().to_owned(),
            direction: sort.direction,
        })
    }

    /// The `WITH` term the rest of the key reads through.
    fn source(&self) -> String {
        format!("{source} AS {SORT_SOURCE}", source = self.source)
    }

    fn projection(&self) -> String {
        format!(
            "{flag} AS sort_flag, {key} AS sort_key, toString(sort_key) AS sort_value",
            flag = self.flag,
            key = self.key,
        )
    }

    fn order_by(&self, tiebreakers: &[&str]) -> String {
        let direction = self.direction.as_sql();
        let mut sql = format!("sort_flag {direction}, sort_key {direction}");
        for tiebreaker in tiebreakers {
            let _ = write!(sql, ", {tiebreaker} {direction}");
        }
        sql
    }

    fn cursor_predicate(
        &self,
        cursor: Option<&CursorKey>,
        key_tuple: &str,
        params: &mut Vec<String>,
    ) -> Option<String> {
        let cursor = cursor?;
        // INVARIANT: bound in the order the tuple names the columns.
        params.push(u8::from(cursor.sort_flag).to_string());
        params.push(cursor.sort_value.clone());
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
        Some(format!(
            "tuple({flag}, {key}, {key_tuple}) {operator} \
             tuple(toUInt8(?), {binding}, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            flag = self.flag,
            key = self.key,
            operator = self.direction.cursor_operator(),
            binding = self.binding,
        ))
    }
}

/// A free-text needle against every column the reader can see, read exactly as
/// the cell is rendered.
///
/// INVARIANT: the UTF-8 form of the comparison. A name or a commit subject is
/// not always ASCII, and the ASCII form would fold only half of one.
///
/// `Who` is the one column the row does not carry as text — the query holds an
/// identity and the reader sees a name — so it is matched by the ids whose
/// names the caller's needle already picked out. Leaving it out would make a
/// visible column silently unsearchable.
fn search_predicate(
    search: Option<&str>,
    columns: &[MetricDrilldownColumn],
    ratio: bool,
    person: &PersonProjection,
    person_ids: &[String],
    params: &mut Vec<String>,
) -> Option<String> {
    let needle = search?;
    let mut matches = Vec::new();
    for column in columns
        .iter()
        .filter_map(|column| column_sql(&column.key, column.r#type, ratio))
    {
        params.push(needle.to_owned());
        matches.push(format!(
            "positionCaseInsensitiveUTF8({}, ?) > 0",
            column.as_text()
        ));
    }
    if person.resolves() && !person_ids.is_empty() {
        let placeholders = vec!["?"; person_ids.len()].join(", ");
        params.extend(person_ids.iter().cloned());
        matches.push(format!(
            "{expression} IN ({placeholders})",
            expression = person.expression
        ));
    }
    if matches.is_empty() {
        return None;
    }
    Some(format!("({})", matches.join(" OR ")))
}

fn conjunction(keyword: &str, predicates: &[Option<String>]) -> String {
    let mut sql = String::new();
    for predicate in predicates.iter().flatten() {
        let _ = write!(sql, " {keyword} {predicate}");
    }
    sql
}

fn where_clause(predicates: &[Option<String>]) -> String {
    let present = predicates.iter().flatten().collect::<Vec<_>>();
    let Some((first, rest)) = present.split_first() else {
        return String::new();
    };
    let mut sql = format!(" WHERE {first}");
    for predicate in rest {
        let _ = write!(sql, " AND {predicate}");
    }
    sql
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
    use crate::domain::metric_definitions::definition::AliasCollapse;
    use crate::domain::metric_definitions::{
        EvidenceGranularity, EvidencePresentation, RatioDenominatorAggregation,
    };
    use crate::domain::metric_drilldown::test_support::{
        TEST_PERSON, TEST_TENANT, commit_presentation, input, plan, validated,
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
                alias_collapse: AliasCollapse::Sum,
                measure_key: value.measure_key,
                presentation: commit_presentation(),
            }],
        );
        let mut request = validated(plan);
        request.cursor = Some(CursorKey {
            sort_flag: false,
            sort_value: "2026-07-01".to_owned(),
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
                &TEST_PERSON.to_string(), // the person, once per resolution arm:
                &TEST_PERSON.to_string(), // account binding first, email fallback
                "2026-07-01",             // period bounds
                "2026-07-31",
                "commit_count", // measure_key IN
                "repository",   // filter: indexOf twice, then values
                "repository",
                "org/repo",
                "0", // cursor tuple: emptiness flag, then the sorted cell
                "2026-07-01",
                "value", // then the tiebreakers, the complete ordering key
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
    fn tenant_query_matches_the_entity_to_its_storage_partition() {
        let value = input(MetricInputRole::Value, "commit_count");
        let plan = plan(
            ComputationSpec::Sum {
                value: value.clone(),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
                measure_key: value.measure_key,
                presentation: commit_presentation(),
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
                alias_collapse: AliasCollapse::Sum,
                measure_key: value.measure_key,
                presentation: commit_presentation(),
            }],
        );
        let second = Uuid::from_u128(0x019e_2830_0000_7000_8000_0000_0000_0002);
        let mut request = validated(plan);
        request.selection.entity = super::super::dto::MetricDrilldownEntity::Persons {
            ids: vec![TEST_PERSON.to_string(), second.to_string()],
        };

        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("WHERE person_id IN (?, ?)"), "account arm");
        assert!(
            sql.contains(
                "AND evidence.entity_id IN (SELECT email FROM identity.person_map WHERE person_id IN (?, ?))"
            ),
            "email fallback arm"
        );
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
                    alias_collapse: AliasCollapse::Sum,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation::undeclared(
                        EvidenceGranularity::SourceSummary,
                    ),
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    alias_collapse: AliasCollapse::Sum,
                    measure_key: denominator.measure_key,
                    presentation: EvidencePresentation::undeclared(
                        EvidenceGranularity::SourceSummary,
                    ),
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
                &TEST_PERSON.to_string(), // the person, once per resolution arm:
                &TEST_PERSON.to_string(), // account binding first, email fallback
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
                presentation: commit_presentation(),
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
            sql.contains(
                "IN (SELECT source_type, source_id, account_id FROM identity.account_assignment WHERE person_id = ?)"
            ),
            "the account binding is consulted first"
        );
        assert!(
            sql.contains(
                "NOT IN (SELECT source_type, source_id, account_id FROM identity.account_assignment)"
            ),
            "a bound account never falls back to the email map — an excluded binding terminates"
        );
        assert!(
            sql.contains("sipHash128(coalesce(evidence.account_source_id, ''))"),
            "the binding key uses identity's minted source id form"
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
                presentation: commit_presentation(),
            }],
        ));
        let (sql, _) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("ORDER BY sort_flag DESC, sort_key DESC, role DESC"));
        assert!(
            sql.contains("ifNull(subject_key, '') DESC, entity_id DESC"),
            "identity is the last ordering column"
        );
    }

    fn commit_plan() -> ValidatedMetricDrilldown {
        validated(plan(
            ComputationSpec::Sum {
                value: input(MetricInputRole::Value, "commit_count"),
            },
            vec![EvidenceInput {
                role: MetricInputRole::Value,
                alias_collapse: AliasCollapse::Sum,
                measure_key: "commit_count".to_owned(),
                presentation: commit_presentation(),
            }],
        ))
    }

    fn roster_plan(ids: &[Uuid]) -> ValidatedMetricDrilldown {
        let mut request = commit_plan();
        request.selection.entity = MetricDrilldownEntity::Persons {
            ids: ids.iter().map(Uuid::to_string).collect(),
        };
        request
    }

    #[test]
    fn a_roster_resolves_who_through_identity_narrowed_to_that_roster() {
        let (sql, params) = compile_query(&roster_plan(&[TEST_PERSON]))
            .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("AS person_id"));
        assert!(
            sql.contains("FROM identity.account_assignment WHERE person_id IN (?)"),
            "the account side reads the roster, not the tenant: {sql}"
        );
        assert!(
            sql.contains("FROM identity.person_map WHERE person_id IN (?)"),
            "the email side reads the roster, not the tenant: {sql}"
        );
        assert_eq!(sql.matches('?').count(), params.len());
    }

    // One person's own drilldown already knows whose records it shows, so it
    // pays for neither the column nor the joins behind it.
    #[test]
    fn one_person_pays_for_no_identity_join() {
        let (sql, params) = compile_query(&commit_plan())
            .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("'' AS person_id"));
        assert!(!sql.contains("AS account_map"));
        assert!(!sql.contains("AS person_map"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn an_ascending_sort_leads_the_key_with_the_emptiness_flag() {
        let mut request = commit_plan();
        request.selection.sort = MetricDrilldownSort {
            key: "ref".to_owned(),
            direction: MetricDrilldownSortDirection::Asc,
        };
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("ORDER BY sort_flag ASC, sort_key ASC, role ASC"));
        assert!(
            sql.contains("evidence.details['ref']"),
            "the order reads the same cell the row projection does: {sql}"
        );
        assert!(
            sql.contains("= '') AS sort_flag"),
            "ascending pushes the blank cells past the filled ones: {sql}"
        );
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn a_descending_page_walks_the_cursor_the_other_way() {
        let mut request = commit_plan();
        request.cursor = Some(CursorKey {
            sort_flag: true,
            sort_value: "2026-07-01".to_owned(),
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

        assert!(sql.contains("< tuple(toUInt8(?)"));
        assert!(params.contains(&"1".to_owned()), "the flag is replayed too");
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn a_search_reads_every_column_the_reader_can_see() {
        let mut request = commit_plan();
        request.selection.search = Some("fix".to_owned());
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert_eq!(
            params.iter().filter(|param| *param == "fix").count(),
            sql.matches("positionCaseInsensitiveUTF8").count(),
            "one bound needle per searched column"
        );
        assert!(sql.contains("evidence.details['title']"));
        assert!(sql.contains("toString(evidence.metric_date)"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    // The reader searches a column they can see. `Who` shows a name and the
    // row carries an identity, so the needle reaches the query as the people
    // it already picked out.
    #[test]
    fn a_search_reaches_the_who_column_through_the_people_it_names() {
        let mut request = roster_plan(&[TEST_PERSON]);
        request.selection.search = Some("ada".to_owned());
        request.search_person_ids = vec![TEST_PERSON.to_string()];
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(
            sql.contains("OR multiIf(coalesce(account_map.account_id"),
            "the needle's people join the same OR chain: {sql}"
        );
        assert_eq!(sql.matches('?').count(), params.len());
    }

    // Nothing to compare against, so nothing is added — and never a bare `IN
    // ()`, which is a syntax error rather than an empty result.
    #[test]
    fn a_search_matching_nobody_adds_no_identity_term() {
        let mut request = roster_plan(&[TEST_PERSON]);
        request.selection.search = Some("nobody".to_owned());
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(!sql.contains("OR multiIf(coalesce(account_map.account_id"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    #[test]
    fn a_ratio_narrows_and_orders_outside_its_aggregation() {
        let mut request = ratio_plan(AliasCollapse::Sum, RatioDenominatorAggregation::Sum);
        request.selection.search = Some("7".to_owned());
        request.selection.sort = MetricDrilldownSort {
            key: "numerator".to_owned(),
            direction: MetricDrilldownSortDirection::Desc,
        };
        let (sql, params) =
            compile_query(&request).unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains(") WHERE (positionCaseInsensitiveUTF8"));
        assert!(sql.contains("ORDER BY sort_flag DESC, sort_key DESC"));
        assert_eq!(sql.matches('?').count(), params.len());
    }

    fn ratio_plan(
        denominator_collapse: AliasCollapse,
        denominator_aggregation: RatioDenominatorAggregation,
    ) -> ValidatedMetricDrilldown {
        let numerator = input(MetricInputRole::Numerator, "commit_count");
        let denominator = input(MetricInputRole::Denominator, "commit_day");
        validated(plan(
            ComputationSpec::Ratio {
                numerator: numerator.clone(),
                denominator: denominator.clone(),
                scale: 1.0,
                denominator_aggregation,
            },
            vec![
                EvidenceInput {
                    role: MetricInputRole::Numerator,
                    alias_collapse: AliasCollapse::Sum,
                    measure_key: numerator.measure_key,
                    presentation: EvidencePresentation::undeclared(
                        EvidenceGranularity::SourceSummary,
                    ),
                },
                EvidenceInput {
                    role: MetricInputRole::Denominator,
                    alias_collapse: denominator_collapse,
                    measure_key: denominator.measure_key,
                    presentation: EvidencePresentation::undeclared(
                        EvidenceGranularity::SourceSummary,
                    ),
                },
            ],
        ))
    }

    #[test]
    fn a_ratio_drilldown_collapses_a_flag_denominator_before_the_daily_rollup() {
        let (sql, params) = compile_query(&ratio_plan(
            AliasCollapse::Max,
            RatioDenominatorAggregation::Sum,
        ))
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
        let (sql, _) = compile_query(&ratio_plan(
            AliasCollapse::Min,
            RatioDenominatorAggregation::Sum,
        ))
        .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("min(ifNull(evidence.contribution, 0))"));
        assert!(!sql.contains("max(ifNull(evidence.contribution, 0))"));
    }

    #[test]
    fn an_all_additive_ratio_drilldown_has_no_collapse_branch() {
        let (sql, _) = compile_query(&ratio_plan(
            AliasCollapse::Sum,
            RatioDenominatorAggregation::Sum,
        ))
        .unwrap_or_else(|error| panic!("query must compile: {error}"));

        // Pinned to the collapse arms, not to `multiIf(` — the resolved-person
        // expression is a multiIf and is present in every ratio drilldown.
        assert!(!sql.contains("max(ifNull(evidence.contribution, 0))"));
        assert!(!sql.contains("min(ifNull(evidence.contribution, 0))"));
        assert!(sql.contains("sum(ifNull(evidence.contribution, 0))"));
    }

    // A roster drilldown collapses each person separately: grouping without the
    // person would take one `max` across the whole team, so a flagged
    // denominator would read 1 for five active people.
    #[test]
    fn the_ratio_collapse_groups_by_the_resolved_person() {
        let (sql, _) = compile_query(&ratio_plan(
            AliasCollapse::Max,
            RatioDenominatorAggregation::Sum,
        ))
        .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(
            sql.contains("LEFT JOIN identity.account_assignment AS account_map"),
            "the collapse resolves the person account-first"
        );
        assert!(
            sql.contains("toJSONString(evidence.dimensions), multiIf(coalesce(account_map"),
            "the resolved person closes the collapse GROUP BY"
        );
    }

    #[test]
    fn ratio_query_uses_distinct_denominator_aggregation() {
        let (sql, params) = compile_query(&ratio_plan(
            AliasCollapse::Max,
            RatioDenominatorAggregation::DistinctCount,
        ))
        .unwrap_or_else(|error| panic!("query must compile: {error}"));

        assert!(sql.contains("uniqExactIf(collapsed.subject_key"));
        assert_eq!(sql.matches('?').count(), params.len());
    }
}
