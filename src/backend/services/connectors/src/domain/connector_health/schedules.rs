use std::collections::HashMap;

use chrono::{DateTime, Utc};
use kube::api::DynamicObject;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};

use super::model::{AppliedSchedule, ConnectorHealthResponse};
use crate::domain::connector_schedules::dto::{ScheduleStatus, status};
use crate::domain::connector_schedules::{Instance, apply, workflow};
use crate::infra::cluster::Cluster;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct HealthScheduleRequest {
    pub revision: u64,
    pub crons: Vec<String>,
    pub last_observation: Option<ScheduleObservation>,
    pub read_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub(crate) struct ScheduleObservation {
    pub status: ScheduleStatus,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, FromQueryResult)]
struct StoredRequest {
    revision: u64,
    crons: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("schedule database read failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("stored schedule is invalid")]
    Stored(#[from] serde_json::Error),
    #[error("live sync jobs cannot be read: {0}")]
    Cluster(#[from] crate::infra::cluster::Error),
    #[error("live sync job is invalid: {0}")]
    Schedule(#[from] crate::domain::connector_schedules::Error),
}

fn request(
    row: Option<&StoredRequest>,
    object: Option<&DynamicObject>,
) -> Result<(HealthScheduleRequest, Option<AppliedSchedule>), Error> {
    let revision = row.as_ref().map_or(0, |row| row.revision);
    let crons = row
        .as_ref()
        .map(|row| serde_json::from_value::<Vec<String>>(row.crons.clone()))
        .transpose()?
        .unwrap_or_default();
    let observed = apply::observation(object)?;
    let status = status(&crons, observed.live.as_ref(), observed.inherited.as_ref()).0;
    let schedule = observed.live.map(|live| AppliedSchedule {
        crons: live.crons,
        timezone: live.timezone,
        origin: live.origin,
        suspended: live.suspended,
    });
    Ok((
        HealthScheduleRequest {
            revision,
            crons,
            read_error: None,
            last_observation: Some(ScheduleObservation {
                status,
                observed_at: Utc::now(),
            }),
        },
        schedule,
    ))
}

pub(crate) async fn enrich(
    db: &DatabaseConnection,
    cluster: &Cluster,
    response: &mut ConnectorHealthResponse,
) -> Result<(), Error> {
    let mut requests = read(db, response).await?;
    let objects = cluster.list().await?;
    let objects: HashMap<_, _> = objects
        .iter()
        .filter_map(|object| object.metadata.name.as_deref().map(|name| (name, object)))
        .collect();
    for row in &mut response.connectors {
        if !row.configured {
            row.schedule_request = None;
            row.schedule = None;
            continue;
        }
        let Some(source) = row.source_id.as_ref() else {
            continue;
        };
        let stored = requests
            .get_mut(&row.connector)
            .and_then(|requests| requests.remove(source));
        match evaluate(&row.connector, source, stored.as_ref(), &objects, cluster) {
            Ok((request, schedule)) => {
                row.schedule_request = Some(request);
                row.schedule = schedule;
            }
            Err(error) => {
                row.schedule = None;
                row.schedule_request = Some(HealthScheduleRequest {
                    revision: stored.as_ref().map_or(0, |row| row.revision),
                    crons: stored
                        .and_then(|row| serde_json::from_value(row.crons).ok())
                        .unwrap_or_default(),
                    last_observation: None,
                    read_error: Some(error.to_string()),
                });
            }
        }
    }
    Ok(())
}

fn evaluate(
    connector: &str,
    source: &str,
    stored: Option<&StoredRequest>,
    objects: &HashMap<&str, &DynamicObject>,
    cluster: &Cluster,
) -> Result<(HealthScheduleRequest, Option<AppliedSchedule>), Error> {
    let instance = Instance::parse(connector, source)?;
    let name = instance.workflow_name(&cluster.tenant_id)?;
    let object = objects.get(name.as_str()).copied();
    if object.is_some_and(|object| {
        !workflow::identity_matches(object, &instance, &cluster.namespace, &cluster.tenant_id)
    }) {
        return Err(crate::domain::connector_schedules::Error::ClusterIdentity.into());
    }
    request(stored, object)
}

async fn read(
    db: &DatabaseConnection,
    response: &ConnectorHealthResponse,
) -> Result<HashMap<String, HashMap<String, StoredRequest>>, Error> {
    let mut filters = Vec::new();
    let mut values = Vec::new();
    for row in &response.connectors {
        let Some(source) = row.source_id.as_ref() else {
            continue;
        };
        filters.push("(s.connector = ? AND s.source_id = ?)");
        values.push(row.connector.clone().into());
        values.push(source.clone().into());
    }
    if filters.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT s.connector, s.source_id, s.revision, s.crons FROM connector_schedules s WHERE {}",
        filters.join(" OR ")
    );
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::MySql,
            sql,
            values,
        ))
        .await?;
    let mut requests: HashMap<String, HashMap<String, StoredRequest>> = HashMap::new();
    for row in rows {
        let connector = row.try_get("", "connector")?;
        let source_id = row.try_get("", "source_id")?;
        requests
            .entry(connector)
            .or_default()
            .insert(source_id, StoredRequest::from_query_result(&row, "")?);
    }
    Ok(requests)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;
