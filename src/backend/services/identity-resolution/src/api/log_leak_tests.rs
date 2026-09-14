//! Seeded-leak checks (insight#2488 AC-4): write a person's address and the
//! store credentials through the logging path — the values must never reach a
//! line. Pins the address-free audit discipline #1847 established.

use insight_log_context::test_support::capture_output;
use uuid::Uuid;

use crate::config::GearConfig;
use crate::domain::login_bootstrap::{self, RosterEmailMatch};

const SEEDED_EMAIL: &str = "seeded.person-3192@example.com";
const SEEDED_DB_PASSWORD: &str = "seeded-db-password-3192";

#[test]
fn a_contested_address_is_audited_without_the_address() {
    let asked = login_bootstrap::RosterEmail {
        tenant_id: Uuid::from_u128(7),
        source_type: "roster",
        address: SEEDED_EMAIL,
    };
    let resolved = RosterEmailMatch {
        person_id: Uuid::from_u128(9),
        candidates: 2,
    };

    let output =
        capture_output(|| super::handlers::audit_contested_roster_email(&asked, &resolved));

    assert!(
        output.contains("login_roster_email_ambiguous"),
        "the audit event must be written: {output}"
    );
    assert!(
        !output.contains(SEEDED_EMAIL),
        "leaked: address in {output}"
    );
}

#[test]
fn a_roster_email_refusal_logs_no_address() {
    let output = capture_output(|| {
        for refusal in [
            login_bootstrap::RosterEmailRefusal::AddressMissing,
            login_bootstrap::RosterEmailRefusal::RosterUnconfigured,
            login_bootstrap::RosterEmailRefusal::TenantUnresolved,
        ] {
            let _ = super::handlers::refused_roster_email(refusal);
        }
    });

    assert!(!output.is_empty(), "the refusal lines must be written");
    assert!(
        !output.contains(SEEDED_EMAIL),
        "leaked: address in {output}"
    );
    assert!(
        !output.contains('@'),
        "an address shape reached a refusal line: {output}"
    );
}

#[test]
fn a_logged_gear_config_renders_markers_never_the_store_credentials() {
    let config = GearConfig {
        database_url: format!("mysql://insight:{SEEDED_DB_PASSWORD}@db:3306/identity"),
        clickhouse_password: SEEDED_DB_PASSWORD.to_owned(),
        ..GearConfig::default()
    };

    let output = capture_output(|| tracing::info!(config = ?config, "seeded leak probe"));

    assert!(
        !output.contains(SEEDED_DB_PASSWORD),
        "leaked: store credential in {output}"
    );
    assert!(output.contains("<redacted>"), "no marker in {output}");
}
