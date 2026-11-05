use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement};

use super::dto::ScheduleRequestState;
use super::{Acknowledgement, Error, acknowledgements, dto};
use crate::domain::connector_health::ConnectorHealthResponse;
use crate::infra::db::entities::connector_schedules::Model;

const SNAPSHOT_SQL: &str = "SELECT
    r.connector AS r_connector,
    r.source_id AS r_source_id,
    r.crons AS r_crons,
    r.revision AS r_revision,
    r.updated_by AS r_updated_by,
    r.updated_at AS r_updated_at,
    a.connector AS a_connector,
    a.source_id AS a_source_id,
    a.applied_revision AS a_applied_revision,
    a.applied_crons AS a_applied_crons,
    a.applied_timezone AS a_applied_timezone,
    a.applied_origin AS a_applied_origin,
    a.suspended AS a_suspended,
    a.deployment_crons AS a_deployment_crons,
    a.deployment_origin AS a_deployment_origin,
    a.observed_at AS a_observed_at,
    a.attempted_revision AS a_attempted_revision,
    a.attempted_at AS a_attempted_at,
    a.last_error AS a_last_error
    FROM
    (SELECT connector, source_id FROM connector_schedules UNION SELECT connector, source_id FROM connector_schedule_acknowledgements) AS instances
    LEFT JOIN connector_schedules r USING (connector, source_id)
    LEFT JOIN connector_schedule_acknowledgements a USING (connector, source_id)";

#[derive(Debug)]
struct Snapshot {
    request: Option<Model>,
    acknowledgement: Option<Acknowledgement>,
}

async fn snapshot(db: &DatabaseConnection) -> Result<Vec<Snapshot>, Error> {
    let rows = db
        .query_all_raw(Statement::from_string(
            db.get_database_backend(),
            SNAPSHOT_SQL,
        ))
        .await?;
    rows.iter()
        .map(|row| {
            let request = row
                .try_get::<Option<String>>("r_", "connector")?
                .map(|_| Model::from_query_result(row, "r_"))
                .transpose()?;
            let acknowledgement = row
                .try_get::<Option<String>>("a_", "connector")?
                .map(|_| Acknowledgement::from_query_result(row, "a_"))
                .transpose()?;
            Ok(Snapshot {
                request,
                acknowledgement,
            })
        })
        .collect()
}

pub(crate) async fn enrich_health(db: &DatabaseConnection, response: &mut ConnectorHealthResponse) {
    let rows = match snapshot(db).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "connector schedule requests unavailable");
            return;
        }
    };
    attach(response, &rows);
}

fn attach(response: &mut ConnectorHealthResponse, rows: &[Snapshot]) {
    let rows: std::collections::HashMap<_, _> = rows
        .iter()
        .filter_map(|row| {
            let key = row
                .request
                .as_ref()
                .map(|r| (&r.connector, &r.source_id))
                .or_else(|| {
                    row.acknowledgement
                        .as_ref()
                        .map(|a| (&a.connector, &a.source_id))
                })?;
            Some(((key.0.as_str(), key.1.as_str()), row))
        })
        .collect();
    for connector in &mut response.connectors {
        let Some(source) = connector.source_id.as_ref() else {
            continue;
        };
        let row = rows
            .get(&(connector.connector.as_str(), source.as_str()))
            .copied();
        match request_state(row) {
            Ok(state) => connector.schedule_request = Some(state),
            Err(error) => tracing::warn!(%error, "stored connector schedule request unreadable"),
        }
    }
}

fn request_state(row: Option<&Snapshot>) -> Result<ScheduleRequestState, Error> {
    let request = row.and_then(|row| row.request.as_ref());
    let acknowledgement = row.and_then(|row| row.acknowledgement.as_ref());
    let crons: Option<Vec<String>> = request
        .map(|r| serde_json::from_value(r.crons.clone()).map_err(Error::StoredCrons))
        .transpose()?;
    let revision = request.map_or(0, |r| r.revision);
    let status = dto::status(revision, crons.as_deref(), acknowledgement);
    let (applied, inherited, error) = acknowledgements::fields(acknowledgement)?;
    Ok(ScheduleRequestState {
        crons: crons.unwrap_or_default(),
        revision,
        status,
        updated_at: request.map(|r| r.updated_at),
        updated_by: request.map(|r| r.updated_by),
        applied,
        inherited,
        error,
    })
}

#[cfg(test)]
mod tests;
