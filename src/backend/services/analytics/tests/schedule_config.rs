use std::process::Command;

use serde_yaml::Value;

type R = Result<(), Box<dyn std::error::Error>>;

fn schedule_config(
    field: &str,
    yaml_value: &str,
    env_value: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let file = tempfile::NamedTempFile::new()?;
    let directory = tempfile::tempdir()?;
    std::fs::write(
        file.path(),
        format!(
            "server:\n  home_dir: {}\ngears:\n  analytics:\n    config:\n      connector_schedules:\n        {field}: {yaml_value}\n",
            directory.path().display()
        ),
    )?;

    let key = format!("APP__gears__analytics__config__connector_schedules__{field}");
    let output = Command::new(env!("CARGO_BIN_EXE_analytics"))
        .arg("--config")
        .arg(file.path())
        .arg("--print-config")
        .env(&key, env_value)
        .output()?;
    assert!(
        output.status.success(),
        "child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout)?;
    let yaml = stdout
        .strip_prefix("Effective configuration:\n")
        .ok_or("effective configuration missing")?;
    let config: Value = serde_yaml::from_str(yaml)?;
    Ok(config["gears"]["analytics"]["config"]["connector_schedules"].clone())
}

#[test]
fn schedule_minimum_environment_overrides_the_yaml_value() -> R {
    let config = schedule_config("min_interval_minutes", "45", "60")?;
    assert_eq!(config["min_interval_minutes"].as_u64(), Some(60));
    Ok(())
}

#[test]
fn schedule_cadence_environment_overrides_yaml_with_a_json_list() -> R {
    let config = schedule_config(
        "reconcile_schedules",
        "['0 * * * *']",
        r#"["7 */2 * * *", "19 */3 * * *"]"#,
    )?;
    let expected: Value = serde_yaml::from_str("['7 */2 * * *', '19 */3 * * *']")?;
    assert_eq!(config["reconcile_schedules"], expected);
    Ok(())
}
