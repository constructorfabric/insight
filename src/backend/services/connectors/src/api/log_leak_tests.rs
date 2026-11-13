use insight_log_context::test_support::capture_output;
use secrecy::SecretString;

use crate::config::GearConfig;

#[test]
fn logged_config_never_exposes_database_or_clickhouse_credentials() {
    let config = GearConfig {
        database_url: SecretString::from("mysql://user:example-url-password@localhost/connectors"),
        database_password: SecretString::from("example-db-password"),
        clickhouse_password: SecretString::from("example-ch-password"),
        ..GearConfig::default()
    };
    let output = capture_output(|| tracing::error!(config = ?config, "config probe"));
    for secret in [
        "example-url-password",
        "example-db-password",
        "example-ch-password",
    ] {
        assert!(!output.contains(secret), "leaked {secret}: {output}");
    }
    assert!(
        output.contains("REDACTED"),
        "missing redaction marker: {output}"
    );
}
