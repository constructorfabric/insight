use std::collections::{BTreeMap, HashMap};

use sea_orm::{DatabaseConnection, FromQueryResult, Statement, Value};
use toolkit_canonical_errors::CanonicalError;
use uuid::Uuid;

use crate::domain::metric_definitions::definition::MetricInputRole;
use crate::domain::metric_definitions::{EvidenceGranularity, EvidenceRelation};

use super::dto::MetricDrilldownCapability;
use super::error::db_error;

#[derive(Debug, FromQueryResult)]
struct CapabilityRow {
    metric_key: String,
    input_role: String,
    evidence_granularity: Option<String>,
    source_key: String,
    evidence_ref: Option<String>,
    evidence_schema_status: String,
}

#[derive(Debug, FromQueryResult)]
pub(super) struct EvidenceInputRow {
    pub(super) input_role: String,
    pub(super) measure_key: String,
    pub(super) alias_collapse: String,
    pub(super) evidence_granularity: Option<String>,
    pub(super) evidence_presentation: Option<String>,
    pub(super) source_key: String,
    pub(super) evidence_ref: Option<String>,
    pub(super) evidence_schema_status: String,
}

pub async fn load_capabilities(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    metric_keys: &[String],
) -> Result<HashMap<String, MetricDrilldownCapability>, CanonicalError> {
    if metric_keys.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; metric_keys.len()].join(", ");
    let sql = format!(
        "SELECT d.metric_key, i.input_role, m.evidence_granularity, s.source_key, \
                s.evidence_ref, s.evidence_schema_status \
         FROM metric_definitions d \
         INNER JOIN metric_definition_inputs i ON i.metric_definition_id = d.id \
         INNER JOIN metric_source_measures m ON m.id = i.source_measure_id \
         INNER JOIN metric_sources s ON s.id = m.source_id \
         WHERE d.metric_key IN ({placeholders}) \
           AND d.id = COALESCE( \
               (SELECT td.id FROM metric_definitions td WHERE td.metric_key = d.metric_key AND td.tenant_id = ? LIMIT 1), \
               (SELECT pd.id FROM metric_definitions pd WHERE pd.metric_key = d.metric_key AND pd.tenant_id IS NULL LIMIT 1) \
           ) \
           AND d.is_enabled = TRUE AND d.schema_status = 'ok' \
           AND m.is_enabled = TRUE AND s.is_enabled = TRUE \
         ORDER BY d.metric_key, i.input_role, m.measure_key"
    );
    let mut values = metric_keys.iter().map(Value::from).collect::<Vec<_>>();
    values.push(Value::Bytes(Some(tenant_id.as_bytes().to_vec())));
    let rows = CapabilityRow::find_by_statement(Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        values,
    ))
    .all(db)
    .await
    .map_err(|error| db_error(&error))?;
    let mut grouped: BTreeMap<String, Vec<CapabilityRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.metric_key.clone()).or_default().push(row);
    }

    Ok(grouped
        .into_iter()
        .filter_map(|(metric_key, rows)| {
            capability_from_rows(&rows).map(|capability| (metric_key, capability))
        })
        .collect())
}

fn capability_from_rows(rows: &[CapabilityRow]) -> Option<MetricDrilldownCapability> {
    let (_, _) = healthy_evidence(rows.iter().map(CapabilityRow::health))?;

    let mut granularity = rows
        .iter()
        .filter_map(|row| {
            row.evidence_granularity
                .as_deref()
                .and_then(EvidenceGranularity::from_db)
        })
        .collect::<Vec<_>>();
    granularity.sort_by_key(|value| value.as_db());
    granularity.dedup();

    Some(MetricDrilldownCapability { granularity })
}

pub(super) struct EvidenceHealthView<'a> {
    input_role: &'a str,
    evidence_granularity: Option<&'a str>,
    source_key: &'a str,
    evidence_ref: Option<&'a str>,
    evidence_schema_status: &'a str,
}

impl CapabilityRow {
    fn health(&self) -> EvidenceHealthView<'_> {
        EvidenceHealthView {
            input_role: &self.input_role,
            evidence_granularity: self.evidence_granularity.as_deref(),
            source_key: &self.source_key,
            evidence_ref: self.evidence_ref.as_deref(),
            evidence_schema_status: &self.evidence_schema_status,
        }
    }
}

