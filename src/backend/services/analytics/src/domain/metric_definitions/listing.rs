//! Wire shape and read path for `GET /v1/metric-definitions`.
//!
//! Read-only display listing of the unified metric definitions: every
//! definition visible to the request tenant (product rows plus tenant
//! overrides, tenant row winning per `metric_key`), regardless of
//! `is_enabled` / schema state — the listing doubles as a health surface,
//! so availability is reported (`is_enabled`, `schema_status`) rather than
//! filtered. Computation internals (inputs, computation type, transform)
//! stay off the wire: consumers get the meaning of a metric, not its
//! implementation.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, Days, NaiveDate};
use sea_orm::{DatabaseConnection, FromQueryResult, Statement, Value};
use serde::Serialize;
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::domain::metric_definitions::builtin::{
    EntityType, RevisionRule, builtin_metrics, builtin_sources,
};
use crate::domain::metric_definitions::definition::{MetricDirection, MetricFormat, MetricOrigin};
use crate::domain::metric_definitions::error_code::{MetricSchemaErrorCode, SchemaStatus};
use crate::domain::metric_definitions::repository::{fetch_dimensions, fetch_tags};
use crate::domain::metric_drilldown::{MetricDrilldownCapability, load_capabilities};

/// Response body for `GET /v1/metric-definitions`. Metrics are sorted by
/// `metric_key` ascending so the payload is byte-stable for caching and
/// diff tooling.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDefinitionListResponse {
    pub metrics: Vec<MetricDefinitionView>,
}

/// One metric definition, display fields only.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct MetricDefinitionView {
    pub metric_key: String,
    pub entity_type: EntityType,
    pub label: String,
    /// Compact label for dense surfaces; absent when the full label is
    /// already compact enough.
    pub short_label: Option<String>,
    /// The single topic this metric belongs to within its family, so a surface
    /// listing a family can partition it into topics rather than only sorting
    /// by name. Exactly one per metric; absent only for metrics that declare
    /// none.
    pub subject: Option<String>,
    pub description: Option<String>,
    pub explanation: Option<String>,
    pub unit: Option<String>,
    pub format: MetricFormat,
    pub direction: MetricDirection,
    pub dimensions: Vec<String>,
    /// Cross-cutting labels a surface can filter or search by; many per metric,
    /// unlike the singular `subject`. Empty when the metric declares none.
    pub tags: Vec<String>,
    pub is_enabled: bool,
    /// `builtin` metrics read managed observation relations; `custom` metrics
    /// execute inline SQL at query time. The validator stamps `schema_status`
    /// and `last_observed_date` from materialized relations only, so for
    /// `custom` those fields stay `unchecked` / absent regardless of data —
    /// readers must not interpret them as "never measured" for custom metrics.
    pub origin: MetricOrigin,
    pub schema_status: SchemaStatus,
    /// Why `schema_status` is `error`; absent otherwise (the DB enforces the
    /// biconditional).
    pub schema_error_code: Option<MetricSchemaErrorCode>,
    /// Oldest `metric_date` the definition's input measures currently hold;
    /// absent when they hold none — whether nothing was ever observed or
    /// retention took the last of it, which a sweep cannot tell apart.
    ///
    /// The oldest observation still available, NOT the date collection began:
    /// it moves forward as retention drops the oldest rows, and is cleared when
    /// a sweep reads the relation and finds nothing. It does not pair with
    /// `last_observed_date` as an interval of what is readable — that one is a
    /// high-water mark and is never cleared.
    pub first_observed_date: Option<chrono::NaiveDate>,
    /// Newest `metric_date` ever observed across the definition's input
    /// measures; absent when no observation has ever been seen. Freshness
    /// signal, orthogonal to `schema_status`. Not maintained for `custom`
    /// metrics (see `origin`).
    pub last_observed_date: Option<chrono::NaiveDate>,
    /// Newest delivered date whose reading can no longer change. Absent where
    /// the source declares no revision rule, and for `custom` metrics, which
    /// read no managed source — absence means "settles on arrival", not
    /// "revised forever".
    ///
    /// A date rather than the rule that produced it: how far back revision
    /// reaches depends on the rule's own anchor, and a consumer holding a day
    /// count has to re-derive the anchor to use it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_through: Option<chrono::NaiveDate>,
    /// Deprecated, kept for consumers written before `settled_through`: how many
    /// days back from `last_observed_date` a reading may still be revised.
    ///
    /// A duration cannot express a boundary anchored to the billing month, so
    /// for a month-anchored measure this is the longest that boundary can ever
    /// be — an over-statement, never an under-statement. Absence still means
    /// "settles on arrival", which is why a metric under any rule reports a
    /// number here rather than omitting one it cannot state exactly. Read
    /// `settled_through` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_window_days: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drilldown: Option<MetricDrilldownCapability>,
}

