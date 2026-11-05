use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn if_match_requires_one_quoted_unsigned_decimal_revision() -> R {
    for (raw, expected) in [
        ("\"0\"", 0),
        ("\"3\"", 3),
        ("\"0003\"", 3),
        ("\"18446744073709551615\"", u64::MAX),
    ] {
        assert_eq!(
            Revision::parse(Some(raw))?.0,
            expected,
            "should accept: {raw:?}"
        );
    }
    assert!(matches!(Revision::parse(None), Err(Error::MissingMatch)));
    for raw in [
        "",
        "0",
        "*",
        "W/\"3\"",
        "\"3\", \"4\"",
        "\"-1\"",
        "\"+1\"",
        "\"1.0\"",
        "\"\"",
        " \"3\"",
        "\"3\" ",
        "\"18446744073709551616\"",
        "\"٣\"",
        "\"3\n\"",
    ] {
        assert!(
            matches!(Revision::parse(Some(raw)), Err(Error::InvalidMatch)),
            "should reject: {raw:?}"
        );
    }
    Ok(())
}

#[test]
fn cleared_rows_keep_the_revision_and_request_metadata() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    let config = ConnectorSchedulesConfig::default();
    let row = Model {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        crons: serde_json::json!([]),
        revision: 4,
        updated_by: Uuid::from_u128(1),
        updated_at: chrono::DateTime::UNIX_EPOCH,
    };
    let response = response(&instance, &config, Some(row), None)?;
    assert_eq!(response.revision, 4);
    assert!(
        response
            .requested
            .ok_or("missing clear metadata")?
            .crons
            .is_empty()
    );
    assert!(response.applied.is_none());
    assert_eq!(response.status, dto::ScheduleStatus::Pending);
    Ok(())
}

#[test]
fn invalid_instance_ids_name_the_path_parameter() {
    for (connector, source, field) in [
        ("invalid/name", "main", "connector"),
        ("example-tracker", "invalid/source", "source_id"),
    ] {
        let Err(Error::Invalid(problem)) = Instance::parse(connector, source) else {
            panic!("wrong error kind");
        };
        assert_eq!(problem.field, field);
    }
}

#[test]
fn raising_the_minimum_keeps_stored_schedules_readable() -> R {
    let instance = Instance::parse("example-tracker", "main")?;
    let config = ConnectorSchedulesConfig {
        min_interval_minutes: 60,
        ..ConnectorSchedulesConfig::default()
    };
    let row = Model {
        connector: "example-tracker".to_owned(),
        source_id: "main".to_owned(),
        crons: serde_json::json!(["*/15 * * * *"]),
        revision: 7,
        updated_by: Uuid::from_u128(1),
        updated_at: chrono::DateTime::UNIX_EPOCH,
    };
    let response = response(&instance, &config, Some(row), None)?;
    assert_eq!(response.min_interval_minutes, 60);
    assert_eq!(
        response.requested.ok_or("missing stored request")?.crons,
        ["*/15 * * * *"]
    );
    Ok(())
}
