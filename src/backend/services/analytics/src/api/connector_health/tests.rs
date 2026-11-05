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
fn the_summary_contract_describes_the_recorded_schedule() -> Result<(), Box<dyn std::error::Error>>
{
    let document = serde_json::to_value(super::super::openapi_document()?)?;
    let schemas = &document["components"]["schemas"];

    assert_eq!(
        schemas["ConnectorHealth"]["properties"]["schedule"]["oneOf"][1]["$ref"],
        "#/components/schemas/AppliedSchedule"
    );
    assert_eq!(
        schemas["AppliedSchedule"]["properties"]["crons"]["minItems"],
        1
    );
    assert_eq!(
        schemas["AppliedSchedule"]["properties"]["suspended"]["type"],
        "boolean"
    );
    assert!(
        schemas["AppliedSchedule"]["required"]
            .as_array()
            .ok_or("required fields must be an array")?
            .contains(&serde_json::json!("suspended"))
    );
    assert_eq!(
        schemas["ScheduleOrigin"]["enum"],
        serde_json::json!(["api", "annotation", "descriptor", "default"])
    );
    assert_eq!(
        schemas["ConnectorHealth"]["properties"]["schedule_request"]["oneOf"][1]["$ref"],
        "#/components/schemas/ScheduleRequestState"
    );
    assert_eq!(
        schemas["RecordedSchedule"]["properties"]["observed_at"]["format"],
        "date-time"
    );
    Ok(())
}

#[tokio::test]
async fn the_health_handler_serializes_nullable_and_acknowledged_request_facts()
-> Result<(), Box<dyn std::error::Error>> {
    let response: ConnectorHealthResponse = serde_json::from_value(serde_json::json!({
        "as_of": "2026-01-15T13:00:00Z", "history_available": true,
        "connectors": [
            {"connector": "example-tracker", "configured": true, "schedule_request": {
                "crons": [], "revision": 2, "status": "error", "applied": null, "inherited": null, "updated_at": "2026-01-15T12:00:00Z",
                "updated_by": "00000000-0000-4000-8000-000000000001", "error": {"attempted_revision": 2, "attempted_at": "2026-01-15T12:45:00Z", "message": "synthetic failure"}
            }},
            {"connector": "example-directory", "configured": true}
        ]
    }))?;
    let response = Json(response).into_response();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 65536).await?;
    let body: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
        body["connectors"][0]["schedule_request"]["crons"],
        serde_json::json!([])
    );
    assert_eq!(body["connectors"][0]["schedule_request"]["status"], "error");
    assert_eq!(
        body["connectors"][0]["schedule_request"]["error"]["attempted_at"],
        "2026-01-15T12:45:00Z"
    );
    assert!(body["connectors"][1]["schedule_request"].is_null());
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
