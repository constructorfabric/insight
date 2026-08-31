//! What answered, stated beside the answer.
//!
//! INVARIANT: the definition version is read from the store, never asserted
//! here, and a store that cannot be read leaves it absent rather than refusing.

use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect};

use crate::infra::db::entities::semantic_metrics;

use super::dto::{Executor, Provenance};

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

pub(super) fn provenance(versions: &BTreeMap<String, i32>, metric_key: &str) -> Provenance {
    Provenance {
        executor: Executor::Semantic,
        definition_version: versions.get(metric_key).copied(),
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
            provenance(&versions, "git.commits"),
            Provenance {
                executor: Executor::Semantic,
                definition_version: Some(3),
            }
        );
    }

    #[test]
    fn a_version_the_store_did_not_answer_with_is_absent_rather_than_invented() {
        let provenance = provenance(&BTreeMap::new(), "git.commits");

        assert_eq!(provenance.definition_version, None);
        assert_eq!(provenance.executor, Executor::Semantic);
    }
}
