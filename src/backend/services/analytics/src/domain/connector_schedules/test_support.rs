use std::collections::BTreeMap;

use sea_orm::{Iterable, ModelTrait, Value};

use super::Acknowledgement;
use crate::infra::db::entities::connector_schedules::Model;

pub(crate) fn acknowledgement() -> Acknowledgement {
    Acknowledgement {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        applied_revision: Some(2),
        applied_crons: Some(serde_json::json!(["0 3 * * *"])),
        applied_timezone: Some("UTC".to_owned()),
        applied_origin: Some("api".to_owned()),
        suspended: Some(false),
        deployment_crons: serde_json::json!(["0 5 * * *"]),
        deployment_origin: "descriptor".to_owned(),
        observed_at: Some(chrono::DateTime::UNIX_EPOCH),
        attempted_revision: 2,
        attempted_at: chrono::DateTime::UNIX_EPOCH,
        last_error: None,
    }
}

pub(crate) fn snapshot_row(
    request: Option<&Model>,
    ack: Option<&Acknowledgement>,
) -> BTreeMap<String, Value> {
    let mut row = BTreeMap::new();
    for column in crate::infra::db::entities::connector_schedules::Column::iter() {
        row.insert(
            format!("r_{}", sea_orm::Iden::to_string(&column)),
            request.map_or(Value::String(None), |r| r.get(column)),
        );
    }
    for column in crate::infra::db::entities::connector_schedule_acknowledgements::Column::iter() {
        row.insert(
            format!("a_{}", sea_orm::Iden::to_string(&column)),
            ack.map_or(Value::String(None), |a| a.get(column)),
        );
    }
    row
}

pub(crate) fn schedule_response(
    request: Model,
    ack: &Acknowledgement,
) -> Result<super::dto::ScheduleResponse, super::Error> {
    let instance = super::Instance::parse(&request.connector, &request.source_id)?;
    super::response(
        &instance,
        &crate::config::ConnectorSchedulesConfig::default(),
        Some(request),
        Some(ack),
    )
}
