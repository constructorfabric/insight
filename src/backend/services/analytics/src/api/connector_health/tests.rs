//! The refusals this surface raises.
//!
//! Asserted against the `Problem` JSON rather than through a router, so they
//! stay free of the production `AppState` (which needs MariaDB, ClickHouse and
//! an identity client). What matters here is that a refused caller learns which
//! surface refused them, and that a read failure never leaks the warehouse's
//! own words onto the wire.

use toolkit_canonical_errors::{CanonicalError, Problem};

use super::*;

const NAMESPACE: &str = "gts.cf.insight.analytics_api.connector_health.v1~";

fn problem(error: CanonicalError) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::to_value(Problem::from(error))
}

#[test]
fn a_refusal_names_the_surface_it_refused() -> Result<(), Box<dyn std::error::Error>> {
    let p = problem(admin_only())?;
    assert_eq!(p["status"], 403);
    assert_eq!(p["context"]["resource_type"], NAMESPACE);
    Ok(())
}

#[test]
fn a_refusal_says_what_the_caller_is_missing() -> Result<(), Box<dyn std::error::Error>> {
    let p = problem(admin_only())?;
    assert_eq!(p["context"]["reason"], ADMIN_ONLY);
    Ok(())
}

#[test]
fn an_unusable_connector_name_is_a_bad_request_not_a_not_found()
-> Result<(), Box<dyn std::error::Error>> {
    let p = problem(unnamed_connector())?;
    assert_eq!(p["status"], 400);
    let violations = p["context"]["field_violations"]
        .as_array()
        .ok_or("field_violations must be an array")?;
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0]["field"], "connector");
    Ok(())
}

fn scope(tenant: Option<&str>, source: Option<&str>) -> SyncScopeQuery {
    SyncScopeQuery {
        tenant_id: tenant.map(str::to_owned),
        source_id: source.map(str::to_owned),
    }
}

#[test]
fn naming_neither_half_asks_about_every_installation() -> Result<(), Box<dyn std::error::Error>> {
    assert!(parse_scope(&scope(None, None))?.is_none());
    Ok(())
}

#[test]
fn naming_both_halves_narrows_to_one_installation() -> Result<(), Box<dyn std::error::Error>> {
    let (tenant, source) =
        parse_scope(&scope(Some("acme"), Some("claude-team-main")))?.ok_or("must narrow")?;
    assert_eq!(tenant.as_str(), "acme");
    assert_eq!(source.as_str(), "claude-team-main");
    Ok(())
}

#[test]
fn half_an_identity_is_refused_rather_than_quietly_widened()
-> Result<(), Box<dyn std::error::Error>> {
    // Widening back to the connector would answer a question about one
    // installation with rows from all of them, and the answer would look right.
    for (tenant, source, missing) in [
        (Some("acme"), None, "source_id"),
        (None, Some("claude-team-main"), "tenant_id"),
    ] {
        let refusal = parse_scope(&scope(tenant, source))
            .err()
            .ok_or("half an identity must be refused")?;
        let p = problem(refusal)?;
        assert_eq!(p["status"], 400);
        assert_eq!(p["context"]["field_violations"][0]["field"], missing);
    }
    Ok(())
}

#[test]
fn an_identity_outside_the_vocabulary_is_a_bad_request() -> Result<(), Box<dyn std::error::Error>> {
    let refusal = parse_scope(&scope(Some("Acme"), Some("main")))
        .err()
        .ok_or("an unusable identity must be refused")?;
    let p = problem(refusal)?;
    assert_eq!(p["status"], 400);
    assert_eq!(p["context"]["field_violations"][0]["field"], "tenant_id");
    Ok(())
}

#[test]
fn a_read_failure_does_not_repeat_the_warehouse_to_the_caller()
-> Result<(), Box<dyn std::error::Error>> {
    let leaky = clickhouse::error::Error::BadResponse(
        "Code: 60. DB::Exception: Table ingestion_history.sync_events does not exist".to_owned(),
    );
    let p = problem(read_error(leaky))?;
    assert_eq!(p["status"], 500);
    let body = serde_json::to_string(&p)?;
    assert!(
        !body.contains("ingestion_history"),
        "an internal relation name must not cross the wire: {body}"
    );
    assert!(!body.contains("DB::Exception"), "{body}");
    Ok(())
}