impl toolkit::api::api_dto::ResponseApiDto for MetricDefinitionListResponse {}

#[derive(Debug, FromQueryResult)]
struct ListingRow {
    definition_id: Uuid,
    tenant_id: Option<Uuid>,
    metric_key: String,
    entity_type: String,
    label: String,
    short_label: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    explanation: Option<String>,
    unit: Option<String>,
    format: String,
    direction: String,
    is_enabled: bool,
    origin: String,
    schema_status: String,
    schema_error_code: Option<String>,
    first_observed_date: Option<chrono::NaiveDate>,
    last_observed_date: Option<chrono::NaiveDate>,
}

pub async fn list_definition_views(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    tenant_metrics_enabled: bool,
) -> Result<MetricDefinitionListResponse, CanonicalError> {
    let rows = fetch_listing_rows(db, tenant_id)
        .await
        .map_err(|error| db_error(&error))?;
    let selected = select_rows(rows, tenant_metrics_enabled);
    let metric_keys = selected
        .iter()
        .map(|row| row.metric_key.clone())
        .collect::<Vec<_>>();
    let mut capabilities = match load_capabilities(db, tenant_id, &metric_keys).await {
        Ok(capabilities) => capabilities,
        Err(error) => {
            tracing::warn!(error = ?error, "metric drilldown capability load failed");
            HashMap::new()
        }
    };

    let definition_ids = selected
        .iter()
        .map(|row| row.definition_id)
        .collect::<Vec<_>>();
    let dimensions = fetch_dimensions(db, &definition_ids)
        .await
        .map_err(|error| db_error(&error))?;
    let tags = fetch_tags(db, &definition_ids)
        .await
        .map_err(|error| db_error(&error))?;

    let mut metrics = build_views(selected, dimensions, tags)?;
    for metric in &mut metrics {
        metric.drilldown = capabilities.remove(&metric.metric_key);
    }
    Ok(MetricDefinitionListResponse { metrics })
}

/// Collapse the tenant + product rows per `metric_key` to the one that wins:
/// a tenant-scoped row overrides the product default. Input order is
/// irrelevant; output is sorted by `metric_key` (`BTreeMap` key order).
fn select_rows(rows: Vec<ListingRow>, tenant_metrics_enabled: bool) -> Vec<ListingRow> {
    let mut grouped: BTreeMap<String, Vec<ListingRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.metric_key.clone()).or_default().push(row);
    }
    let mut selected = Vec::with_capacity(grouped.len());
    for (_, mut candidates) in grouped {
        // Tenant override (tenant_id = Some) sorts before the product default.
        candidates.sort_by_key(|row| row.tenant_id.is_none());
        let row = candidates.remove(0);
        if row.entity_type != "tenant" || tenant_metrics_enabled {
            selected.push(row);
        }
    }
    selected
}

