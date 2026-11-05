use chrono::{DateTime, Utc};
use sea_orm::DatabaseConnection;

use super::dto::{
    RequestedSchedule, ScheduleRequestState, ScheduleStatus, after_pass, status_for_crons,
};
use super::{Error, repository, sync_history};
use crate::config::ConnectorSchedulesConfig;
use crate::domain::connector_health::{ConnectorHealth, ConnectorHealthResponse, ScheduleOrigin};
use crate::infra::db::entities::connector_schedules::Model;

pub(crate) async fn enrich_health(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    config: &ConnectorSchedulesConfig,
    response: &mut ConnectorHealthResponse,
) {
    if !config.enabled {
        return;
    }
    let rows = match repository::all(db).await {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "connector schedule requests unavailable");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let Some(through) = response
        .checked_at
        .as_ref()
        .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
    else {
        return;
    };
    let completed_passes =
        match sync_history::completed_passes(ch, through.with_timezone(&Utc)).await {
            Ok(completed_passes) => completed_passes,
            Err(error) => {
                tracing::warn!(%error, "connector schedule completed_passes unavailable");
                return;
            }
        };
    attach(response, &rows, &completed_passes);
}

fn attach(
    response: &mut ConnectorHealthResponse,
    rows: &[Model],
    completed_passes: &[DateTime<Utc>],
) {
    let rows: std::collections::HashMap<_, _> = rows
        .iter()
        .map(|row| ((row.connector.as_str(), row.source_id.as_str()), row))
        .collect();
    for connector in &mut response.connectors {
        let Some(source) = connector.source_id.as_ref() else {
            continue;
        };
        let Some(row) = rows.get(&(connector.connector.as_str(), source.as_str())) else {
            continue;
        };
        match request_state(connector, row, completed_passes) {
            Ok(state) => connector.schedule_request = state,
            Err(error) => tracing::warn!(%error, "stored connector schedule request unreadable"),
        }
    }
}

fn request_state(
    connector: &ConnectorHealth,
    row: &Model,
    completed_passes: &[DateTime<Utc>],
) -> Result<Option<ScheduleRequestState>, Error> {
    let requested = RequestedSchedule {
        crons: serde_json::from_value(row.crons.clone()).map_err(Error::StoredCrons)?,
        updated_at: row.updated_at,
        updated_by: row.updated_by,
    };
    let applied = connector
        .schedule
        .as_ref()
        .filter(|schedule| matches!(schedule.origin, Some(ScheduleOrigin::Api)))
        .map(|schedule| schedule.crons.as_slice());
    let status = status_for_crons(Some(&requested.crons), applied);
    let (status, not_applied_since) = after_pass(status, Some(&requested), completed_passes);
    Ok(
        (status != ScheduleStatus::Inherited).then_some(ScheduleRequestState {
            crons: requested.crons,
            status,
            updated_at: requested.updated_at,
            updated_by: requested.updated_by,
            not_applied_since,
        }),
    )
}

#[cfg(test)]
mod tests;
