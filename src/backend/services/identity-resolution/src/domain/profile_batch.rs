use std::collections::{HashMap, HashSet};

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use super::profile::{BatchProfilesRequest, BatchProfilesResponse, assemble_batch_profile};
use crate::config::VisibilityPolicy;
use crate::infra::db::persons_repo::BatchProfileReadError;
use crate::infra::db::{persons_repo, subchart_repo};

#[derive(Debug, thiserror::Error)]
pub enum BatchProfilesError {
    #[error("profile visibility read failed")]
    Visibility,
    #[error(transparent)]
    ProfileRead(#[from] BatchProfileReadError),
}

pub async fn resolve_batch_profiles(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    caller_id: Uuid,
    request: BatchProfilesRequest,
    org_chart_source_type: &str,
    visibility_policy: VisibilityPolicy,
) -> Result<BatchProfilesResponse, BatchProfilesError> {
    let visible = subchart_repo::visible_targets(
        db,
        tenant_id,
        caller_id,
        &request.person_ids,
        org_chart_source_type,
        visibility_policy,
    )
    .await
    .map_err(|error| {
        tracing::error!(%error, "batch profile visibility read failed");
        BatchProfilesError::Visibility
    })?
    .into_iter()
    .collect::<HashSet<_>>();
    let existing = persons_repo::persons_in_tenant(db, tenant_id, &request.person_ids)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let person_ids = request
        .person_ids
        .into_iter()
        .filter(|person_id| visible.contains(person_id) && existing.contains(person_id))
        .collect::<Vec<_>>();

    let observations =
        persons_repo::current_profile_observations(db, tenant_id, &person_ids).await?;
    let parents = persons_repo::current_parents_for_children(db, tenant_id, &person_ids).await?;
    let mut supervisors = HashMap::new();
    for edge in parents {
        if edge.source_type == org_chart_source_type {
            supervisors
                .entry(edge.child_person_id)
                .or_insert(edge.parent_person_id);
        }
    }

    let supervisor_ids = supervisors.values().copied().collect::<HashSet<_>>();
    let supervisor_ids = supervisor_ids.into_iter().collect::<Vec<_>>();
    let existing_supervisor_ids = persons_repo::persons_in_tenant(db, tenant_id, &supervisor_ids)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    supervisors.retain(|_, supervisor_id| existing_supervisor_ids.contains(supervisor_id));
    let supervisor_ids = supervisors.values().copied().collect::<Vec<_>>();
    let supervisor_observations =
        persons_repo::current_profile_observations(db, tenant_id, &supervisor_ids).await?;

    let profiles = person_ids
        .into_iter()
        .map(|person_id| {
            let supervisor = supervisors.get(&person_id).map(|supervisor_id| {
                (
                    *supervisor_id,
                    supervisor_observations
                        .get(supervisor_id)
                        .cloned()
                        .unwrap_or_default(),
                )
            });
            assemble_batch_profile(
                person_id,
                observations.get(&person_id).cloned().unwrap_or_default(),
                supervisor,
            )
        })
        .collect();

    Ok(BatchProfilesResponse { profiles })
}
