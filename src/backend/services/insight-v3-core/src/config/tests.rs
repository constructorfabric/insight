use secrecy::SecretString;
use serde_json::json;

use super::*;

fn valid_config() -> GearConfig {
    GearConfig {
        dataset_preview_rows: 50,
        dataset_lease_secs: 60,
        clickhouse_url: "http://clickhouse.example.test:8123".to_owned(),
        clickhouse_database: "insight".to_owned(),
        identity_database: "identity".to_owned(),
        datasets_database: "insight_datasets".to_owned(),
        clickhouse_cluster_mode: false,
        clickhouse_cluster_name: String::new(),
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
        alerts: AlertsConfig::default(),
    }
}

fn alerts_on() -> AlertsConfig {
    AlertsConfig {
        enabled: true,
        redis_url: "redis://redis.example.test:6379".to_owned(),
        destinations: std::collections::BTreeMap::from([(
            "ops".to_owned(),
            DestinationConfig::Discord {
                webhook_url: SecretString::from(
                    "https://discord.example.test/api/webhooks/1/alert-webhook-secret",
                ),
            },
        )]),
        ..AlertsConfig::default()
    }
}

#[test]
fn alerts_are_off_by_default_and_need_no_redis_then() {
    let config = valid_config();
    assert!(!config.alerts.enabled);
    assert!(config.validate().is_ok());
}

#[test]
fn enabled_alerts_need_a_redis_and_sane_bounds() {
    let mut config = valid_config();
    config.alerts = AlertsConfig {
        redis_url: String::new(),
        ..alerts_on()
    };
    assert!(matches!(
        config.validate(),
        Err(ConfigError::Empty("alerts.redis_url"))
    ));

    let cases: Vec<(&str, AlertsConfig)> = vec![
        (
            "min above max",
            AlertsConfig {
                min_interval_secs: 10,
                max_interval_secs: 5,
                ..alerts_on()
            },
        ),
        (
            "zero interval",
            AlertsConfig {
                min_interval_secs: 0,
                ..alerts_on()
            },
        ),
        (
            "no rules allowed",
            AlertsConfig {
                max_rules: 0,
                ..alerts_on()
            },
        ),
        (
            "no concurrency",
            AlertsConfig {
                evaluation_concurrency: 0,
                ..alerts_on()
            },
        ),
        (
            "destination with a space",
            AlertsConfig {
                destinations: std::collections::BTreeMap::from([(
                    "ops team".to_owned(),
                    DestinationConfig::Telegram {
                        bot_token: SecretString::from("token"),
                        chat_id: "1".to_owned(),
                    },
                )]),
                ..alerts_on()
            },
        ),
        (
            "webhook over plain http",
            AlertsConfig {
                destinations: std::collections::BTreeMap::from([(
                    "ops".to_owned(),
                    DestinationConfig::Discord {
                        webhook_url: SecretString::from(
                            "http://discord.example.test/api/webhooks/1/x",
                        ),
                    },
                )]),
                ..alerts_on()
            },
        ),
        (
            "zulip without a topic",
            AlertsConfig {
                destinations: std::collections::BTreeMap::from([(
                    "ops".to_owned(),
                    DestinationConfig::Zulip {
                        site_url: "https://zulip.example.test".to_owned(),
                        bot_email: "bot@example.test".to_owned(),
                        api_key: SecretString::from("key"),
                        stream: "alerts".to_owned(),
                        topic: " ".to_owned(),
                    },
                )]),
                ..alerts_on()
            },
        ),
    ];
    for (case, alerts) in cases {
        let mut config = valid_config();
        config.alerts = alerts;
        assert!(config.validate().is_err(), "should refuse: {case}");
    }

    let mut config = valid_config();
    config.alerts = alerts_on();
    let validated = config
        .validate()
        .unwrap_or_else(|error| panic!("a whole alerts section is accepted: {error}"));
    assert_eq!(
        validated.alerts().destinations().provider_of("ops"),
        Some("discord")
    );
    assert_eq!(validated.alerts().limits().max_rules, 200);
}

