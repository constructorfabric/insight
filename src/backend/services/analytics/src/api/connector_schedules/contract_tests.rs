use axum::body::to_bytes;
use axum::http::StatusCode;
use sea_orm::{DatabaseBackend, MockDatabase};
use serde_json::{Value, json};
use tower::ServiceExt;

use super::support::*;
use crate::infra::db::entities::connector_schedules::Model;
use crate::infra::identity::IdentityClient;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../ingestion/reconcile-connectors/tests/fixtures/connector-schedules.json"
);

fn validate(value: &Value, schema: &Value, document: &Value) -> R {
    if let Some(reference) = schema["$ref"].as_str() {
        return validate(
            value,
            document
                .pointer(&reference[1..])
                .ok_or("missing schema reference")?,
            document,
        );
    }
    for key in schema.as_object().ok_or("schema must be an object")?.keys() {
        assert!(
            [
                "type",
                "properties",
                "required",
                "items",
                "minimum",
                "format"
            ]
            .contains(&key.as_str()),
            "extend fixture validation for schema keyword {key}"
        );
    }
    match schema["type"].as_str().ok_or("schema type missing")? {
        "object" => {
            let object = value.as_object().ok_or("expected object")?;
            for field in schema["required"]
                .as_array()
                .ok_or("required fields missing")?
            {
                assert!(
                    object.contains_key(field.as_str().ok_or("invalid required field")?),
                    "{field}"
                );
            }
            for (field, value) in object {
                validate(value, &schema["properties"][field], document)?;
            }
        }
        "array" => {
            for item in value.as_array().ok_or("expected array")? {
                validate(item, &schema["items"], document)?;
            }
        }
        "integer" => {
            let integer = value.as_u64().ok_or("expected unsigned integer")?;
            assert!(integer >= schema["minimum"].as_u64().unwrap_or(0));
        }
        "string" => assert!(value.is_string()),
        other => panic!("extend fixture validation for type {other}"),
    }
    Ok(())
}

#[tokio::test]
async fn reconcile_handler_bytes_match_the_shared_reader_fixture() -> R {
    eprintln!(
        "regenerate from src/backend: UPDATE_CONNECTOR_SCHEDULES_FIXTURE=1 cargo test -p analytics reconcile_handler_bytes_match_the_shared_reader_fixture"
    );
    let mut rows = Vec::new();
    for (connector, source, crons, revision) in [
        (
            "example-connector",
            "main-east",
            vec!["0 3 * * *", "0 15 * * *"],
            3,
        ),
        ("example-connector", "second-west", vec!["30 6 * * 1-5"], 7),
        ("example-other", "main-east", vec!["0 9 * * *"], 2),
        ("example-other", "cleared-row", vec![], 4),
    ] {
        rows.push(Model {
            connector: connector.to_owned(),
            source_id: source.to_owned(),
            crons: json!(crons),
            revision,
            updated_by: CALLER,
            updated_at: chrono::DateTime::UNIX_EPOCH,
        });
    }
    let db = MockDatabase::new(DatabaseBackend::MySql)
        .append_query_results([rows
            .into_iter()
            .filter(|row| row.crons != json!([]))
            .collect::<Vec<_>>()])
        .into_connection();
    let app = app(
        db.clone(),
        "http://127.0.0.1:1",
        false,
        context("service", &["connector-schedules-reader"])?,
        IdentityClient::new("http://127.0.0.1:1")?,
    )?;
    let response = app
        .oneshot(request(
            "GET",
            "/v1/connector-schedules",
            None,
            &json!(null),
        )?)
        .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    let log = format!("{:?}", db.into_transaction_log());
    assert!(log.contains("JSON_LENGTH(crons) > 0"), "{log}");
    assert!(
        log.contains(
            "ORDER BY `connector_schedules`.`connector` ASC, `connector_schedules`.`source_id` ASC"
        ),
        "{log}"
    );
    let document: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../../docs/components/backend/analytics/openapi.json"
    )))?;
    validate(
        &serde_json::from_slice(&bytes)?,
        &document["paths"]["/v1/connector-schedules"]["get"]["responses"]["200"]["content"]["application/json"]
            ["schema"],
        &document,
    )?;
    if std::env::var_os("UPDATE_CONNECTOR_SCHEDULES_FIXTURE").is_some() {
        std::fs::write(FIXTURE, &bytes)?;
    }
    assert_eq!(
        bytes.as_ref(),
        std::fs::read(FIXTURE)?,
        "regenerate from src/backend: UPDATE_CONNECTOR_SCHEDULES_FIXTURE=1 cargo test -p analytics reconcile_handler_bytes_match_the_shared_reader_fixture"
    );
    Ok(())
}
