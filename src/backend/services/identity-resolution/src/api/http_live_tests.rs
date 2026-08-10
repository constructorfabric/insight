//! HTTP-level tests driving the **real** route table through `tower::oneshot`
//! against a live MariaDB — the layer the repo-level `visible_set_live_tests`
//! cannot reach: extractor → gate → handler → `Result`→`Response` mapping, and
//! with it the status codes the analytics gate and the SPA depend on.
//!
//! INVARIANT: never `#[ignore]` these — the identity CI job runs `cargo test`
//! without `--include-ignored`. They skip at runtime when
//! `INTEGRATION_TESTS_MARIADB_URL` is unset, like every other live suite here.
//!
//! The host authn pipeline is bypassed: the `SecurityContext` is injected
//! directly, because the gateway-JWT verification needs a live JWKS and is
//! covered by the plugin's own tests plus the compose e2e. Each test mints its
//! own tenant, so the suite is parallel-safe.

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use toolkit::api::OpenApiRegistryImpl;
use toolkit_security::SecurityContext;

use super::AppState;
use crate::config::GearConfig;
use crate::infra::db::test_fixture::{Fixture, fixture_or_skip};

type TestResult = anyhow::Result<()>;

/// The caller the injected `SecurityContext` speaks for.
#[derive(Clone, Copy)]
struct Caller {
    person_id: Uuid,
    tenant: Uuid,
}

fn app(f: &Fixture, caller: Uuid) -> Router {
    app_for(
        f,
        Caller {
            person_id: caller,
            tenant: f.tenant,
        },
    )
}

fn app_for(f: &Fixture, caller: Caller) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    let state = Arc::new(AppState {
        db: f.db.clone(),
        config: GearConfig::default(),
    });
    let api = super::build_operations(Router::new(), &openapi)
        .layer(from_fn_with_state(caller, inject_host_context))
        .layer(axum::Extension(state));

    Router::new().merge(api)
}

async fn inject_host_context(
    axum::extract::State(caller): axum::extract::State<Caller>,
    mut req: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let Ok(ctx) = SecurityContext::builder()
        .subject_id(caller.person_id)
        .subject_type("user")
        .subject_tenant_id(caller.tenant)
        .build()
    else {
        unreachable!("subject_id + subject_tenant_id are set")
    };
    req.extensions_mut().insert(ctx);
    next.run(req).await
}

fn json_req(uri: &str, body: &Value) -> anyhow::Result<Request<Body>> {
    Ok(Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))?)
}

async fn post(app: Router, uri: &str, body: &Value) -> anyhow::Result<(StatusCode, Value)> {
    let resp = app.oneshot(json_req(uri, body)?).await?;
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await?;
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, payload))
}

fn visible_ids(payload: &Value) -> Vec<String> {
    payload["visible"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn by_person_id(person_id: Uuid) -> Value {
    json!({"value_type": "person_id", "value": person_id.to_string()})
}

// ── POST /v1/visible-persons ────────────────────────────────

#[tokio::test]
async fn visible_persons_answers_the_callers_subtree_and_omits_a_stranger() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let manager = f.person("manager@http-live.test").await?;
    let report = f.person("report@http-live.test").await?;
    let stranger = f.person("stranger@http-live.test").await?;
    f.reports_to(report, manager).await?;

    let (status, body) = post(
        app(&f, manager),
        "/v1/visible-persons",
        &json!({"person_ids": [report.to_string(), stranger.to_string()]}),
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(visible_ids(&body), vec![report.to_string()]);
    Ok(())
}

#[tokio::test]
async fn visible_persons_refuses_a_request_naming_nobody() -> TestResult {
    // Empty and all-nil both collapse to "no ids to check". A 200 with an empty
    // `visible` would read to analytics as "everything you asked for is hidden".
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller@http-live.test").await?;

    for body in [
        json!({"person_ids": []}),
        json!({"person_ids": [Uuid::nil().to_string()]}),
    ] {
        let (status, _) = post(app(&f, caller), "/v1/visible-persons", &body).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "should reject: {body}");
    }
    Ok(())
}

#[tokio::test]
async fn visible_persons_rejects_more_ids_than_the_cap() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("cap@http-live.test").await?;
    let over_cap: Vec<String> = (0..=super::visible_persons::MAX_PERSON_IDS)
        .map(|i| Uuid::from_u128(i as u128 + 1).to_string())
        .collect();

    let (status, body) = post(
        app(&f, caller),
        "/v1/visible-persons",
        &json!({"person_ids": over_cap}),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body.to_string().contains("person_ids"),
        "the violation names the field: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn visible_persons_rejects_a_value_that_is_not_a_uuid() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("shape@http-live.test").await?;

    let (status, _) = post(
        app(&f, caller),
        "/v1/visible-persons",
        &json!({"person_ids": ["somebody@example.com"]}),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn a_wildcard_holder_gets_only_ids_their_own_tenant_has() -> TestResult {
    // The wildcard branch of the handler, at the layer analytics consumes it:
    // a grant covers everyone IN THE TENANT, so a foreign or invented id must
    // not come back confirmed — analytics reads this answer as authorization.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let viewer = f.person("wildcard@http-live.test").await?;
    let ours = f.person("colleague@http-live.test").await?;
    let foreign = f
        .in_another_tenant()
        .person("foreign@http-live.test")
        .await?;
    f.grant(viewer, None).await?;

    let (status, body) = post(
        app(&f, viewer),
        "/v1/visible-persons",
        &json!({"person_ids": [
            ours.to_string(),
            foreign.to_string(),
            Uuid::now_v7().to_string(),
        ]}),
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        visible_ids(&body),
        vec![ours.to_string()],
        "the wildcard reaches the tenant, and stops there"
    );
    Ok(())
}

#[tokio::test]
async fn visible_persons_without_a_caller_is_unauthenticated() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let somebody = f.person("somebody@http-live.test").await?;

    let (status, _) = post(
        app(&f, Uuid::nil()),
        "/v1/visible-persons",
        &json!({"person_ids": [somebody.to_string()]}),
    )
    .await?;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}

// ── POST /v1/profiles, value_type = person_id ───────────────

#[tokio::test]
async fn a_profile_resolves_by_person_id() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("self@http-live.test").await?;

    let (status, body) = post(app(&f, caller), "/v1/profiles", &by_person_id(caller)).await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["person_id"], caller.to_string());
    Ok(())
}