/// Map selected rows to wire views, attaching each row's dimensions and
/// decoding its enum columns. Errors on a row whose stored enum value is not
/// canonical (a corrupt-config invariant, not reachable via the write path).
fn build_views(
    selected: Vec<ListingRow>,
    mut dimensions: HashMap<Uuid, Vec<String>>,
    mut tags: HashMap<Uuid, Vec<String>>,
) -> Result<Vec<MetricDefinitionView>, CanonicalError> {
    let mut metrics = Vec::with_capacity(selected.len());
    let revision_rules = revision_rules_by_metric();
    for row in selected {
        let format = MetricFormat::from_db(&row.format)
            .ok_or_else(|| config_error(&row.metric_key, "format", &row.format))?;
        let entity_type = EntityType::from_db(&row.entity_type)
            .ok_or_else(|| config_error(&row.metric_key, "entity_type", &row.entity_type))?;
        let direction = MetricDirection::from_db(&row.direction)
            .ok_or_else(|| config_error(&row.metric_key, "direction", &row.direction))?;
        let origin = MetricOrigin::from_db(&row.origin)
            .ok_or_else(|| config_error(&row.metric_key, "origin", &row.origin))?;
        let schema_status = SchemaStatus::from_db(&row.schema_status)
            .ok_or_else(|| config_error(&row.metric_key, "schema_status", &row.schema_status))?;
        let schema_error_code = row
            .schema_error_code
            .as_deref()
            .map(|code| {
                MetricSchemaErrorCode::from_db(code)
                    .ok_or_else(|| config_error(&row.metric_key, "schema_error_code", code))
            })
            .transpose()?;
        // INVARIANT: keyed by metric_key AND gated on origin. `metric_key` is
        // unique per tenant, not globally, so a custom definition may carry a
        // builtin's key and win the listing over it (`select_rows`). It reads
        // its own SQL and no managed source, so the source's revision rule is
        // not its to report.
        let rules = match origin {
            MetricOrigin::Builtin => revision_rules.get(row.metric_key.as_str()),
            MetricOrigin::Custom => None,
        };
        let settled_through =
            rules.and_then(|rules| settled_through_all(rules, row.last_observed_date));
        let revision_window_days = rules.and_then(|rules| legacy_window_all(rules));
        metrics.push(MetricDefinitionView {
            metric_key: row.metric_key,
            entity_type,
            label: row.label,
            short_label: row.short_label,
            subject: row.subject,
            description: row.description,
            explanation: row.explanation,
            unit: row.unit,
            format,
            direction,
            dimensions: dimensions.remove(&row.definition_id).unwrap_or_default(),
            tags: tags.remove(&row.definition_id).unwrap_or_default(),
            is_enabled: row.is_enabled,
            origin,
            schema_status,
            schema_error_code,
            first_observed_date: row.first_observed_date,
            last_observed_date: row.last_observed_date,
            settled_through,
            revision_window_days,
            drilldown: None,
        });
    }
    Ok(metrics)
}

/// The revision rules a builtin metric reads through, one per input measure.
///
/// The rule belongs to the supplier, not to the tenant, so it comes from the
/// seed rather than from `metric_definitions` — a stored copy would be a second
/// truth to keep in step with the registry. A custom metric reads no managed
/// source and so appears here for no key.
///
/// Per measure rather than per source, because one supplier can report on two
/// date anchors: `ai_cost` stamps its month-to-date totals at the month they
/// bill for, which every later reading rewrites, and its per-day steps at the
/// day they were read, which the next reading does not touch. A measure with no
/// rule of its own inherits the source's, and a metric whose measures have none
/// between them appears here not at all.
fn revision_rules_by_metric() -> HashMap<&'static str, Vec<RevisionRule>> {
    let by_measure: HashMap<(&str, &str), RevisionRule> = builtin_sources()
        .iter()
        .flat_map(|source| {
            let key = source.source.key.as_str();
            let inherited = source.source.revision;
            source.measures.iter().filter_map(move |measure| {
                measure
                    .revision
                    .or(inherited)
                    .map(|rule| ((key, measure.key.as_str()), rule))
            })
        })
        .collect();
    builtin_metrics()
        .iter()
        .filter_map(|metric| {
            let source = metric.source_key.as_str();
            let rules = metric
                .inputs
                .iter()
                .filter_map(|input| {
                    by_measure
                        .get(&(source, input.measure_key.as_str()))
                        .copied()
                })
                .collect::<Vec<_>>();
            (!rules.is_empty()).then(|| (metric.metric_key.as_str(), rules))
        })
        .collect()
}

/// The newest date every one of a metric's rules calls final.
///
/// The earliest of them: a metric is only as settled as its least settled
/// input, and a ratio whose denominator is still moving has not settled because
/// its numerator has.
fn settled_through_all(
    rules: &[RevisionRule],
    last_observed: Option<NaiveDate>,
) -> Option<NaiveDate> {
    rules
        .iter()
        .map(|rule| settled_through(*rule, last_observed))
        .min()
        .flatten()
}

/// The legacy window wide enough for every one of a metric's rules.
fn legacy_window_all(rules: &[RevisionRule]) -> Option<u16> {
    rules
        .iter()
        .map(|rule| legacy_revision_window_days(*rule))
        .max()
}

/// The longest a month can be, and so the longest a billing-month boundary can
/// hold a day open. What the `ai_cost` source declared before the rule existed.
const LONGEST_MONTH_DAYS: u16 = 31;

/// The rule as the pre-`settled_through` wire field could express it.
///
/// A billing-month boundary is not a duration, so it collapses to the widest
/// one it can ever take. That over-states, which is the safe direction: a
/// consumer that reads this draws a settled day as provisional, where omitting
/// the field would have it draw an open month as final.
fn legacy_revision_window_days(rule: RevisionRule) -> u16 {
    match rule {
        RevisionRule::RollingDays(days) => days,
        RevisionRule::BillingMonth => LONGEST_MONTH_DAYS,
    }
}

