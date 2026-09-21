use secrecy::SecretString;

use super::*;

fn valid_config() -> GearConfig {
    GearConfig {
        dataset_preview_rows: 50,
        dataset_lease_secs: 60,
        clickhouse_url: "http://clickhouse.example.test:8123".to_owned(),
        clickhouse_database: "insight".to_owned(),
        identity_database: "identity".to_owned(),
        datasets_database: "insight_datasets".to_owned(),
        clickhouse_user: None,
        clickhouse_password: None,
        clickhouse_query_user: None,
        clickhouse_query_password: None,
        ingest_token: SecretString::from("test-ingest-token-0123456789abcdef"),
        anthropic_token: SecretString::from("test-anthropic-token"),
        mcp: McpConfig::default(),
        chat_model: "claude-sonnet-5".to_owned(),
        database_url: "mysql://insight:secret@mariadb.example.test:3306/insight_v3".to_owned(),
        identity_url: "http://identity-resolution.example.test:8082".to_owned(),
    }
}

#[test]
fn required_values_must_not_be_empty() {
    for field in ["clickhouse_url", "clickhouse_database", "ingest_token"] {
        let mut config = valid_config();
        match field {
            "clickhouse_url" => config.clickhouse_url.clear(),
            "clickhouse_database" => config.clickhouse_database.clear(),
            "ingest_token" => config.ingest_token = SecretString::from(String::new()),
            _ => unreachable!(),
        }

        assert!(config.validate().is_err(), "empty {field} must be rejected");
    }
}

/// The datasets database is a name this service writes into SQL and the one
/// place it creates and drops tables, so it is a plain identifier of its own.
#[test]
fn the_datasets_database_is_an_identifier_apart_from_the_warehouse() {
    let cases = [
        ("insight_datasets", true),
        ("ds2", true),
        ("", false),
        ("insight datasets", false),
        ("insight`datasets", false),
        ("insight-datasets", false),
        ("insight", false),
    ];

    for (name, accepted) in cases {
        let mut config = valid_config();
        config.datasets_database = name.to_owned();

        assert_eq!(
            config.validate().is_ok(),
            accepted,
            "datasets_database {name:?} should be accepted: {accepted}"
        );
    }
}

#[test]
fn the_dataset_settings_stay_within_their_bounds() {
    let previews = [(0, false), (1, true), (500, true), (501, false)];
    for (rows, accepted) in previews {
        let mut config = valid_config();
        config.dataset_preview_rows = rows;

        assert_eq!(
            config.validate().is_ok(),
            accepted,
            "dataset_preview_rows {rows} should be accepted: {accepted}"
        );
    }

    let leases = [(0, false), (1, true), (3600, true), (3601, false)];
    for (seconds, accepted) in leases {
        let mut config = valid_config();
        config.dataset_lease_secs = seconds;

        assert_eq!(
            config.validate().is_ok(),
            accepted,
            "dataset_lease_secs {seconds} should be accepted: {accepted}"
        );
    }
}

#[test]
fn a_migration_refuses_the_same_datasets_database_as_a_served_request() {
    let mut config = valid_config();
    config.datasets_database = "insight".to_owned();

    assert!(config.validate_stores().is_err());
}

#[test]
fn chat_model_must_not_be_empty() {
    let mut config = valid_config();
    config.chat_model.clear();

    assert!(config.validate().is_err());
}

#[test]
fn a_stand_without_an_anthropic_token_still_serves_everything_else() {
    let mut config = valid_config();
    config.anthropic_token = SecretString::from(String::new());
    assert!(
        config.validate().is_ok(),
        "the assistant is one endpoint; the rest of the service does not wait on its key"
    );
}

#[test]
fn secrets_are_redacted_from_debug_output() {
    let mut config = valid_config();
    config.clickhouse_user = Some("writer".to_owned());
    config.clickhouse_password = Some(SecretString::from("database-secret"));

    let rendered = format!("{config:?}");

    assert!(!rendered.contains("test-ingest-token-0123456789abcdef"));
    assert!(!rendered.contains("database-secret"));
    assert!(!rendered.contains("test-anthropic-token"));
}

#[test]
fn credentials_must_be_complete() {
    let mut config = valid_config();
    config.clickhouse_user = Some("writer".to_owned());

    assert!(matches!(
        config.validate(),
        Err(ConfigError::IncompleteCredentials)
    ));
}

#[test]
fn ingest_token_must_be_usable_in_the_instance_token_header() {
    let invalid_tokens = [
        "token with space".to_owned(),
        "töken".to_owned(),
        "x".repeat(1025),
    ];

    for token in invalid_tokens {
        let mut config = valid_config();
        config.ingest_token = SecretString::from(token);

        assert!(
            config.validate().is_err(),
            "configured token outside the instance-token header contract must be rejected"
        );
    }
}

