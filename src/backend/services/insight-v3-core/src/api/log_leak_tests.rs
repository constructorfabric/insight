//! Seeded-leak checks (insight#2488 AC-4).
//!
//! This service holds four secrets and reads one credential-bearing header:
//! the ingest token arrives in `X-Insight-Token`, and the config carries the
//! MariaDB connection string, the ClickHouse passwords and the Anthropic key.
//! Each is seeded here with a value a grep can find, so a regression fails
//! where the line is written rather than in a log somewhere.

use insight_log_context::test_support::{capture_output, capture_probe_output};
use insight_log_context::{CORRELATION_ID_HEADER, LogContextLayer};
use uuid::Uuid;

use crate::config::GearConfig;

type R = Result<(), Box<dyn std::error::Error>>;

const SEEDED_INGEST_TOKEN: &str = "seeded-ingest-token-3192";
const SEEDED_DB_PASSWORD: &str = "seeded-mariadb-password-3192";
const SEEDED_ANTHROPIC_TOKEN: &str = "seeded-anthropic-token-3192";
const SEEDED_CLICKHOUSE_PASSWORD: &str = "seeded-clickhouse-password-3192";

#[test]
fn a_request_carrying_the_ingest_token_logs_none_of_it() -> R {
    let output = capture_probe_output(
        LogContextLayer::new(),
        &[
            (CORRELATION_ID_HEADER, "corr-3192"),
            ("x-insight-token", SEEDED_INGEST_TOKEN),
            ("authorization", &format!("Bearer {SEEDED_INGEST_TOKEN}")),
        ],
        Some(Uuid::from_u128(7)),
    )?;

    assert!(!output.is_empty(), "the probe request must log something");
    assert!(
        !output.contains(SEEDED_INGEST_TOKEN),
        "leaked: ingest token in {output}"
    );

    Ok(())
}

#[test]
fn logging_the_config_renders_markers_rather_than_its_secrets() {
    let config = GearConfig {
        database_url: format!("mysql://insight:{SEEDED_DB_PASSWORD}@mariadb:3306/insight_v3"),
        clickhouse_password: Some(SEEDED_CLICKHOUSE_PASSWORD.to_owned().into()),
        anthropic_token: SEEDED_ANTHROPIC_TOKEN.to_owned().into(),
        ingest_token: SEEDED_INGEST_TOKEN.to_owned().into(),
        ..GearConfig::default()
    };

    let output = capture_output(|| tracing::info!(?config, "config loaded"));

    for seeded in [
        SEEDED_DB_PASSWORD,
        SEEDED_CLICKHOUSE_PASSWORD,
        SEEDED_ANTHROPIC_TOKEN,
        SEEDED_INGEST_TOKEN,
    ] {
        assert!(!output.contains(seeded), "leaked: {seeded} in {output}");
    }

    // The line itself has to exist, or the assertions above pass vacuously.
    assert!(output.contains("config loaded"), "{output}");
    // And it has to carry the config, redacted — not be missing the field.
    assert!(output.contains("<redacted>"), "{output}");
}
