use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

use super::dto::{InheritedSchedule, RecordedSchedule, ScheduleAttemptError};
use super::{Acknowledgement, Error, Instance};
use crate::infra::db::entities::connector_schedule_acknowledgements::{Column, Entity};

type Fields = (
    Option<RecordedSchedule>,
    Option<InheritedSchedule>,
    Option<ScheduleAttemptError>,
);

pub(super) async fn read(
    db: &DatabaseConnection,
    instance: &Instance,
) -> Result<Option<Acknowledgement>, Error> {
    Ok(Entity::find()
        .filter(Column::Connector.eq(instance.connector.as_str()))
        .filter(Column::SourceId.eq(instance.source_id.as_str()))
        .one(db)
        .await?)
}

pub(super) fn fields(ack: Option<&Acknowledgement>) -> Result<Fields, Error> {
    let Some(ack) = ack else {
        return Ok((None, None, None));
    };
    let applied = match (
        ack.applied_revision,
        ack.applied_crons.as_ref(),
        ack.applied_origin.as_ref(),
        ack.suspended,
        ack.observed_at,
    ) {
        (Some(revision), Some(crons), Some(origin), Some(suspended), Some(observed_at)) => {
            Some(RecordedSchedule {
                crons: serde_json::from_value(crons.clone()).map_err(Error::StoredCrons)?,
                timezone: ack.applied_timezone.clone(),
                origin: origin.clone(),
                suspended,
                revision,
                observed_at,
            })
        }
        _ => None,
    };
    let inherited = Some(InheritedSchedule {
        crons: serde_json::from_value(ack.deployment_crons.clone()).map_err(Error::StoredCrons)?,
        origin: ack.deployment_origin.clone(),
    });
    let error = ack.last_error.as_ref().map(|message| ScheduleAttemptError {
        attempted_revision: ack.attempted_revision,
        attempted_at: ack.attempted_at,
        message: message.clone(),
    });
    Ok((applied, inherited, error))
}
