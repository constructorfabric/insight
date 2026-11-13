use kube::api::DynamicObject;
use sea_orm::DatabaseConnection;

use super::dto::{InheritedSchedule, LiveSchedule, ScheduleResponse};
use super::{Error, Instance, repository, workflow};
use crate::infra::cluster::Cluster;

const UNREADABLE: &str = "the CronWorkflow could not be read back";
const MISMATCH: &str = "the sync job runs a different schedule";

#[derive(Debug, Default)]
pub(crate) struct Observation {
    pub(crate) live: Option<LiveSchedule>,
    pub(crate) inherited: Option<InheritedSchedule>,
    pub(crate) read_error: Option<String>,
}

pub(crate) fn observation(object: Option<&DynamicObject>) -> Result<Observation, Error> {
    Ok(Observation {
        live: object.map(workflow::live).transpose()?,
        inherited: object.and_then(workflow::inherited),
        read_error: None,
    })
}

async fn read(cluster: &Cluster, instance: &Instance) -> Result<Option<DynamicObject>, Error> {
    if !cluster.enabled() {
        return Err(crate::infra::cluster::Error::Disabled.into());
    }
    let object = cluster
        .get(&instance.workflow_name(&cluster.tenant_id)?)
        .await?;
    if object.as_ref().is_some_and(|o| {
        !workflow::identity_matches(o, instance, &cluster.namespace, &cluster.tenant_id)
    }) {
        return Err(Error::ClusterIdentity);
    }
    Ok(object)
}

pub(super) async fn observe(cluster: &Cluster, instance: &Instance) -> Result<Observation, Error> {
    let result = async { observation(read(cluster, instance).await?.as_ref()) }.await;
    Ok(result.unwrap_or_else(|error: Error| Observation {
        read_error: Some(format!("the sync job cannot be read: {error}")),
        ..Observation::default()
    }))
}

pub(super) async fn require_reset_target(
    cluster: &Cluster,
    instance: &Instance,
) -> Result<(), Error> {
    let object = read(cluster, instance)
        .await
        .map_err(|error| Error::NotChanged(error.to_string()))?;
    if object
        .as_ref()
        .is_some_and(|object| workflow::inherited(object).is_none())
    {
        return Err(Error::ResetUnavailable);
    }
    Ok(())
}

pub(super) async fn stored(
    db: &DatabaseConnection,
    cluster: &Cluster,
    instance: &Instance,
) -> Result<ScheduleResponse, Error> {
    let row = repository::read(db, instance).await?;
    let revision = row.as_ref().map_or(0, |r| r.revision);
    let requested = row
        .as_ref()
        .map(|r| serde_json::from_value::<Vec<String>>(r.crons.clone()).map_err(Error::StoredCrons))
        .transpose()?
        .unwrap_or_default();
    let (observed, failure) = attempt(cluster, instance, &requested, revision).await;
    if let Some(reason) = failure {
        return Err(Error::NotChanged(reason));
    }
    super::response(instance, row, observed)
}

#[derive(Debug, Default)]
enum PatchOutcome {
    #[default]
    NotSent,
    Refused,
    Unconfirmed(LiveSchedule),
    Accepted(LiveSchedule),
}

async fn attempt(
    cluster: &Cluster,
    instance: &Instance,
    requested: &[String],
    revision: u64,
) -> (Observation, Option<String>) {
    let mut target = workflow::desired(requested, None);
    let mut outcome = PatchOutcome::NotSent;
    let (observed, failure) = attempt_once(
        cluster,
        instance,
        requested,
        revision,
        &mut target,
        &mut outcome,
    )
    .await;
    if failure.is_none() {
        return (observed, None);
    }
    #[cfg(test)]
    super::save_hooks::checkpoint(instance, "confirm").await;
    let object = read(cluster, instance).await;
    let confirmed = match &object {
        Ok(object) => {
            observation(object.as_ref()).unwrap_or_else(|error| unreadable(&error.to_string()).0)
        }
        Err(error) => unreadable(&error.to_string()).0,
    };
    if target
        .as_ref()
        .zip(confirmed.live.as_ref())
        .is_some_and(|(target, live)| workflow::matches(live, target))
    {
        if let PatchOutcome::Unconfirmed(previous) = &outcome
            && let Some(target) = &target
        {
            log_change(revision, instance, Some(previous), target);
        }
        return (confirmed, None);
    }
    let (PatchOutcome::Unconfirmed(previous) | PatchOutcome::Accepted(previous)) = &outcome else {
        return (observed, failure);
    };
    let Some(live) = confirmed.live.as_ref() else {
        return (confirmed, None);
    };
    let unchanged = live.crons == previous.crons
        && live.timezone == previous.timezone
        && live.origin == previous.origin;
    if !unchanged {
        return (confirmed, None);
    }
    let Ok(Some(object)) = object else {
        return (confirmed, None);
    };
    if fence(cluster, &object).await.is_ok() {
        return (confirmed, failure);
    }
    let observed = observe(cluster, instance).await.unwrap_or_default();
    if let PatchOutcome::Unconfirmed(previous) = &outcome
        && let Some(target) = &target
        && observed
            .live
            .as_ref()
            .is_some_and(|live| workflow::matches(live, target))
    {
        log_change(revision, instance, Some(previous), target);
    }
    (observed, None)
}