/// The newest date the rule declares final, given the newest date delivered.
///
/// Nothing is settled before anything has been delivered, so a metric with no
/// observation gets no boundary rather than one in the distant past.
fn settled_through(rule: RevisionRule, last_observed: Option<NaiveDate>) -> Option<NaiveDate> {
    let last_observed = last_observed?;
    match rule {
        RevisionRule::RollingDays(days) => {
            last_observed.checked_sub_days(Days::new(u64::from(days)))
        }
        // The day before the reported month began. A reading always belongs to
        // the month it reports, so every earlier month has closed and no
        // reading of the reported one is final yet — not even one already
        // superseded, since a later reading can lower it again.
        RevisionRule::BillingMonth => last_observed.with_day(1).and_then(|first| first.pred_opt()),
    }
}

async fn fetch_listing_rows(
    db: &DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<ListingRow>, sea_orm::DbErr> {
    ListingRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        "SELECT \
            d.id AS definition_id, \
            d.tenant_id AS tenant_id, \
            d.metric_key AS metric_key, \
            d.entity_type AS entity_type, \
            d.label AS label, \
            d.short_label AS short_label, \
            d.subject AS subject, \
            d.description AS description, \
            d.explanation AS explanation, \
            d.unit AS unit, \
            d.format AS format, \
            d.direction AS direction, \
            d.is_enabled AS is_enabled, \
            d.origin AS origin, \
            d.schema_status AS schema_status, \
            d.schema_error_code AS schema_error_code, \
            d.first_observed_date AS first_observed_date, \
            d.last_observed_date AS last_observed_date \
         FROM metric_definitions d \
         WHERE d.tenant_id IS NULL OR d.tenant_id = ? \
         ORDER BY d.metric_key",
        [Value::Bytes(Some(tenant_id.as_bytes().to_vec()))],
    ))
    .all(db)
    .await
}

fn db_error(error: &sea_orm::DbErr) -> CanonicalError {
    tracing::error!(error = %error, "metric definition listing query failed");
    CanonicalError::internal("failed to list metric definitions").create()
}