#[tokio::test]
async fn a_profile_resolves_by_person_id_for_a_person_with_no_email() -> TestResult {
    // The reason the key changed: this person is unreachable by email.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let manager = f.person("manager2@http-live.test").await?;
    let emailless = f.emailless_person().await?;
    f.reports_to(emailless, manager).await?;

    let (status, body) = post(app(&f, manager), "/v1/profiles", &by_person_id(emailless)).await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["person_id"], emailless.to_string());
    Ok(())
}

#[tokio::test]
async fn a_person_id_outside_the_visible_set_is_not_found() -> TestResult {
    // Visibility applies to the person_id key exactly as it does to email: a
    // hidden person is indistinguishable from a missing one.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller2@http-live.test").await?;
    let hidden = f.person("hidden@http-live.test").await?;

    let (status, _) = post(app(&f, caller), "/v1/profiles", &by_person_id(hidden)).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "hidden reads as missing");

    f.grant(caller, Some(hidden)).await?;
    let (granted, body) = post(app(&f, caller), "/v1/profiles", &by_person_id(hidden)).await?;
    assert_eq!(granted, StatusCode::OK, "the grant opens the same lookup");
    assert_eq!(body["person_id"], hidden.to_string());
    Ok(())
}

#[tokio::test]
async fn a_person_id_from_another_tenant_is_not_found() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller3@http-live.test").await?;
    let foreign = f
        .in_another_tenant()
        .person("foreign2@http-live.test")
        .await?;

    let (status, _) = post(app(&f, caller), "/v1/profiles", &by_person_id(foreign)).await?;

    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn an_unobserved_person_id_is_not_found() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller4@http-live.test").await?;

    let (status, _) = post(
        app(&f, caller),
        "/v1/profiles",
        &by_person_id(Uuid::now_v7()),
    )
    .await?;

    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn a_person_id_value_that_is_not_a_person_uuid_is_a_client_error() -> TestResult {
    // The pre-cutover email shape and the nil UUID: both parse-level refusals,
    // never a 404 that would read as "no such person".
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller5@http-live.test").await?;

    for value in ["somebody@example.com", &Uuid::nil().to_string(), "   "] {
        let body = json!({"value_type": "person_id", "value": value});
        let (status, _) = post(app(&f, caller), "/v1/profiles", &body).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "should reject: {value:?}");
    }
    Ok(())
}

#[tokio::test]
async fn a_person_id_lookup_forbids_the_source_fields() -> TestResult {
    // A person id is tenant-wide; naming a source instance is what selects the
    // 'id' mode instead, so the combination is a contradiction, not a filter.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller6@http-live.test").await?;

    let (status, _) = post(
        app(&f, caller),
        "/v1/profiles",
        &json!({
            "value_type": "person_id",
            "value": caller.to_string(),
            "insight_source_type": "bamboohr",
            "insight_source_id": Uuid::now_v7().to_string(),
        }),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn an_unknown_value_type_is_a_client_error() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("caller7@http-live.test").await?;

    let (status, _) = post(
        app(&f, caller),
        "/v1/profiles",
        &json!({"value_type": "person-id", "value": caller.to_string()}),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}