async fn fence(cluster: &Cluster, object: &DynamicObject) -> Result<(), Error> {
    let name = object
        .metadata
        .name
        .as_deref()
        .ok_or(Error::ClusterIdentity)?;
    let uid = object
        .metadata
        .uid
        .as_deref()
        .ok_or(Error::ClusterIdentity)?;
    let version = object
        .metadata
        .resource_version
        .as_deref()
        .ok_or(Error::ClusterIdentity)?;
    let patch = serde_json::json!({"metadata": {
        "uid": uid,
        "resourceVersion": version,
        "annotations": {"insight.cyberfabric.com/schedule-fence": uuid::Uuid::new_v4().to_string()}
    }});
    cluster.patch(name, &patch).await?;
    Ok(())
}

async fn attempt_once(
    cluster: &Cluster,
    instance: &Instance,
    requested: &[String],
    revision: u64,
    attempted_target: &mut Option<InheritedSchedule>,
    outcome: &mut PatchOutcome,
) -> (Observation, Option<String>) {
    for retry in [true, false] {
        let object = match read(cluster, instance).await {
            Ok(Some(object)) => object,
            Ok(None) => return (Observation::default(), None),
            Err(error) => return unreadable(&error.to_string()),
        };
        let observed = match observation(Some(&object)) {
            Ok(observed) => observed,
            Err(error) => return unreadable(&error.to_string()),
        };
        let Some(target) = workflow::desired(requested, workflow::inherited(&object)) else {
            return (observed, Some("Reset requires the deployment schedule annotation; run reconcile before resetting".to_owned()));
        };
        *attempted_target = Some(InheritedSchedule {
            crons: target.crons.clone(),
            origin: target.origin,
        });
        if observed
            .live
            .as_ref()
            .is_some_and(|l| workflow::matches(l, &target))
        {
            return (observed, None);
        }
        let patch = match workflow::patch(&object, &target) {
            Ok(patch) => patch,
            Err(error) => {
                return (
                    observed,
                    Some(format!("the cluster refused the change: {error}")),
                );
            }
        };
        let name = object.metadata.name.as_deref().unwrap_or_default();
        let Some(previous) = observed.live else {
            return unreadable(UNREADABLE);
        };
        #[cfg(test)]
        super::save_hooks::checkpoint(instance, "patch").await;
        match cluster.patch(name, &patch).await {
            Err(error) if retry && error.conflict() => {
                *outcome = PatchOutcome::Refused;
                continue;
            }
            Err(error) => {
                tracing::error!(error = %error, "schedule patch failed");
                let message = if error.refused() {
                    *outcome = PatchOutcome::Refused;
                    format!("the cluster refused the change: {error}")
                } else if error.server_error() {
                    format!("the cluster returned an error: {error}")
                } else {
                    format!("the cluster did not answer in time or its answer was lost: {error}")
                };
                if !error.refused() {
                    *outcome = PatchOutcome::Unconfirmed(previous);
                }
                let observed = observation(Some(&object)).unwrap_or_default();
                return (observed, Some(message));
            }
            Ok(()) => {}
        }
        #[cfg(test)]
        super::save_hooks::checkpoint(instance, "readback").await;
        log_change(revision, instance, Some(&previous), &target);
        *outcome = PatchOutcome::Accepted(previous);
        return match read(cluster, instance).await {
            Ok(Some(readback)) if readback.metadata.uid == object.metadata.uid => {
                match observation(Some(&readback)) {
                    Ok(observation) => {
                        let matched = observation
                            .live
                            .as_ref()
                            .is_some_and(|l| workflow::matches(l, &target));
                        (observation, (!matched).then(|| MISMATCH.to_owned()))
                    }
                    Err(error) => unreadable(&error.to_string()),
                }
            }
            Ok(_) => unreadable(UNREADABLE),
            Err(error) => unreadable(&error.to_string()),
        };
    }
    unreadable(UNREADABLE)
}

fn unreadable(message: &str) -> (Observation, Option<String>) {
    let message = format!("the sync job cannot be read: {message}");
    (
        Observation {
            read_error: Some(message.clone()),
            ..Observation::default()
        },
        Some(message),
    )
}

fn log_change(
    revision: u64,
    instance: &Instance,
    before: Option<&LiveSchedule>,
    target: &InheritedSchedule,
) {
    let Some(before) = before else {
        return;
    };
    tracing::info!(connector = instance.connector.as_str(), source_id = instance.source_id.as_str(), from_crons = ?before.crons, to_crons = ?target.crons, revision, "sync job schedule changed");
}

#[cfg(test)]
mod tests;
