use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use serde::Deserialize;

use crate::domain::metric_definitions::{CohortSource, ObservationSource};

use super::super::validation::{ValidatedMetricRequest, ValidatedMetricView};

/// The cache may never be the reason a request stalls, so a slow epoch lookup
/// degrades to "nothing was cached" like every other failure here.
const EPOCH_TIMEOUT: Duration = Duration::from_millis(500);

/// A warehouse relation a cached fragment was computed from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RelationRef {
    pub database: &'static str,
    pub table: String,
}

/// Current table UUID per relation. dbt rebuilds a gold model as a new table,
/// so the UUID changing is exactly "the data behind this fragment was
/// replaced" — cached keys carrying the old UUID become unreachable.
#[derive(Debug, Default)]
pub struct RelationEpochs(BTreeMap<RelationRef, String>);

impl RelationEpochs {
    pub fn get(&self, relation: &RelationRef) -> Option<&str> {
        self.0.get(relation).map(String::as_str)
    }
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct RelationEpochRow {
    database: String,
    name: String,
    uuid: String,
}

/// Relations every cacheable fragment of this request depends on: each managed
/// observation relation, plus the cohort relation when a peer view is asked
/// for. Custom observation SQL has no relation to pin, so it contributes
/// nothing here and its views never cache.
pub fn required_relations(metrics: &[ValidatedMetricRequest]) -> BTreeSet<RelationRef> {
    let mut relations = BTreeSet::new();
    for metric in metrics {
        for input in metric.def.inputs() {
            if let ObservationSource::Managed(relation) = &input.observation {
                let (database, table) = relation.table_ref();
                relations.insert(RelationRef {
                    database,
                    table: table.to_owned(),
                });
            }
        }

        let wants_cohort = metric
            .views
            .iter()
            .any(|view| matches!(view, ValidatedMetricView::Peer { .. }));
        if wants_cohort {
            let (database, table) = CohortSource::MetricEntityCohortsCurrent.table_ref();
            relations.insert(RelationRef {
                database,
                table: table.to_owned(),
            });
        }
    }
    relations
}

/// Reads every epoch in one query. A relation missing from the reply is left
/// out, which makes its views uncacheable rather than wrong.
pub async fn relation_epochs(
    ch: &insight_clickhouse::Client,
    relations: &BTreeSet<RelationRef>,
) -> Option<RelationEpochs> {
    if relations.is_empty() {
        return Some(RelationEpochs::default());
    }

    let databases: BTreeSet<&str> = relations.iter().map(|r| r.database).collect();
    let tables: BTreeSet<&str> = relations.iter().map(|r| r.table.as_str()).collect();
    let sql = format!(
        "SELECT database, name, toString(uuid) AS uuid \
         FROM system.tables WHERE database IN ({}) AND name IN ({})",
        placeholders(databases.len()),
        placeholders(tables.len()),
    );

    let mut query = ch
        .query(&sql)
        .with_setting("log_comment", "metric-results:epochs");
    for database in &databases {
        query = query.bind(*database);
    }
    for table in &tables {
        query = query.bind(*table);
    }

    let rows =
        match tokio::time::timeout(EPOCH_TIMEOUT, query.fetch_all::<RelationEpochRow>()).await {
            Ok(Ok(rows)) => rows,
            Ok(Err(error)) => {
                tracing::debug!(error = %error, "metric-results cache epoch lookup failed");
                return None;
            }
            Err(_) => {
                tracing::debug!("metric-results cache epoch lookup timed out");
                return None;
            }
        };

    let by_relation = rows
        .into_iter()
        .filter_map(|row| {
            let relation = relations
                .iter()
                .find(|r| r.database == row.database && r.table == row.name)?;
            Some((relation.clone(), row.uuid))
        })
        .collect();

    Some(RelationEpochs(by_relation))
}

fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
impl RelationEpochs {
    pub fn from_pairs(pairs: Vec<(RelationRef, String)>) -> Self {
        Self(pairs.into_iter().collect())
    }
}

#[cfg(test)]
pub fn relation_epochs_for_test(relations: &BTreeSet<RelationRef>, epoch: &str) -> RelationEpochs {
    RelationEpochs(
        relations
            .iter()
            .cloned()
            .map(|relation| (relation, epoch.to_owned()))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metric_results::cache::test_support::{
        custom_sql_metric, metric_request, sum_metric,
    };
    use crate::domain::metric_results::view::Bucket;

    #[test]
    fn managed_observation_and_cohort_relations_are_both_required() {
        let metrics = vec![metric_request(
            sum_metric(),
            vec![
                ValidatedMetricView::Period,
                ValidatedMetricView::Peer {
                    cohort_key: "org_unit".to_owned(),
                },
            ],
        )];

        let relations = required_relations(&metrics);

        let tables: Vec<&str> = relations.iter().map(|r| r.table.as_str()).collect();
        assert_eq!(
            tables,
            vec!["ai_metric_observations", "metric_entity_cohorts_current"]
        );
    }

    #[test]
    fn cohort_relation_is_required_only_for_peer_views() {
        let metrics = vec![metric_request(
            sum_metric(),
            vec![
                ValidatedMetricView::Period,
                ValidatedMetricView::Timeseries {
                    bucket: Bucket::Day,
                    dimensions: Vec::new(),
                    group_limit: None,
                },
            ],
        )];

        let relations = required_relations(&metrics);

        assert_eq!(relations.len(), 1);
    }

    #[test]
    fn custom_observation_sql_contributes_no_relation() {
        let metrics = vec![metric_request(
            custom_sql_metric(),
            vec![ValidatedMetricView::Period],
        )];

        assert!(required_relations(&metrics).is_empty());
    }
}
