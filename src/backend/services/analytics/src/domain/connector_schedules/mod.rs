mod acknowledgements;
pub(crate) mod dto;
mod health;
mod membership;
mod repository;
pub(crate) use health::enrich_health;
pub(crate) mod validation;

use sea_orm::DatabaseConnection;
use uuid::Uuid;

use crate::domain::connector_health::{ConnectorName, SourceId};
use crate::infra::db::entities::connector_schedule_acknowledgements::Model as Acknowledgement;
use crate::infra::db::entities::connector_schedules::Model;
use dto::{RequestedSchedule, ScheduleResponse};
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
    instance: &Instance,
) -> Result<ScheduleResponse, Error> {
    let membership = match membership::require(ch, instance).await {
        Ok(()) => Ok(()),
        Err(error @ Error::NotFound { .. }) => Err(error),
        Err(error) => return Err(error),
    };
    let row = repository::read(db, instance).await?;
    if row.is_none() {
        membership?;
    }
    let acknowledgement = acknowledgements::read(db, instance).await?;
    response(instance, row, acknowledgement.as_ref())
}

#[derive(Debug)]
pub(crate) struct Editor<'a> {
    instance: &'a Instance,
}

pub(crate) async fn editor<'a>(
    ch: &'a insight_clickhouse::Client,
    instance: &'a Instance,
) -> Result<Editor<'a>, Error> {
    membership::require(ch, instance).await?;
    Ok(Editor { instance })
}

pub(crate) async fn clear_editor<'a>(
    db: &DatabaseConnection,
    ch: &'a insight_clickhouse::Client,
    instance: &'a Instance,
) -> Result<Editor<'a>, Error> {
    let error = match editor(ch, instance).await {
        Ok(editor) => return Ok(editor),
        Err(error @ Error::NotFound { .. }) => error,
        Err(error) => return Err(error),
    };
    if repository::read(db, instance).await?.is_none() {
        return Err(error);
    }
    Ok(Editor { instance })
}

impl Editor<'_> {
    pub(crate) async fn write(
        self,
        db: &DatabaseConnection,
        revision: Revision,
        change: Change,
        caller: Uuid,
    ) -> Result<ScheduleResponse, Error> {
        let crons = match change {
            Change::Put(crons) => crons.into_vec(),
            Change::Delete => Vec::new(),
        };
        let row = repository::write(db, self.instance, revision, crons, caller).await?;
        let acknowledgement = acknowledgements::read(db, self.instance).await?;
        response(self.instance, row, acknowledgement.as_ref())
    }
}

fn response(
    instance: &Instance,
    row: Option<Model>,
    acknowledgement: Option<&Acknowledgement>,
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
        revision,
        requested.as_ref().map(|r| r.crons.as_slice()),
        acknowledgement,
    );
    let (applied, inherited, error) = acknowledgements::fields(acknowledgement)?;
    Ok(ScheduleResponse {
        connector: instance.connector.as_str().to_owned(),
        source_id: instance.source_id.as_str().to_owned(),
        revision,
        requested,
        applied,
        status,
        inherited,
        error,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
pub(super) mod live_tests;

#[cfg(test)]
pub(crate) use repository::{AFTER_WRITE_READ, AFTER_WRITE_READ_RELEASE};