#[test]
fn the_redis_url_is_redacted_from_debug_output() {
    let mut config = valid_config();
    config.alerts = AlertsConfig {
        redis_url: "redis://:alert-redis-secret@redis.example.test:6379".to_owned(),
        redis_password: Some(SecretString::from("alert-redis-password")),
        ..alerts_on()
    };

    let shown = format!("{config:?}");
    assert!(!shown.contains("alert-redis-secret"), "{shown}");
    assert!(!shown.contains("alert-redis-password"), "{shown}");
    assert!(!shown.contains("alert-webhook-secret"), "{shown}");
    assert!(shown.contains("Discord"), "{shown}");
}

#[test]
fn the_delivery_settings_show_in_debug_output() {
    let mut config = valid_config();
    config.alerts = AlertsConfig {
        delivery_attempts: 7,
        delivery_backoff_secs: 11,
        delivery_timeout_secs: 13,
        delivery_concurrency: 3,
        ..alerts_on()
    };

    let shown = format!("{config:?}");
    for setting in [
        "delivery_attempts: 7",
        "delivery_backoff_secs: 11",
        "delivery_timeout_secs: 13",
        "delivery_concurrency: 3",
    ] {
        assert!(shown.contains(setting), "should show {setting}: {shown}");
    }
}

#[test]
fn a_destination_name_is_a_plain_lowercase_identifier() {
    let cases = [
        ("ops", true),
        ("ops-team", true),
        ("ops_team_2", true),
        ("Ops-Team", false),
        ("OPS", false),
        ("ops team", false),
        ("ops.team", false),
        ("", false),
    ];

    for (name, accepted) in cases {
        let mut config = valid_config();
        config.alerts = AlertsConfig {
            destinations: std::collections::BTreeMap::from([(
                name.to_owned(),
                DestinationConfig::Telegram {
                    bot_token: SecretString::from("token"),
                    chat_id: "1".to_owned(),
                },
            )]),
            ..alerts_on()
        };

        let outcome = config.validate();
        assert_eq!(
            outcome.is_ok(),
            accepted,
            "destination {name:?} should be accepted: {accepted}"
        );
        if !accepted {
            assert!(
                matches!(&outcome, Err(ConfigError::AlertDestinationName(refused)) if refused == name),
                "should refuse the name itself: {name:?}"
            );
        }
    }
}

#[test]
fn a_plain_http_destination_is_admitted_on_loopback_only() {
    let cases = [
        ("https://discord.example.test/api/webhooks/1/x", true),
        ("http://localhost:9000/webhook", true),
        ("http://127.0.0.1:9000/webhook", true),
        ("http://[::1]:9000/webhook", true),
        ("http://discord.example.test/api/webhooks/1/x", false),
        ("http://10.0.0.1:9000/webhook", false),
        ("http://[2001:db8::1]:9000/webhook", false),
        ("ftp://localhost/webhook", false),
        ("not a url", false),
    ];

    for (value, accepted) in cases {
        assert_eq!(
            is_https_url(value),
            accepted,
            "should accept {value:?}: {accepted}"
        );
    }
}

#[test]
fn a_redis_password_kept_apart_reaches_the_client_whatever_it_contains() {
    use redis::IntoConnectionInfo as _;

    for password in ["plain", "p@ss/w:rd#?%25 x", "pässwörd"] {
        let mut config = valid_config();
        config.alerts = AlertsConfig {
            redis_url: "redis://redis.example.test:6379".to_owned(),
            redis_password: Some(SecretString::from(password)),
            ..alerts_on()
        };

        let validated = config
            .validate()
            .unwrap_or_else(|error| panic!("should accept {password:?}: {error}"));
        let alerts = validated.alerts();
        let info = alerts
            .redis_url
            .as_str()
            .into_connection_info()
            .unwrap_or_else(|error| {
                panic!("the client should read the URL for {password:?}: {error}")
            });

        assert_eq!(
            info.redis_settings().password(),
            Some(password),
            "{password:?}"
        );
        assert!(
            matches!(info.addr(), redis::ConnectionAddr::Tcp(host, 6379) if host == "redis.example.test"),
            "{password:?}: {:?}",
            info.addr()
        );
        assert!(
            alerts.redis_password.is_none(),
            "once validated the password lives in the URL alone"
        );
    }
}

