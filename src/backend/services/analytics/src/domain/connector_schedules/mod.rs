pub(crate) mod dto;
mod health;
mod repository;
mod sync_history;
pub(crate) use health::enrich_health;
pub(crate) mod validation;

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::config::ConnectorSchedulesConfig;
use crate::domain::connector_health::{ConnectorName, SourceId};
use crate::infra::db::entities::connector_schedules::Model;
use dto::{RequestedSchedule, ScheduleResponse};
pub(crate) use repository::list;
use validation::Crons;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(
        "No connector instance {connector}/{source_id} is configured on this install. Connector health lists the configured instances by connector and source id."
    )]
    NotFound {
        connector: String,
        source_id: String,
    },
    #[error("schedule editing is turned off on this install")]
    Disabled,
    #[error("If-Match is required")]
    MissingMatch,
    #[error("If-Match must be a single quoted decimal ETag")]
    InvalidMatch,
    #[error("schedule revision does not match If-Match")]
    Stale(u64),
    #[error("schedule revision is exhausted")]
    RevisionExhausted,
    #[error("{0}")]
    Invalid(#[from] validation::Violation),
    #[error("schedule database read or write failed")]
    Database(#[from] sea_orm::DbErr),
    #[error("sync history read failed")]
    SyncHistory(#[from] clickhouse::error::Error),
    #[error("stored schedule is invalid")]
    StoredCrons(serde_json::Error),
}

#[derive(Debug)]
pub(crate) struct Instance {
    connector: ConnectorName,
    source_id: SourceId,
}

impl Instance {
    pub(crate) fn parse(connector: &str, source_id: &str) -> Result<Self, Error> {
        Ok(Self {
            connector: ConnectorName::parse(connector)
                .ok_or_else(|| validation::Violation::new("connector", "invalid connector name"))?,
            source_id: SourceId::parse(source_id)
                .ok_or_else(|| validation::Violation::new("source_id", "invalid source_id"))?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Revision(u64);

impl Revision {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, Error> {
        let raw = raw.ok_or(Error::MissingMatch)?;
        let digits = raw
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or(Error::InvalidMatch)?;
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return Err(Error::InvalidMatch);
        }
        digits.parse().map(Self).map_err(|_| Error::InvalidMatch)
    }
}

#[derive(Debug)]
pub(crate) enum Change {
    Put(Crons),
    Delete,
}

pub(crate) async fn get(
    db: &DatabaseConnection,
    ch: &insight_clickhouse::Client,
    config: &ConnectorSchedulesConfig,
    instance: &Instance,
) -> Result<ScheduleResponse, Error> {
    let applied_and_completion = match sync_history::applied_schedule_and_pass_completion(
        ch,
        instance.connector.as_str(),
        instance.source_id.as_str(),
    )
    .await
    {
        Ok(applied_and_completion) => Ok(applied_and_completion),
        Err(error @ Error::NotFound { .. }) => Err(error),
        Err(error) => return Err(error),
    };
    let row = repository::read(db, instance).await?;
    let (applied, pass_completed_at) = match applied_and_completion {
        Ok(applied_and_completion) => applied_and_completion,
        Err(Error::NotFound { .. }) if row.is_some() => (None, chrono::Utc::now()),
        Err(error) => return Err(error),
    };
    let response = response(instance, config, row, applied)?;
    observe(ch, response, pass_completed_at).await
}

#[derive(Debug)]
pub(crate) struct Editor<'a> {
    instance: &'a Instance,
    config: &'a ConnectorSchedulesConfig,
    applied: Option<dto::RecordedSchedule>,
    pass_completed_at: chrono::DateTime<chrono::Utc>,
}

pub(crate) async fn editor<'a>(
    ch: &'a insight_clickhouse::Client,
    config: &'a ConnectorSchedulesConfig,
    instance: &'a Instance,
) -> Result<Editor<'a>, Error> {
    let (applied, pass_completed_at) = sync_history::applied_schedule_and_pass_completion(
        ch,
        instance.connector.as_str(),
        instance.source_id.as_str(),
    )
    .await?;
    if !config.enabled {
        return Err(Error::Disabled);
    }
    Ok(Editor {
        instance,
        config,
        applied,
        pass_completed_at,
    })
}

pub(crate) async fn clear_editor<'a>(
    db: &DatabaseConnection,
    ch: &'a insight_clickhouse::Client,
    config: &'a ConnectorSchedulesConfig,
    instance: &'a Instance,
) -> Result<Editor<'a>, Error> {
    let error = match editor(ch, config, instance).await {
        Ok(editor) => return Ok(editor),
        Err(error @ Error::NotFound { .. }) => error,
        Err(error) => return Err(error),
    };
    if repository::read(db, instance).await?.is_none() {
        return Err(error);
    }
    if !config.enabled {
        return Err(Error::Disabled);
    }
    Ok(Editor {
        instance,
        config,
        applied: None,
        pass_completed_at: chrono::Utc::now(),
    })
}

impl Editor<'_> {
    pub(crate) async fn write(
        self,
        db: &DatabaseConnection,
        ch: &insight_clickhouse::Client,
        revision: Revision,
        change: Change,
        caller: Uuid,
    ) -> Result<ScheduleResponse, Error> {
        let crons = match change {
            Change::Put(crons) => crons.into_vec(),
            Change::Delete => Vec::new(),
        };
        let row = repository::write(db, self.instance, revision, crons, caller).await?;
        let response = response(self.instance, self.config, row, self.applied)?;
        observe(ch, response, self.pass_completed_at).await
    }
}

async fn observe(
    ch: &insight_clickhouse::Client,
    mut response: ScheduleResponse,
    pass_completed_at: chrono::DateTime<chrono::Utc>,
) -> Result<ScheduleResponse, Error> {
    if response.status != dto::ScheduleStatus::Pending
        || response
            .requested
            .as_ref()
            .is_none_or(|request| request.updated_at >= pass_completed_at)
    {
        return Ok(response);
    }
    let completed_passes = sync_history::completed_passes(ch, pass_completed_at).await?;
    (response.status, response.not_applied_since) = dto::observed_status(
        response.requested.as_ref(),
        response.applied.as_ref(),
        &completed_passes,
    );
    Ok(response)
}

fn response(
    instance: &Instance,
    config: &ConnectorSchedulesConfig,
    row: Option<Model>,
    applied: Option<dto::RecordedSchedule>,
) -> Result<ScheduleResponse, Error> {
    let revision = row.as_ref().map_or(0, |row| row.revision);
    let requested = row
        .map(|row| {
            let crons: Vec<String> =
                serde_json::from_value(row.crons).map_err(Error::StoredCrons)?;
            Ok::<_, Error>(RequestedSchedule {
                crons,
                updated_by: row.updated_by,
                updated_at: row.updated_at,
            })
        })
        .transpose()?;
    let status = dto::status(
        requested.as_ref().map(|r| r.crons.as_slice()),
        applied.as_ref(),
    );
    Ok(ScheduleResponse {
        connector: instance.connector.as_str().to_owned(),
        source_id: instance.source_id.as_str().to_owned(),
        editable: config.enabled,
        revision,
        requested,
        applied,
        status,
        not_applied_since: None,
        reconcile_schedules: config.reconcile_schedules.clone(),
        min_interval_minutes: config.min_interval_minutes,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) mod live_tests;

#[cfg(test)]
pub(crate) use repository::{AFTER_WRITE_READ, AFTER_WRITE_READ_RELEASE};