impl EvidenceInputRow {
    pub(super) fn health(&self) -> EvidenceHealthView<'_> {
        EvidenceHealthView {
            input_role: &self.input_role,
            evidence_granularity: self.evidence_granularity.as_deref(),
            source_key: &self.source_key,
            evidence_ref: self.evidence_ref.as_deref(),
            evidence_schema_status: &self.evidence_schema_status,
        }
    }
}

pub(super) fn healthy_evidence<'a>(
    rows: impl IntoIterator<Item = EvidenceHealthView<'a>>,
) -> Option<(EvidenceRelation, &'a str)> {
    let mut rows = rows.into_iter();
    let first = rows.next()?;
    let relation = EvidenceRelation::parse(first.evidence_ref?)?;
    let source_key = first.source_key;

    let row_is_healthy = |row: &EvidenceHealthView<'_>| {
        MetricInputRole::from_db(row.input_role).is_some()
            && row.evidence_schema_status == "ok"
            && row.source_key == source_key
            && row.evidence_ref == Some(relation.source_ref())
            && row
                .evidence_granularity
                .and_then(EvidenceGranularity::from_db)
                .is_some()
    };
    if !row_is_healthy(&first) || !rows.all(|row| row_is_healthy(&row)) {
        return None;
    }

    Some((relation, source_key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_row(status: &str, granularity: Option<&str>) -> CapabilityRow {
        CapabilityRow {
            metric_key: "ai.accepted_lines".to_owned(),
            input_role: "value".to_owned(),
            evidence_granularity: granularity.map(str::to_owned),
            source_key: "ai_usage".to_owned(),
            evidence_ref: Some("ai_metric_evidence".to_owned()),
            evidence_schema_status: status.to_owned(),
        }
    }

    #[test]
    fn healthy_rows_yield_sorted_deduplicated_granularity() {
        let rows = vec![
            capability_row("ok", Some("source_summary")),
            capability_row("ok", Some("event")),
            capability_row("ok", Some("event")),
        ];
        let Some(capability) = capability_from_rows(&rows) else {
            panic!("healthy rows must yield a capability");
        };
        assert_eq!(
            capability
                .granularity
                .iter()
                .map(|value| value.as_db())
                .collect::<Vec<_>>(),
            ["event", "source_summary"]
        );
    }

    #[test]
    fn capability_fails_closed_on_any_unhealthy_input() {
        let cases = [
            (Vec::new(), "no rows"),
            (vec![capability_row("error", Some("event"))], "probe error"),
            (
                vec![capability_row("unchecked", Some("event"))],
                "unchecked probe",
            ),
            (vec![capability_row("ok", None)], "granularity missing"),
            (
                vec![capability_row("ok", Some("nonsense"))],
                "granularity unknown",
            ),
            (
                vec![
                    capability_row("ok", Some("event")),
                    capability_row("error", Some("event")),
                ],
                "one input unhealthy",
            ),
        ];
        for (rows, case) in cases {
            assert!(
                capability_from_rows(&rows).is_none(),
                "should fail closed: {case}"
            );
        }
    }

    #[test]
    fn inputs_must_share_one_source_and_evidence_relation() {
        let mut other_source = capability_row("ok", Some("event"));
        other_source.source_key = "task".to_owned();
        assert!(
            capability_from_rows(&[capability_row("ok", Some("event")), other_source]).is_none()
        );

        let mut other_relation = capability_row("ok", Some("event"));
        other_relation.evidence_ref = Some("task_metric_evidence".to_owned());
        assert!(
            capability_from_rows(&[capability_row("ok", Some("event")), other_relation]).is_none()
        );
    }

    #[test]
    fn unparseable_evidence_relation_yields_no_capability() {
        let mut invalid = capability_row("ok", Some("event"));
        invalid.evidence_ref = Some("Not A Relation".to_owned());
        assert!(capability_from_rows(&[invalid]).is_none());

        let mut absent = capability_row("ok", Some("event"));
        absent.evidence_ref = None;
        assert!(capability_from_rows(&[absent]).is_none());
    }

    #[test]
    fn unknown_input_role_yields_no_capability() {
        let mut row = capability_row("ok", Some("event"));
        row.input_role = "nonsense".to_owned();
        assert!(capability_from_rows(&[row]).is_none());
    }
}