#[test]
fn a_redis_url_without_a_separate_password_passes_through_untouched() {
    let mut config = valid_config();
    config.alerts = AlertsConfig {
        redis_url: "redis://:embedded-secret@redis.example.test:6379/1".to_owned(),
        redis_password: Some(SecretString::from(String::new())),
        ..alerts_on()
    };

    let validated = config
        .validate()
        .unwrap_or_else(|error| panic!("an embedded password alone is fine: {error}"));

    assert_eq!(
        validated.alerts().redis_url,
        "redis://:embedded-secret@redis.example.test:6379/1"
    );
}

#[test]
fn a_second_redis_password_or_an_unusable_url_is_refused() {
    let cases = [
        (
            "redis://:embedded@redis.example.test:6379",
            "credentials twice",
        ),
        ("redis://user@redis.example.test:6379", "credentials twice"),
        ("not a url", "unusable url"),
        ("redis.example.test:6379", "unusable url"),
    ];

    for (url, expected) in cases {
        let mut config = valid_config();
        config.alerts = AlertsConfig {
            redis_url: url.to_owned(),
            redis_password: Some(SecretString::from("apart")),
            ..alerts_on()
        };

        let refusal = config.validate().err();
        let refused_as_expected = match expected {
            "credentials twice" => matches!(refusal, Some(ConfigError::AlertRedisCredentialsTwice)),
            _ => matches!(refusal, Some(ConfigError::AlertRedisUrl)),
        };
        assert!(
            refused_as_expected,
            "should refuse {url:?} as {expected}: {refusal:?}"
        );
    }
}

#[test]
fn alerts_enabled_without_a_destination_are_accepted_with_a_warning() {
    let mut config = valid_config();
    config.alerts = AlertsConfig {
        destinations: std::collections::BTreeMap::new(),
        ..alerts_on()
    };

    let mut accepted = false;
    let logged = insight_log_context::test_support::capture_output(|| {
        accepted = config.validate().is_ok();
    });

    assert!(
        accepted,
        "a section with no destination still validates; compose ships one"
    );
    assert!(logged.contains("no destinations"), "{logged}");
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

#[test]
fn a_telegram_chat_id_is_read_whether_the_environment_gave_digits_or_text() {
    let cases = [
        (json!(754_770_951), "754770951"),
        (json!(-1_001_234_567_890_i64), "-1001234567890"),
        (json!("754770951"), "754770951"),
        (json!("@insight_alerts"), "@insight_alerts"),
    ];

    for (written, expected) in cases {
        let parsed: DestinationConfig = serde_json::from_value(json!({
            "provider": "telegram",
            "bot_token": "token",
            "chat_id": written,
        }))
        .unwrap_or_else(|error| panic!("should read chat id {written}: {error}"));

        let DestinationConfig::Telegram { chat_id, .. } = parsed else {
            panic!("should be a Telegram destination: {written}");
        };
        assert_eq!(chat_id, expected, "should read chat id: {written}");
    }
}

#[test]
fn a_warehouse_is_standalone_until_the_operator_says_otherwise() {
    let validated = valid_config()
        .validate()
        .unwrap_or_else(|error| panic!("the default config is valid: {error}"));

    assert_eq!(
        validated.datasets_client().config().topology,
        insight_clickhouse::Topology::Standalone
    );
}

#[test]
fn a_clustered_warehouse_carries_its_cluster_into_every_client() {
    let validated = GearConfig {
        clickhouse_cluster_mode: true,
        clickhouse_cluster_name: "insight_cluster".to_owned(),
        ..valid_config()
    }
    .validate()
    .unwrap_or_else(|error| panic!("a named cluster is valid: {error}"));

    for client in [
        validated.clickhouse_client(),
        validated.clickhouse_query_client(),
        validated.datasets_client(),
    ] {
        assert_eq!(
            client.config().topology.on_cluster(),
            Some("insight_cluster")
        );
    }
}