#[test]
fn ingest_token_accepts_the_wire_size_boundary() {
    let mut config = valid_config();
    config.ingest_token = SecretString::from("x".repeat(MAX_INGEST_TOKEN_BYTES));

    assert!(config.validate().is_ok());
}

#[test]
fn ingest_token_rejects_values_shorter_than_32_bytes() {
    let mut config = valid_config();
    config.ingest_token = SecretString::from("x".repeat(31));

    assert!(config.validate().is_err());
}

#[test]
fn ingest_token_accepts_the_minimum_size_boundary() {
    let mut config = valid_config();
    config.ingest_token = SecretString::from("x".repeat(32));

    assert!(config.validate().is_ok());
}

#[test]
fn the_query_client_falls_back_to_the_ordinary_credentials_when_no_reader_is_configured() {
    let mut config = valid_config();
    config.clickhouse_user = Some("writer".to_owned());
    config.clickhouse_password = Some(SecretString::from("writer-secret"));

    let validated = config
        .validate()
        .unwrap_or_else(|error| panic!("config must be valid: {error}"));
    let client = validated.clickhouse_query_client();

    assert_eq!(client.config().user.as_deref(), Some("writer"));
    assert_eq!(client.config().password.as_deref(), Some("writer-secret"));
}

#[test]
fn the_query_client_uses_the_reader_when_it_is_configured() {
    let mut config = valid_config();
    config.clickhouse_user = Some("writer".to_owned());
    config.clickhouse_password = Some(SecretString::from("writer-secret"));
    config.clickhouse_query_user = Some("reader".to_owned());
    config.clickhouse_query_password = Some(SecretString::from("reader-secret"));

    let validated = config
        .validate()
        .unwrap_or_else(|error| panic!("config must be valid: {error}"));
    let client = validated.clickhouse_query_client();

    assert_eq!(client.config().user.as_deref(), Some("reader"));
    assert_eq!(client.config().password.as_deref(), Some("reader-secret"));
}

#[test]
fn a_blank_reader_setting_reads_as_unset_rather_than_as_a_credential() {
    let mut config = valid_config();
    config.clickhouse_user = Some("writer".to_owned());
    config.clickhouse_password = Some(SecretString::from("writer-secret"));
    config.clickhouse_query_user = Some(String::new());
    config.clickhouse_query_password = Some(SecretString::from(String::new()));

    let validated = config
        .validate()
        .unwrap_or_else(|error| panic!("config must be valid: {error}"));
    let client = validated.clickhouse_query_client();

    assert_eq!(client.config().user.as_deref(), Some("writer"));
}

#[test]
fn half_a_reader_credential_is_refused_instead_of_falling_back() {
    let mut config = valid_config();
    config.clickhouse_query_user = Some("reader".to_owned());

    assert!(matches!(
        config.validate(),
        Err(ConfigError::IncompleteQueryCredentials)
    ));
}
fn mcp(enabled: bool, bind_addr: &str, public_url: &str) -> McpConfig {
    McpConfig {
        enabled,
        bind_addr: bind_addr.to_owned(),
        public_url: public_url.to_owned(),
        jwks_url: String::new(),
        allow_insecure_private_network: false,
    }
}

#[test]
fn the_mcp_server_is_off_and_bound_to_the_documented_port_by_default() {
    let config = McpConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.bind_addr, "0.0.0.0:8087");
    assert!(config.public_url.is_empty());
}

#[test]
fn a_disabled_mcp_server_needs_no_public_url() {
    assert!(validate_mcp(&McpConfig::default()).is_ok());
}

#[test]
fn an_enabled_mcp_server_without_a_public_url_is_refused() {
    let Err(error) = validate_mcp(&mcp(true, "0.0.0.0:8087", "")) else {
        panic!("a server with no origin has no issuer to verify a token against");
    };

    assert!(
        matches!(error, ConfigError::Empty("mcp.public_url")),
        "{error:?}"
    );
}

#[test]
fn an_enabled_mcp_server_with_an_unparseable_bind_address_is_refused() {
    let Err(error) = validate_mcp(&mcp(
        true,
        "not-an-address",
        "https://insight.example.invalid",
    )) else {
        panic!("an address that is not a socket address cannot be bound");
    };

    assert!(matches!(error, ConfigError::McpBindAddr), "{error:?}");
}

#[test]
fn an_enabled_mcp_server_is_accepted_with_an_origin_and_an_address() {
    assert!(
        validate_mcp(&mcp(
            true,
            "0.0.0.0:8087",
            "https://insight.example.invalid"
        ))
        .is_ok()
    );
}

#[test]
fn a_config_whose_mcp_section_is_invalid_is_refused_as_a_whole() {
    let mut raw = valid_config();
    raw.mcp = mcp(true, "0.0.0.0:8087", "");

    let Err(error) = raw.validate() else {
        panic!("an enabled server with no origin makes the whole config invalid");
    };

    assert!(
        matches!(error, ConfigError::Empty("mcp.public_url")),
        "{error:?}"
    );
}
