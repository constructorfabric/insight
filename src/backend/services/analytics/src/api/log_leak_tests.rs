//! Seeded-leak checks (insight#2488 AC-4): write a token and a person-bearing
//! query through the logging path — the value must never reach a line.

use insight_log_context::test_support::capture_output;

use crate::domain::ai::dto::ApiToken;
use crate::infra::query::log_query_failure;

type R = Result<(), Box<dyn std::error::Error>>;

const SEEDED_EMAIL: &str = "seeded.person-3192@example.com";
const SEEDED_NAME: &str = "Seeded Person 3192";
const SEEDED_API_TOKEN: &str = "sk-ant-seeded-api-token-3192";

#[test]
fn a_failed_query_line_carries_a_hash_never_the_sql() {
    let sql = format!(
        "SELECT * FROM gold.person_summary WHERE email = '{SEEDED_EMAIL}' AND name = '{SEEDED_NAME}'"
    );

    let output = capture_output(|| {
        log_query_failure(
            "connection refused",
            "probe-comment",
            &sql,
            "seeded leak probe",
        );
    });

    assert!(!output.contains(SEEDED_EMAIL), "leaked: email in {output}");
    assert!(!output.contains(SEEDED_NAME), "leaked: name in {output}");
    assert!(output.contains("sql_hash"), "no sql_hash field in {output}");
    assert!(
        output.contains("probe-comment"),
        "the join key must survive: {output}"
    );
}

#[test]
fn an_error_echoing_the_query_is_scrubbed_before_it_is_logged() {
    let sql = format!("SELECT name FROM persons WHERE email = '{SEEDED_EMAIL}'");
    let echoing_errors = [
        format!(
            "bad response: Code: 62. DB::Exception: Syntax error: failed at position 8 ('{SEEDED_EMAIL}')"
        ),
        format!(
            "bad response: Code: 47. DB::Exception: Missing columns while processing query: '{sql}'"
        ),
        format!("bad response: Code: 47. DB::Exception: Unknown identifier. In query: {sql}"),
        format!("bad response: {sql}"),
    ];

    for error in &echoing_errors {
        let output =
            capture_output(|| log_query_failure(error, "probe-comment", &sql, "seeded probe"));

        assert!(
            !output.contains(SEEDED_EMAIL),
            "leaked via error echo: {error} -> {output}"
        );
        assert!(
            output.contains("query echo scrubbed"),
            "no scrub marker for: {error} -> {output}"
        );
        assert!(
            output.contains("DB::Exception") || output.contains("bad response"),
            "the diagnosis must survive: {error} -> {output}"
        );
    }
}

#[test]
fn a_logged_api_token_renders_a_marker_never_the_value() -> R {
    let token = ApiToken::parse(SEEDED_API_TOKEN).map_err(|e| format!("{e:?}"))?;

    let output = capture_output(|| tracing::error!(token = ?token, "seeded leak probe"));

    assert!(
        !output.contains(SEEDED_API_TOKEN),
        "leaked: token in {output}"
    );
    assert!(output.contains("redacted"), "no marker in {output}");
    Ok(())
}
