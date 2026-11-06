use axum::Json;
use axum::extract::rejection::JsonRejection;
use serde_json::Value;
use toolkit_canonical_errors::CanonicalError;

use super::error::{ScheduleError, invalid, unsupported_media_type};
use crate::domain::connector_schedules::dto::ScheduleRequest;

pub(super) fn parse(
    request: Result<Json<Value>, JsonRejection>,
) -> Result<ScheduleRequest, ScheduleError> {
    let Json(value) = request.map_err(|error| rejection(&error))?;
    Ok(parse_value(&value)?)
}

fn rejection(error: &JsonRejection) -> ScheduleError {
    if matches!(error, JsonRejection::MissingJsonContentType(_)) {
        return unsupported_media_type();
    }
    invalid(
        "body",
        format!("invalid JSON request body: {}", error.body_text()),
    )
    .into()
}

fn parse_value(value: &Value) -> Result<ScheduleRequest, CanonicalError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("body", "expected a JSON object with crons"))?;
    if let Some(field) = object.keys().find(|field| field.as_str() != "crons") {
        let detail = if field == "timezone" {
            "unknown field timezone: schedules are always UTC".to_owned()
        } else {
            format!("unknown field {field:?}; only crons is accepted")
        };
        return Err(invalid(field, detail));
    }
    let crons = object
        .get("crons")
        .ok_or_else(|| invalid("crons", "missing required field crons"))?;
    let crons = crons
        .as_array()
        .ok_or_else(|| invalid("crons", "expected a list of cron expression strings"))?;
    let crons = crons
        .iter()
        .enumerate()
        .map(|(index, cron)| {
            cron.as_str().map(str::to_owned).ok_or_else(|| {
                invalid(
                    &format!("crons[{index}]"),
                    "expected a cron expression string",
                )
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(ScheduleRequest { crons })
}

#[cfg(test)]
mod tests;
