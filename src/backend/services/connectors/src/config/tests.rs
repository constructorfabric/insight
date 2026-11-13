use super::*;

type R = Result<(), Box<dyn std::error::Error>>;

fn app(config: serde_json::Value) -> AppConfig {
    let mut app = AppConfig::default();
    app.gears.insert(
        "connectors".to_owned(),
        serde_json::Value::Object(serde_json::Map::from_iter([("config".to_owned(), config)])),
    );
    app
}

fn valid_config() -> serde_json::Value {
    serde_json::json!({
        "database_url": "mysql://user:password@localhost/connectors",
        "clickhouse_url": "http://localhost:8123",
        "identity_url": "http://localhost:8082"
    })
}

#[test]
fn check_requires_the_connectors_config_section() -> R {
    let error = crate::gear::check_config(&AppConfig::default())
        .err()
        .ok_or("expected invalid configuration")?;
    assert!(
        error
            .to_string()
            .contains("missing `gears.connectors.config`")
    );
    Ok(())
}

#[test]
fn check_names_the_invalid_leaf_and_its_environment_override() -> R {
    for (key, value) in [
        ("database_url", "https://localhost/connectors"),
        ("database_url", "mysql://localhost"),
        ("clickhouse_url", ""),
        ("clickhouse_url", "file:///tmp/example"),
        ("clickhouse_database", " "),
        ("identity_url", ""),
        ("identity_url", "file:///tmp/example"),
    ] {
        let mut config = valid_config();
        config[key] = serde_json::json!(value);
        let error = crate::gear::check_config(&app(config))
            .err()
            .ok_or("expected invalid configuration")?
            .to_string();
        assert!(
            error.contains(&format!("APP__gears__connectors__config__{key}")),
            "should reject {key}={value:?}: {error}"
        );
    }
    Ok(())
}

#[test]
fn check_accepts_a_database_url_or_database_leaves() -> R {
    crate::gear::check_config(&app(valid_config()))?;
    let mut config = valid_config();
    config
        .as_object_mut()
        .ok_or("missing config")?
        .remove("database_url");
    config["database_host"] = serde_json::json!("localhost");
    config["database_user"] = serde_json::json!("user");
    crate::gear::check_config(&app(config))?;
    Ok(())
}

#[test]
fn database_leaves_are_required_without_a_url() -> R {
    for (key, value) in [
        ("database_host", serde_json::json!("")),
        ("database_user", serde_json::json!("")),
        ("database_name", serde_json::json!("")),
        ("database_port", serde_json::json!(0)),
    ] {
        let mut raw = serde_json::json!({"database_host": "localhost", "database_user": "user"});
        raw[key] = value;
        let config: GearConfig = serde_json::from_value(raw)?;
        let error = config
            .database_url()
            .err()
            .ok_or("expected invalid configuration")?
            .to_string();
        assert!(error.contains(key), "should reject {key}: {error}");
    }
    Ok(())
}

#[test]
fn database_credentials_cannot_change_url_structure() -> R {
    let config = GearConfig {
        database_host: "localhost".to_owned(),
        database_user: "user@host".to_owned(),
        database_name: "example/name".to_owned(),
        database_password: SecretString::from("p@ss:/?#%2F word"),
        ..GearConfig::default()
    };
    let url = config.database_url()?;
    let parsed = Url::parse(url.expose_secret())?;
    assert_eq!(parsed.host_str(), Some("localhost"));
    assert_eq!(parsed.username(), "user%40host");
    assert_eq!(parsed.password(), Some("p%40ss%3A%2F%3F%23%252F%20word"));
    assert_eq!(parsed.path(), "/example%2Fname");
    assert!(parsed.query().is_none());
    assert!(parsed.fragment().is_none());
    Ok(())
}

#[test]
fn explicit_url_takes_precedence_over_database_leaves() -> R {
    let config: GearConfig = serde_json::from_value(valid_config())?;
    assert_eq!(
        config.database_url()?.expose_secret(),
        "mysql://user:password@localhost/connectors"
    );
    Ok(())
}

#[test]
fn printed_configuration_masks_every_service_secret() -> R {
    let raw = serde_json::json!({
        "database_url": "mysql://user:example-url-secret@localhost/connectors",
        "database_password": "example-db-secret",
        "clickhouse_password": "example-ch-secret",
        "database_host": "localhost"
    });
    let output = redacted_yaml(&app(raw))?;
    for value in [
        "example-url-secret",
        "example-db-secret",
        "example-ch-secret",
    ] {
        assert!(!output.contains(value), "leaked {value}: {output}");
    }
    assert!(output.contains("localhost"));
    Ok(())
}

#[test]
fn malformed_config_errors_never_echo_secrets() -> R {
    let error = crate::gear::check_config(&app(serde_json::json!({
        "database_port": "example-secret"
    })))
    .err()
    .ok_or("expected invalid configuration")?
    .to_string();
    assert!(!error.contains("example-secret"));
    assert!(error.contains("invalid `gears.connectors.config`"));
    Ok(())
}

#[test]
fn cluster_configuration_requires_identity() {
    for (key, value) in [
        ("namespace", serde_json::json!("")),
        ("tenant_id", serde_json::json!("")),
        ("namespace", serde_json::json!("INVALID")),
    ] {
        let mut raw = valid_config();
        raw["cluster"] = serde_json::json!({"enabled": true});
        raw["namespace"] = serde_json::json!("example");
        raw["tenant_id"] = serde_json::json!("tenant01");
        raw[key] = value;
        assert!(
            crate::gear::check_config(&app(raw)).is_err(),
            "should reject {key}"
        );
    }
}
