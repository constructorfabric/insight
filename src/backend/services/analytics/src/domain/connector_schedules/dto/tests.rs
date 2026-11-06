use super::*;
use crate::domain::connector_schedules::test_support::acknowledgement;

#[test]
fn status_precedence_uses_the_current_revision_and_successful_observation() {
    for (name, revision, crons, applied, attempted, failed, expected) in [
        (
            "newer save",
            3,
            vec!["0 3 * * *"],
            Some(2),
            2,
            false,
            ScheduleStatus::Pending,
        ),
        (
            "current failure",
            2,
            vec!["0 3 * * *"],
            Some(2),
            2,
            true,
            ScheduleStatus::Error,
        ),
        (
            "older failure",
            3,
            vec!["0 3 * * *"],
            Some(2),
            2,
            true,
            ScheduleStatus::Pending,
        ),
        (
            "acknowledged clear",
            3,
            vec![],
            Some(3),
            3,
            false,
            ScheduleStatus::Inherited,
        ),
        (
            "unacknowledged clear",
            3,
            vec![],
            Some(2),
            2,
            false,
            ScheduleStatus::Pending,
        ),
        (
            "verified override",
            2,
            vec!["0 3 * * *"],
            Some(2),
            2,
            false,
            ScheduleStatus::Applied,
        ),
        (
            "failed first apply",
            2,
            vec!["0 3 * * *"],
            None,
            2,
            true,
            ScheduleStatus::Error,
        ),
        (
            "no successful observation",
            2,
            vec!["0 3 * * *"],
            None,
            1,
            true,
            ScheduleStatus::Pending,
        ),
        (
            "failed clear",
            3,
            vec![],
            Some(2),
            3,
            true,
            ScheduleStatus::Error,
        ),
    ] {
        let mut ack = acknowledgement();
        ack.applied_revision = applied;
        ack.attempted_revision = attempted;
        ack.last_error = failed.then(|| "synthetic readback failure".to_owned());
        let crons: Vec<String> = crons.into_iter().map(str::to_owned).collect();
        assert_eq!(
            status(revision, Some(&crons), Some(&ack)),
            expected,
            "{name}"
        );
    }
}

#[test]
fn missing_acknowledgements_without_an_override_are_inherited() {
    for revision in [0, 1, u64::MAX] {
        for requested in [None, Some([].as_slice())] {
            assert_eq!(
                status(revision, requested, None),
                ScheduleStatus::Inherited,
                "{revision}, {requested:?}"
            );
        }
    }
}

#[test]
fn requests_without_acknowledgements_are_pending() {
    let crons = ["0 3 * * *".to_owned()];
    for revision in [1, u64::MAX] {
        assert_eq!(
            status(revision, Some(&crons), None),
            ScheduleStatus::Pending,
            "{revision}"
        );
    }
}

#[test]
fn applied_observations_require_every_fact_except_timezone()
-> Result<(), Box<dyn std::error::Error>> {
    let instance = super::super::Instance::parse("example-tracker", "main")?;
    for missing in [
        "revision",
        "crons",
        "origin",
        "suspended",
        "observed_at",
        "timezone",
    ] {
        let mut ack = acknowledgement();
        match missing {
            "revision" => ack.applied_revision = None,
            "crons" => ack.applied_crons = None,
            "origin" => ack.applied_origin = None,
            "suspended" => ack.suspended = None,
            "observed_at" => ack.observed_at = None,
            "timezone" => ack.applied_timezone = None,
            _ => return Err("unknown observation field".into()),
        }
        let row = crate::infra::db::entities::connector_schedules::Model {
            connector: "example-tracker".to_owned(),
            source_id: "main".to_owned(),
            crons: serde_json::json!(["0 6 * * *"]),
            revision: 3,
            updated_by: uuid::Uuid::from_u128(1),
            updated_at: chrono::DateTime::UNIX_EPOCH,
        };
        let response = super::super::response(&instance, Some(row), Some(&ack))?;
        assert_eq!(
            response.applied.is_some(),
            missing == "timezone",
            "{missing}"
        );
        if let Some(applied) = response.applied {
            assert!(applied.timezone.is_none(), "{missing}");
            let applied = serde_json::to_value(applied)?;
            assert_eq!(
                applied.get("timezone"),
                Some(&serde_json::Value::Null),
                "{missing}"
            );
        }
        assert!(response.inherited.is_some(), "{missing}");
        assert_eq!(response.status, ScheduleStatus::Pending, "{missing}");
    }
    Ok(())
}

#[test]
fn nullable_response_fields_are_present_as_null() -> Result<(), Box<dyn std::error::Error>> {
    let instance = super::super::Instance::parse("example-tracker", "main")?;
    let response = super::super::response(&instance, None, None)?;
    let state = ScheduleRequestState {
        crons: vec![],
        revision: 0,
        status: ScheduleStatus::Inherited,
        updated_at: None,
        updated_by: None,
        applied: None,
        inherited: None,
        error: None,
    };
    for (value, fields) in [
        (
            serde_json::to_value(response)?,
            vec!["requested", "applied", "inherited", "error"],
        ),
        (
            serde_json::to_value(state)?,
            vec!["updated_at", "updated_by", "applied", "inherited", "error"],
        ),
    ] {
        let object = value.as_object().ok_or("response must be an object")?;
        for field in fields {
            assert_eq!(
                object.get(field),
                Some(&serde_json::Value::Null),
                "{field}: {value}"
            );
        }
    }
    Ok(())
}