fn config_error(metric_key: &str, field: &str, value: &str) -> CanonicalError {
    tracing::error!(
        metric_key = %metric_key,
        field = %field,
        value = %value,
        "corrupt metric definition row"
    );
    CanonicalError::internal("corrupt metric definition configuration").create()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(metric_key: &str, tenant_id: Option<Uuid>, label: &str) -> ListingRow {
        ListingRow {
            definition_id: Uuid::now_v7(),
            tenant_id,
            metric_key: metric_key.to_owned(),
            entity_type: "person".to_owned(),
            label: label.to_owned(),
            short_label: None,
            subject: None,
            description: None,
            explanation: None,
            unit: None,
            format: "integer".to_owned(),
            direction: "higher_is_better".to_owned(),
            is_enabled: true,
            origin: "builtin".to_owned(),
            schema_status: "unchecked".to_owned(),
            schema_error_code: None,
            first_observed_date: None,
            last_observed_date: None,
        }
    }

    #[test]
    fn select_rows_prefers_tenant_override_and_sorts_by_key() {
        let tenant = Uuid::now_v7();
        let rows = vec![
            row("git.commits", None, "product"),
            row("git.commits", Some(tenant), "override"),
            row("ai.cost", None, "product-ai"),
        ];
        let selected = select_rows(rows, false);
        assert_eq!(
            selected
                .iter()
                .map(|r| r.metric_key.as_str())
                .collect::<Vec<_>>(),
            vec!["ai.cost", "git.commits"]
        );
        let Some(commits) = selected.iter().find(|r| r.metric_key == "git.commits") else {
            panic!("git.commits must be selected");
        };
        assert_eq!(commits.label, "override");
    }

    #[test]
    fn tenant_definitions_follow_the_installation_gate() {
        let tenant_metric = || {
            let mut metric = row("ci.runs", None, "CI runs");
            metric.entity_type = "tenant".to_owned();
            metric
        };

        assert!(select_rows(vec![tenant_metric()], false).is_empty());
        assert_eq!(select_rows(vec![tenant_metric()], true).len(), 1);
    }

    fn date(iso: &str) -> NaiveDate {
        let Ok(date) = iso.parse::<NaiveDate>() else {
            panic!("test date must parse: {iso}");
        };
        date
    }

    #[test]
    fn a_rolling_rule_settles_a_fixed_distance_behind_the_newest_delivered_day() {
        assert_eq!(
            settled_through(RevisionRule::RollingDays(3), Some(date("2026-03-10"))),
            Some(date("2026-03-07"))
        );
        // Crossing a month boundary is arithmetic, not a special case.
        assert_eq!(
            settled_through(RevisionRule::RollingDays(3), Some(date("2026-03-02"))),
            Some(date("2026-02-27"))
        );
    }

    #[test]
    fn a_billing_month_rule_settles_at_the_end_of_the_month_before_the_reported_one() {
        // Mid-month: the whole reported month stays open, whatever the day.
        assert_eq!(
            settled_through(RevisionRule::BillingMonth, Some(date("2026-03-10"))),
            Some(date("2026-02-28"))
        );
        // The last day of a month does not close it — a later reading inside
        // the same month can still lower every day of it.
        assert_eq!(
            settled_through(RevisionRule::BillingMonth, Some(date("2026-03-31"))),
            Some(date("2026-02-28"))
        );
        // The first day of a month settles every earlier month at once.
        assert_eq!(
            settled_through(RevisionRule::BillingMonth, Some(date("2026-01-01"))),
            Some(date("2025-12-31"))
        );
    }

    #[test]
    fn a_custom_definition_reports_no_revision_metadata_under_a_builtin_key() {
        // metric_key is unique per tenant, so a tenant's custom definition can
        // carry a builtin AI key and override it in the listing. It executes
        // its own SQL and never reads `ai_cost`, so neither the boundary nor
        // the legacy window may be attributed to it.
        let key = "ai.daily_approximate_extra_usage_cost";
        let mut custom = row(key, Some(Uuid::now_v7()), "Tenant's own");
        custom.origin = "custom".to_owned();
        custom.last_observed_date = Some(date("2026-03-10"));

        let Ok(views) = build_views(vec![custom], HashMap::new(), HashMap::new()) else {
            panic!("canonical rows must map");
        };
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.settled_through, None);
        assert_eq!(view.revision_window_days, None);

        // The product row under the same key still reports both.
        let mut builtin = row(key, None, "AI actual usage cost — approximate distribution");
        builtin.last_observed_date = Some(date("2026-03-10"));
        let Ok(views) = build_views(vec![builtin], HashMap::new(), HashMap::new()) else {
            panic!("canonical rows must map");
        };
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.settled_through, Some(date("2026-03-10")));
        assert_eq!(view.revision_window_days, Some(0));
    }

    #[test]
    fn the_legacy_window_over_states_a_billing_month_rather_than_going_absent() {
        // Absence means "settles on arrival" to a consumer written before
        // `settled_through`, so a month-anchored source must report a number it
        // can never fall short of rather than report nothing.
        assert_eq!(
            legacy_revision_window_days(RevisionRule::BillingMonth),
            LONGEST_MONTH_DAYS
        );
        assert_eq!(legacy_revision_window_days(RevisionRule::RollingDays(3)), 3);
    }

    #[test]
    fn nothing_is_settled_before_anything_is_delivered() {
        assert_eq!(settled_through(RevisionRule::RollingDays(3), None), None);
        assert_eq!(settled_through(RevisionRule::BillingMonth, None), None);
    }

    #[test]
    fn ai_cost_month_facts_settle_by_month_and_its_daily_step_on_arrival() {
        let rules = revision_rules_by_metric();

        // The step between two readings is fixed once the later one lands, so
        // it does not wait for the month to close the way the month-to-date
        // figures beside it do.
        assert_eq!(
            rules.get("ai.daily_approximate_extra_usage_cost"),
            Some(&vec![RevisionRule::RollingDays(0)])
        );
        assert_eq!(
            rules.get("ai.extra_usage_cost"),
            Some(&vec![RevisionRule::BillingMonth])
        );
        assert_eq!(
            rules.get("ai.seat_cost"),
            Some(&vec![RevisionRule::BillingMonth])
        );
        // A ratio carries one rule per side; both of these are month-anchored.
        assert_eq!(
            rules.get("ai.extra_usage_utilisation"),
            Some(&vec![
                RevisionRule::BillingMonth,
                RevisionRule::BillingMonth
            ])
        );
        // A measure with no rule of its own inherits the source's.
        assert_eq!(
            rules.get("ai.accepted_lines"),
            Some(&vec![RevisionRule::RollingDays(3)])
        );
        // A source that declares no rule leaves its metrics without one.
        assert_eq!(rules.get("git.commits"), None);
    }

    #[test]
    fn a_metric_is_only_as_settled_as_its_least_settled_input() {
        let last = Some(date("2026-03-10"));
        // The earlier boundary wins, whichever side it came from.
        assert_eq!(
            settled_through_all(
                &[RevisionRule::RollingDays(0), RevisionRule::BillingMonth],
                last
            ),
            Some(date("2026-02-28"))
        );
        assert_eq!(
            settled_through_all(&[RevisionRule::RollingDays(0)], last),
            Some(date("2026-03-10"))
        );
        // And the legacy window is wide enough for every rule.
        assert_eq!(
            legacy_window_all(&[RevisionRule::RollingDays(0), RevisionRule::BillingMonth]),
            Some(LONGEST_MONTH_DAYS)
        );
        assert_eq!(legacy_window_all(&[RevisionRule::RollingDays(0)]), Some(0));
    }

    #[test]
    fn a_day_read_is_a_day_settled_for_the_daily_distribution() {
        // The metric this whole boundary exists for: with the step anchored to
        // the day it was read, nothing already delivered is left provisional.
        let mut r = row(
            "ai.daily_approximate_extra_usage_cost",
            None,
            "AI actual usage cost — approximate distribution",
        );
        r.last_observed_date = Some(date("2026-03-10"));

        let Ok(views) = build_views(vec![r], HashMap::new(), HashMap::new()) else {
            panic!("canonical rows must map");
        };
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.settled_through, Some(date("2026-03-10")));
        assert_eq!(view.revision_window_days, Some(0));
    }

    #[test]
    fn build_views_decodes_columns_and_attaches_dimensions() {
        let mut r = row("git.commits", None, "Commits");
        r.subject = Some("commits".to_owned());
        r.schema_status = "error".to_owned();
        r.schema_error_code = Some("table_not_found".to_owned());
        let id = r.definition_id;
        let dims = HashMap::from([(id, vec!["repo".to_owned()])]);
        let tags = HashMap::from([(id, vec!["rate".to_owned()])]);

        let Ok(views) = build_views(vec![r], dims, tags) else {
            panic!("canonical rows must map");
        };
        assert_eq!(views.len(), 1);
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.format, MetricFormat::Integer);
        assert_eq!(view.entity_type, EntityType::Person);
        assert_eq!(view.direction, MetricDirection::HigherIsBetter);
        assert_eq!(view.origin, MetricOrigin::Builtin);
        assert_eq!(view.schema_status, SchemaStatus::Error);
        assert_eq!(
            view.schema_error_code,
            Some(MetricSchemaErrorCode::TableNotFound)
        );
        assert_eq!(view.dimensions, vec!["repo".to_owned()]);
        assert_eq!(view.subject.as_deref(), Some("commits"));
        assert_eq!(view.tags, vec!["rate".to_owned()]);
    }

    #[test]
    fn build_views_rejects_unknown_entity_types() {
        let mut r = row("git.commits", None, "Commits");
        r.entity_type = "repository".to_owned();

        assert!(build_views(vec![r], HashMap::new(), HashMap::new()).is_err());
    }

    #[test]
    fn build_views_decodes_custom_origin() {
        let mut r = row("team.velocity", None, "Velocity");
        r.origin = "custom".to_owned();

        let Ok(views) = build_views(vec![r], HashMap::new(), HashMap::new()) else {
            panic!("canonical rows must map");
        };
        let Some(view) = views.first() else {
            panic!("one view");
        };
        assert_eq!(view.origin, MetricOrigin::Custom);
        assert_eq!(view.schema_status, SchemaStatus::Unchecked);
        assert_eq!(view.schema_error_code, None);
        assert_eq!(view.last_observed_date, None);
        assert_eq!(view.subject, None);
        assert!(view.tags.is_empty());
    }

    #[test]
    fn build_views_rejects_a_noncanonical_enum_value() {
        let mut r = row("git.commits", None, "Commits");
        r.format = "not-a-format".to_owned();
        assert!(build_views(vec![r], HashMap::new(), HashMap::new()).is_err());

        let mut r = row("git.commits", None, "Commits");
        r.origin = "not-an-origin".to_owned();
        assert!(build_views(vec![r], HashMap::new(), HashMap::new()).is_err());
    }
}
