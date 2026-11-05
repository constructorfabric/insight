use super::{ConnectorSchedulesConfig, GearConfig};

type R = Result<(), Box<dyn std::error::Error>>;

#[test]
fn schedule_minimum_defaults_to_thirty_minutes() -> R {
    let config: GearConfig = serde_json::from_value(serde_json::json!({}))?;
    assert_eq!(config.connector_schedules.min_interval_minutes, 30);
    assert_eq!(ConnectorSchedulesConfig::default().min_interval_minutes, 30);
    Ok(())
}

#[test]
fn schedule_minimum_rejects_values_outside_thirty_minutes_to_one_day() {
    for value in [0, 1, 29, 1441] {
        let Err(problem) = serde_json::from_value::<GearConfig>(serde_json::json!({
            "connector_schedules": { "min_interval_minutes": value }
        })) else {
            panic!("should reject invalid minimum: {value}");
        };
        let message = problem.to_string();
        assert!(
            message.contains("connector_schedules__min_interval_minutes"),
            "{value}: {message}"
        );
        assert!(
            message.contains("between 30 and 1440"),
            "{value}: {message}"
        );
    }
    for value in [30, 1440] {
        assert!(
            serde_json::from_value::<GearConfig>(serde_json::json!({
                "connector_schedules": { "min_interval_minutes": value }
            }))
            .is_ok(),
            "should accept: {value}"
        );
    }
}
