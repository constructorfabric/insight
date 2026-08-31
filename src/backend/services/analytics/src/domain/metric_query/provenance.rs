//! What answered, stated beside the answer.
//!
//! INVARIANT: the definition version is read from the store, never asserted
//! here, and a store that cannot be read leaves it absent rather than refusing.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};

use crate::infra::db::entities::semantic_metrics;

use super::dto::{Executor, Provenance, ServedFrom};

/// A key the store carries no row for is absent from the result.
pub(super) async fn metric_versions(
    db: &DatabaseConnection,
    metric_keys: &[String],
) -> BTreeMap<String, i32> {
    if metric_keys.is_empty() {
        return BTreeMap::new();
    }

    let read = semantic_metrics::Entity::find()
        .select_only()
        .column(semantic_metrics::Column::MetricKey)
        .column(semantic_metrics::Column::DefinitionVersion)
        .filter(semantic_metrics::Column::TenantId.is_null())
        .filter(semantic_metrics::Column::MetricKey.is_in(metric_keys.iter().map(String::as_str)))
        .into_tuple::<(String, i32)>()
        .all(db)
        .await;

    match read {
        Ok(rows) => rows.into_iter().collect(),
        Err(error) => {
            tracing::warn!(%error, "the definition versions behind a values answer went unread");
            BTreeMap::new()
        }
    }
}

pub(super) fn provenance(
    versions: &BTreeMap<String, i32>,
    metric_key: &str,
    served_from: ServedFrom,
) -> Provenance {
    Provenance {
        executor: Executor::Semantic,
        definition_version: versions.get(metric_key).copied(),
        served_from,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_metric_the_store_carries_a_row_for_states_its_version() {
        let versions = BTreeMap::from([("git.commits".to_owned(), 3)]);

        assert_eq!(
            provenance(&versions, "git.commits", ServedFrom::Cache),
            Provenance {
                executor: Executor::Semantic,
                definition_version: Some(3),
                served_from: ServedFrom::Cache,
            }
        );
    }

    #[test]
    fn a_version_the_store_did_not_answer_with_is_absent_rather_than_invented() {
        let provenance = provenance(&BTreeMap::new(), "git.commits", ServedFrom::Computed);

        assert_eq!(provenance.definition_version, None);
        assert_eq!(provenance.executor, Executor::Semantic);
    }

    #[test]
    fn an_answer_states_cache_only_when_every_read_behind_it_was_cached() {
        let cases = [
            (vec![true, true], ServedFrom::Cache),
            (vec![true, false], ServedFrom::Mixed),
            (vec![false, false], ServedFrom::Computed),
            (vec![false], ServedFrom::Computed),
            (vec![true], ServedFrom::Cache),
        ];

        for (reads, expected) in cases {
            assert_eq!(ServedFrom::of(reads.clone()), expected, "{reads:?}");
        }
    }
}
