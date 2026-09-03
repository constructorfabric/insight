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

use std::collections::{BTreeMap, HashMap};
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
use crate::config::{GearConfig, VisibilityPolicy};
use crate::domain::resolution::EXCLUDED_PERSON;
use crate::infra::db::test_fixture::{
    FIXTURE_REASON, Fixture, ProjectedPerson, SOURCE_TYPE, fixture_or_skip,
};
use crate::infra::db::{ops_repo, person_roles_repo, resolution_repo, roles_repo, visibility_repo};

type TestResult = anyhow::Result<()>;

/// The caller the injected `SecurityContext` speaks for.
#[derive(Clone, Copy)]
struct Caller {
    person_id: Uuid,
    tenant: Uuid,
}

fn app(f: &Fixture, caller: Uuid) -> Router {
    app_with(f, caller, GearConfig::default())
}

fn flat_app(f: &Fixture, caller: Uuid) -> Router {
    app_with(
        f,
        caller,
        GearConfig {
            visibility_policy: VisibilityPolicy::Flat,
            ..GearConfig::default()
        },
    )
}

fn app_with(f: &Fixture, caller: Uuid, config: GearConfig) -> Router {
    app_for(
        f,
        Caller {
            person_id: caller,
            tenant: f.tenant,
        },
        config,
    )
}

fn app_for(f: &Fixture, caller: Caller, config: GearConfig) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    let state = Arc::new(AppState {
        db: f.db.clone(),
        config,
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

async fn get(app: Router, uri: &str) -> anyhow::Result<(StatusCode, Value)> {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await?;
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, payload))
}

/// `person_id`s of a listing page, in the order it served them.
fn listed_ids(payload: &Value) -> Vec<String> {
    payload["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["person_id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
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

fn batch_person_ids(person_ids: &[Uuid]) -> Value {
    json!({"person_ids": person_ids})
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

// ── POST /v1/profiles/batch ─────────────────────────────────

#[tokio::test]
async fn batch_profiles_preserves_order_and_omits_hidden_or_unknown_people() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let manager = f.person("batch-manager@http-live.test").await?;
    let github_only = f.emailless_person().await?;
    let hidden = f.person("batch-hidden@http-live.test").await?;
    f.observed_from("github", github_only, "username", "github-only")
        .await?;
    f.observed(github_only, "department", "Engineering").await?;
    f.observed(manager, "display_name", "Batch Manager").await?;
    f.reports_to(github_only, manager).await?;

    let unknown = Uuid::now_v7();
    let (status, body) = post(
        app(&f, manager),
        "/v1/profiles/batch",
        &batch_person_ids(&[hidden, github_only, unknown, manager]),
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    let profiles = body["profiles"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("missing profiles: {body}"))?;
    let ids = profiles
        .iter()
        .filter_map(|profile| profile["person_id"].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![github_only.to_string(), manager.to_string()]);
    assert_eq!(profiles[0]["attributes"]["username"], "github-only");
    assert_eq!(profiles[0]["attributes"]["department"], "Engineering");
    assert!(profiles[0]["attributes"].get("email").is_none());
    assert_eq!(profiles[0]["supervisor"]["person_id"], manager.to_string());
    assert_eq!(
        profiles[0]["supervisor"]["attributes"]["display_name"],
        "Batch Manager"
    );
    for forbidden in ["insight_tenant_id", "ids", "subordinates"] {
        assert!(
            profiles[0].get(forbidden).is_none(),
            "must omit {forbidden}"
        );
    }
    assert!(profiles[1].get("supervisor").is_none());
    Ok(())
}

#[tokio::test]
async fn batch_profiles_apply_flat_visibility_with_tenant_isolation() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("batch-flat-caller@http-live.test").await?;
    let unrelated = f.person("batch-flat-unrelated@http-live.test").await?;
    let foreign = f
        .in_another_tenant()
        .person("batch-flat-foreign@http-live.test")
        .await?;

    let (status, body) = post(
        flat_app(&f, caller),
        "/v1/profiles/batch",
        &batch_person_ids(&[foreign, unrelated]),
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["profiles"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("missing profiles: {body}"))?
            .iter()
            .filter_map(|profile| profile["person_id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>(),
        vec![unrelated.to_string()]
    );
    Ok(())
}

#[tokio::test]
async fn batch_profiles_reject_duplicate_and_unknown_request_fields() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("batch-invalid-caller@http-live.test").await?;

    for body in [
        batch_person_ids(&[caller, caller]),
        json!({"person_ids": [caller], "unexpected": true}),
    ] {
        let (status, _) = post(app(&f, caller), "/v1/profiles/batch", &body).await?;
        assert_eq!(status, StatusCode::BAD_REQUEST, "should reject: {body}");
    }
    Ok(())
}

// ── GET /v1/me ──────────────────────────────────────────────

async fn grant_admin(f: &Fixture, person: Uuid) -> anyhow::Result<Uuid> {
    let grant = Uuid::now_v7();
    person_roles_repo::insert(
        &f.db,
        grant,
        f.tenant,
        person,
        roles_repo::ADMIN_ROLE_ID,
        None,
        person,
        Some("http-live fixture"),
    )
    .await?;
    Ok(grant)
}

fn role_names(payload: &Value) -> anyhow::Result<Vec<String>> {
    // Fails on a missing/renamed field instead of defaulting to empty, so an
    // "expects no roles" assertion cannot pass vacuously on shape drift.
    let roles = payload["roles"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no `roles` array in {payload}"))?;
    Ok(roles
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_owned))
        .collect())
}

#[tokio::test]
async fn me_names_the_caller_and_their_active_admin_role() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("me-admin@http-live.test").await?;
    grant_admin(&f, caller).await?;

    let (status, body) = get(app(&f, caller), "/v1/me").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["person_id"], caller.to_string());
    assert_eq!(body["insight_tenant_id"], f.tenant.to_string());
    assert_eq!(role_names(&body)?, vec!["admin".to_owned()]);
    assert_eq!(
        body["roles"][0]["role_id"],
        roles_repo::ADMIN_ROLE_ID.to_string()
    );
    Ok(())
}

#[tokio::test]
async fn me_with_no_grants_is_an_empty_list_not_an_error() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("me-plain@http-live.test").await?;

    let (status, body) = get(app(&f, caller), "/v1/me").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(role_names(&body)?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn me_omits_a_revoked_grant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("me-revoked@http-live.test").await?;
    let grant = grant_admin(&f, caller).await?;
    person_roles_repo::soft_delete(&f.db, f.tenant, grant, None).await?;

    let (status, body) = get(app(&f, caller), "/v1/me").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(role_names(&body)?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn me_omits_a_grant_from_another_tenant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("me-crosstenant@http-live.test").await?;
    grant_admin(&f.in_another_tenant(), caller).await?;

    let (status, body) = get(app(&f, caller), "/v1/me").await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(role_names(&body)?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn me_without_a_caller_is_unauthenticated() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };

    let (status, _) = get(app(&f, Uuid::nil()), "/v1/me").await?;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    Ok(())
}

// ── GET /v1/persons ─────────────────────────────────────────

fn found_ids(payload: &Value) -> anyhow::Result<Vec<String>> {
    // Same shape-drift guard as `role_names`.
    let items = payload["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no `items` array in {payload}"))?;
    Ok(items
        .iter()
        .filter_map(|i| i["person_id"].as_str().map(str::to_owned))
        .collect())
}

#[tokio::test]
async fn people_lists_only_current_roster_people_visible_to_the_caller() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("people-caller@http-live.test").await?;
    let marker = format!("find-{}", Uuid::now_v7().simple());
    let report = f.person("people-report@http-live.test").await?;
    f.project_person(
        report,
        Some("canonical@http-live.test"),
        Some("canonical-handle"),
        Some(&format!("Canonical {marker}")),
        Some("Canonical"),
        Some("Person"),
    )
    .await?;
    let profile_name = format!("Canonical {marker}");
    let profile_attributes = BTreeMap::from([("department".to_owned(), "Engineering".to_owned())]);
    f.project_person_with_attributes(
        report,
        ProjectedPerson {
            email: Some("canonical@http-live.test"),
            username: Some("canonical-handle"),
            display_name: Some(&profile_name),
            first_name: Some("Canonical"),
            last_name: Some("Person"),
            attributes: &profile_attributes,
        },
    )
    .await?;
    f.reports_to(report, caller).await?;
    let identity_only = f
        .identity_only_person(&format!("observed-{marker}@http-live.test"))
        .await?;
    let unrelated = f
        .person(&format!("unrelated-{marker}@http-live.test"))
        .await?;

    let (status, body) = get(app(&f, caller), &format!("/v1/people?q={marker}")).await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, vec![report.to_string()]);
    let item = &body["items"][0];
    assert_eq!(item["display_name"], format!("Canonical {marker}"));
    assert_eq!(item["first_name"], "Canonical");
    assert_eq!(item["last_name"], "Person");
    assert_eq!(item["username"], "canonical-handle");
    assert_eq!(item["email"], "canonical@http-live.test");
    assert_eq!(item["attributes"]["department"], "Engineering");
    assert_eq!(item["manager_person_id"], caller.to_string());
    for legacy_field in ["provisional", "job_title", "status"] {
        assert!(
            item.get(legacy_field).is_none(),
            "unexpected {legacy_field}: {item}"
        );
    }
    assert!(
        !found_ids(&body)?.contains(&identity_only.to_string()),
        "identity evidence without a people projection is not roster membership"
    );
    assert!(
        !found_ids(&body)?.contains(&unrelated.to_string()),
        "caller visibility excludes unrelated roster people"
    );
    Ok(())
}

#[tokio::test]
async fn people_preserve_source_names_and_only_synthesize_a_missing_display_name() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("people-names-caller@http-live.test").await?;
    let marker = format!("find-{}", Uuid::now_v7().simple());
    let explicit = f
        .person(&format!("people-names-explicit-{marker}@http-live.test"))
        .await?;
    let parts = f
        .person(&format!("people-names-parts-{marker}@http-live.test"))
        .await?;
    let single = f
        .person(&format!("people-names-single-{marker}@http-live.test"))
        .await?;
    let explicit_display = format!("Source Display {marker}");
    let parts_last = format!("Parts {marker}");
    let single_first = format!("Mononym {marker}");
    f.project_person(
        explicit,
        None,
        None,
        Some(&explicit_display),
        Some("Separate"),
        Some("Parts"),
    )
    .await?;
    f.project_person(
        parts,
        None,
        None,
        None,
        Some("Available"),
        Some(&parts_last),
    )
    .await?;
    f.project_person(single, None, None, None, Some(&single_first), None)
        .await?;

    let (status, body) = get(flat_app(&f, caller), &format!("/v1/people?q={marker}")).await?;

    assert_eq!(status, StatusCode::OK);
    let items = body["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no `items` array in {body}"))?;
    let by_id = items
        .iter()
        .filter_map(|item| {
            item["person_id"]
                .as_str()
                .map(|person_id| (person_id, item))
        })
        .collect::<HashMap<_, _>>();
    let explicit_id = explicit.to_string();
    let parts_id = parts.to_string();
    let single_id = single.to_string();
    assert_eq!(
        by_id[explicit_id.as_str()]["display_name"],
        explicit_display
    );
    assert_eq!(by_id[explicit_id.as_str()]["first_name"], "Separate");
    assert_eq!(by_id[explicit_id.as_str()]["last_name"], "Parts");
    assert_eq!(
        by_id[parts_id.as_str()]["display_name"],
        format!("Available {parts_last}")
    );
    assert_eq!(by_id[parts_id.as_str()]["first_name"], "Available");
    assert_eq!(by_id[parts_id.as_str()]["last_name"], parts_last);
    assert_eq!(by_id[single_id.as_str()]["display_name"], single_first);
    assert_eq!(by_id[single_id.as_str()]["first_name"], single_first);
    assert!(by_id[single_id.as_str()]["last_name"].is_null());
    Ok(())
}

#[tokio::test]
async fn people_org_chart_visibility_does_not_widen_without_an_edge() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let marker = format!("find-{}", Uuid::now_v7().simple());
    let caller = f.person(&format!("caller-{marker}@http-live.test")).await?;
    let other = f.person(&format!("other-{marker}@http-live.test")).await?;
    let uri = format!("/v1/people?q={marker}");

    let (status, body) = get(app(&f, caller), &uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, vec![caller.to_string()]);

    let (status, body) = get(flat_app(&f, caller), &uri).await?;
    assert_eq!(status, StatusCode::OK);
    let mut ids = found_ids(&body)?;
    ids.sort();
    let mut expected = vec![caller.to_string(), other.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
    Ok(())
}

#[tokio::test]
async fn people_hide_a_manager_outside_the_callers_visible_set() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("manager-hidden-caller@http-live.test").await?;
    let marker = format!("find-{}", Uuid::now_v7().simple());
    let report = f
        .person(&format!("manager-hidden-report-{marker}@http-live.test"))
        .await?;
    let manager = f
        .person(&format!("manager-hidden-manager-{marker}@http-live.test"))
        .await?;
    f.reports_to(report, manager).await?;
    f.grant(caller, Some(report)).await?;

    let (status, body) = get(app(&f, caller), &format!("/v1/people?q={marker}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, vec![report.to_string()]);
    assert!(body["items"][0]["manager_person_id"].is_null());
    Ok(())
}

#[tokio::test]
async fn tenant_people_visibility_requires_admin_and_lists_the_tenant_roster() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("people-plain@http-live.test").await?;
    let marker = Uuid::now_v7().simple().to_string();
    let unrelated = f.person(&format!("tenant-{marker}@http-live.test")).await?;
    let uri = format!("/v1/people?visibility=tenant&q=tenant-{marker}");

    let (status, _) = get(app(&f, caller), &uri).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    grant_admin(&f, caller).await?;
    let (status, body) = get(app(&f, caller), &uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, vec![unrelated.to_string()]);

    let (status, _) = get(app(&f, caller), "/v1/people?visibility=all").await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn people_cursor_resumes_only_the_listing_that_issued_it() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("people-page-caller@http-live.test").await?;
    let marker = format!("find-{}", Uuid::now_v7().simple());
    for index in 0..2 {
        let person = f
            .person(&format!("people-page-{marker}-{index}@http-live.test"))
            .await?;
        f.project_person(
            person,
            None,
            None,
            Some(&format!("People Page {marker} {index}")),
            None,
            None,
        )
        .await?;
    }

    let uri = format!("/v1/people?q={marker}&limit=1");
    let (status, first) = get(flat_app(&f, caller), &uri).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&first)?.len(), 1);
    let cursor = first["next_cursor"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("expected next cursor: {first}"))?;

    let (status, second) = get(flat_app(&f, caller), &format!("{uri}&cursor={cursor}")).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&second)?.len(), 1);
    assert_ne!(found_ids(&first)?, found_ids(&second)?);

    grant_admin(&f, caller).await?;
    let (status, _) = get(
        flat_app(&f, caller),
        &format!("/v1/people?visibility=tenant&q={marker}&limit=1&cursor={cursor}"),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

async fn admin_operator(f: &Fixture) -> anyhow::Result<Uuid> {
    let operator = f.person("operator@http-live.test").await?;
    grant_admin(f, operator).await?;
    Ok(operator)
}

#[tokio::test]
async fn persons_search_requires_the_admin_row() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let plain = f.person("plain-searcher@http-live.test").await?;

    let (status, _) = get(app(&f, plain), "/v1/persons?q=anything").await?;

    assert_eq!(status, StatusCode::FORBIDDEN);
    Ok(())
}

#[tokio::test]
async fn a_request_without_terms_lists_the_roster() -> TestResult {
    // No terms is not a malformed search, it is the console's person mode:
    // an operator reviewing identities needs to see who exists rather than
    // guess a name to type.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let listed = f.person("rostered@http-live.test").await?;

    for uri in ["/v1/persons", "/v1/persons?q=%20%20"] {
        let (status, body) = get(app(&f, operator), uri).await?;

        assert_eq!(status, StatusCode::OK, "should list: {uri}");
        assert!(
            found_ids(&body)?.contains(&listed.to_string()),
            "the roster is missing a person of the tenant: {uri}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_person_is_found_by_a_current_email_fragment() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let marker = Uuid::now_v7().simple().to_string();
    let person = f.person(&format!("find-{marker}@http-live.test")).await?;

    let (status, body) = get(app(&f, operator), &format!("/v1/persons?q=find-{marker}")).await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, vec![person.to_string()]);
    assert_eq!(
        body["items"][0]["email"],
        format!("find-{marker}@http-live.test")
    );
    Ok(())
}

#[tokio::test]
async fn a_superseded_email_no_longer_finds_its_old_owner() -> TestResult {
    // The moved-mailbox case: the old owner observed a NEWER email from the
    // same source, so the old address is theirs no more — only the person the
    // address currently belongs to comes back.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let marker = Uuid::now_v7().simple().to_string();
    let moved = format!("moved-{marker}@http-live.test");

    let old_owner = f.person(&moved).await?;
    f.observed(
        old_owner,
        "email",
        &format!("fresh-{marker}@http-live.test"),
    )
    .await?;
    let new_owner = f.person(&moved).await?;

    let (status, body) = get(app(&f, operator), &format!("/v1/persons?q=moved-{marker}")).await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        found_ids(&body)?,
        vec![new_owner.to_string()],
        "the old owner's claim is superseded by their own fresher email"
    );
    Ok(())
}

#[tokio::test]
async fn a_still_current_email_two_persons_claim_returns_both() -> TestResult {
    // The handed-over mailbox with no fresher data for the old owner: the
    // journal honestly says two persons currently claim it, and the operator
    // is the one who gets to disambiguate.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let marker = Uuid::now_v7().simple().to_string();
    let shared = format!("shared-{marker}@http-live.test");

    let first = f.person(&shared).await?;
    let second = f.person(&shared).await?;

    let (status, body) = get(app(&f, operator), &format!("/v1/persons?q=shared-{marker}")).await?;

    assert_eq!(status, StatusCode::OK);
    let mut ids = found_ids(&body)?;
    ids.sort();
    let mut expected = vec![first.to_string(), second.to_string()];
    expected.sort();
    assert_eq!(ids, expected);
    Ok(())
}

#[tokio::test]
async fn every_term_must_match_though_not_the_same_value() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let marker = Uuid::now_v7().simple().to_string();
    let person = f.person(&format!("multi-{marker}@http-live.test")).await?;
    f.observed(person, "display_name", &format!("Terman Findable {marker}"))
        .await?;

    let by_both = format!("/v1/persons?q=multi-{marker}%20findable");
    let (status, body) = get(app(&f, operator), &by_both).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, vec![person.to_string()]);
    assert_eq!(
        body["items"][0]["display_name"],
        format!("Terman Findable {marker}")
    );

    let with_a_miss = format!("/v1/persons?q=multi-{marker}%20nosuchterm");
    let (status, body) = get(app(&f, operator), &with_a_miss).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?, Vec::<String>::new());
    Ok(())
}

#[tokio::test]
async fn the_listing_is_ordered_by_the_label_each_row_shows() -> TestResult {
    // Alphabetical by the label the row displays — display name, else the
    // address — so the order can be followed down the column. A person the
    // journal knows by nothing but a binding has no label to place, and sits
    // after everyone who has one rather than under an id nobody reads.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let marker = Uuid::now_v7().simple().to_string();

    // The addresses and the name are deliberately in opposite orders: `named`
    // has the EARLIER address and the LATER name. Sorting by the address would
    // put them first, sorting by the label they show puts them second — so the
    // assertion can tell the two rules apart, which a same-direction pair
    // cannot.
    let email_only = f
        .person(&format!("mmm-order-{marker}@http-live.test"))
        .await?;
    let named = f
        .person(&format!("aaa-order-{marker}@http-live.test"))
        .await?;
    f.observed(named, "display_name", &format!("Zzz Named {marker}"))
        .await?;
    let unlabelled = f.emailless_person().await?;

    let (status, body) = get(app(&f, operator), "/v1/persons").await?;

    assert_eq!(status, StatusCode::OK);
    let expected = [email_only, named, unlabelled].map(|id| id.to_string());
    let listed: Vec<String> = found_ids(&body)?
        .into_iter()
        .filter(|id| expected.contains(id))
        .collect();
    assert_eq!(
        listed,
        expected.to_vec(),
        "the displayed name places the named person, not their address, and the \
         label-less person comes last"
    );
    Ok(())
}

#[tokio::test]
async fn the_roster_leaves_out_the_person_that_stands_for_exclusion() -> TestResult {
    // Excluding an account binds it to a sentinel person id. It is a marker, not
    // somebody: listed, it would be a nameless row an operator could pick as a
    // bind or merge target, and every excluded account would look like theirs.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let account = format!("excluded-{}", Uuid::now_v7().simple());
    f.bound_at(&account, EXCLUDED_PERSON, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = get(app(&f, operator), "/v1/persons").await?;

    assert_eq!(status, StatusCode::OK);
    assert!(
        !found_ids(&body)?.contains(&EXCLUDED_PERSON.to_string()),
        "the exclusion sentinel is not a person to list"
    );
    Ok(())
}

#[tokio::test]
async fn a_cut_page_offers_the_next_one_and_a_whole_answer_does_not() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let operator = admin_operator(&f).await?;
    let marker = Uuid::now_v7().simple().to_string();
    for i in 0..3 {
        f.person(&format!("cut-{marker}-{i}@http-live.test"))
            .await?;
    }

    let (status, body) = get(
        app(&f, operator),
        &format!("/v1/persons?q=cut-{marker}&limit=2"),
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(found_ids(&body)?.len(), 2, "the page honours the limit");
    assert!(
        body["next_cursor"].is_string(),
        "a cut page offers the way on: {body}"
    );

    let (_, full) = get(app(&f, operator), &format!("/v1/persons?q=cut-{marker}")).await?;
    assert_eq!(found_ids(&full)?.len(), 3);
    assert!(
        full["next_cursor"].is_null(),
        "a whole answer offers no next page: {full}"
    );
    Ok(())
}

#[tokio::test]
async fn a_flat_policy_resolves_a_profile_outside_the_reporting_line() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("flat-caller@http-live.test").await?;
    let unrelated = f.person("flat-unrelated@http-live.test").await?;

    let (org_chart, _) = post(app(&f, caller), "/v1/profiles", &by_person_id(unrelated)).await?;
    let (flat, _) = post(
        flat_app(&f, caller),
        "/v1/profiles",
        &by_person_id(unrelated),
    )
    .await?;

    assert_eq!(
        org_chart,
        StatusCode::NOT_FOUND,
        "reporting-line rule hides them"
    );
    assert_eq!(flat, StatusCode::OK, "flat policy resolves them");
    Ok(())
}

#[tokio::test]
async fn a_flat_policy_keeps_another_tenants_profile_not_found() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("flat-boundary@http-live.test").await?;
    let foreign = f
        .in_another_tenant()
        .person("flat-foreign@http-live.test")
        .await?;

    let (status, _) = post(flat_app(&f, caller), "/v1/profiles", &by_person_id(foreign)).await?;

    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn the_roster_lists_the_caller_and_everyone_the_policy_shows_them() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.account_holder("roster-caller@http-live.test").await?;
    let other = f.account_holder("roster-other@http-live.test").await?;

    let (status, body) = get(flat_app(&f, caller), "/v1/visible-persons").await?;

    assert_eq!(status, StatusCode::OK);
    let listed = listed_ids(&body);
    for person in [caller, other] {
        assert!(
            listed.contains(&person.to_string()),
            "missing {person}: {body}"
        );
    }
    assert!(body["next_cursor"].is_null(), "one page held them all");
    Ok(())
}

#[tokio::test]
async fn the_roster_leaves_out_an_identity_nobody_claims() -> TestResult {
    // The shape a git-only organisation is full of: an address a commit carried,
    // which the journal turned into a person because that is what an observation
    // log does. No connector ever claimed it as an account, so it is not a
    // member, and a roster listing it is a directory of strangers.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let holder = f.account_holder("roster-holder@http-live.test").await?;
    let observed_only = f.person("roster-observed@http-live.test").await?;

    let (status, body) = get(flat_app(&f, holder), "/v1/visible-persons").await?;

    assert_eq!(status, StatusCode::OK);
    let listed = listed_ids(&body);
    assert!(
        listed.contains(&holder.to_string()),
        "the account holder IS the roster: {body}"
    );
    assert!(
        !listed.contains(&observed_only.to_string()),
        "an address nobody claims is not a member: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn a_roster_cursor_resumes_after_the_page_it_was_issued_for() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let first_person = f.account_holder("roster-aaa@http-live.test").await?;
    let second_person = f.account_holder("roster-bbb@http-live.test").await?;

    let (status, first) = get(flat_app(&f, first_person), "/v1/visible-persons?limit=1").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed_ids(&first), vec![first_person.to_string()]);

    let cursor = first["next_cursor"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("a cut page must carry a cursor: {first}"))?;
    let (status, next) = get(
        flat_app(&f, first_person),
        &format!("/v1/visible-persons?limit=1&cursor={cursor}"),
    )
    .await?;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        listed_ids(&next),
        vec![second_person.to_string()],
        "the row after the last one served, never it again"
    );
    Ok(())
}

#[tokio::test]
async fn a_sign_in_minted_person_is_listed_by_their_login_and_marked() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("roster-named@http-live.test").await?;
    let minted = f.login_minted_person("octocat-probe").await?;

    let (status, body) = get(flat_app(&f, caller), "/v1/visible-persons").await?;

    assert_eq!(status, StatusCode::OK);
    let entry = body["items"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["person_id"].as_str() == Some(&minted.to_string()))
        })
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("a login-minted person is still a person: {body}"))?;

    assert_eq!(entry["provisional"], true, "offered, and marked as thin");
    Ok(())
}

#[tokio::test]
async fn a_cursor_from_another_query_is_refused_rather_than_resumed() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.account_holder("roster-cursor@http-live.test").await?;
    let _second = f.account_holder("roster-cursor-2@http-live.test").await?;

    let (_, browsed) = get(flat_app(&f, caller), "/v1/visible-persons?limit=1").await?;
    let cursor = browsed["next_cursor"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("expected a cursor: {browsed}"))?;

    // The same position, presented under a narrowed query it never ordered.
    let (status, _) = get(
        flat_app(&f, caller),
        &format!("/v1/visible-persons?q=roster&limit=1&cursor={cursor}"),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn an_over_long_roster_query_is_refused_rather_than_scanned() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("roster-q@http-live.test").await?;

    let (status, _) = get(
        flat_app(&f, caller),
        &format!("/v1/visible-persons?q={}", "x".repeat(201)),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

// ── POST /v1/resolution/{bind,merge,detach,exclude} ─────────
//
// INVARIANT: every case seeds the account's binding first. The other way in —
// an account no binding names, vouched for by connector evidence — is a
// ClickHouse read, and this suite has none.

/// A caller who may correct: the admin grant is what every one of these needs.
async fn operator(f: &Fixture) -> anyhow::Result<Uuid> {
    let person = f.person("operator@http-live.test").await?;
    grant_admin(f, person).await?;
    Ok(person)
}

fn account_ref(f: &Fixture, account_id: &str) -> Value {
    json!({
        "source": SOURCE_TYPE,
        "source_id": f.source_id.to_string(),
        "id": account_id,
    })
}

/// The person the account is bound to right now. This is the by-name read the
/// correction itself performs; the review surface asks the same question of the
/// whole tenant through a different statement, and nothing here corroborates the
/// two against each other.
async fn bound_to(f: &Fixture, account_id: &str) -> anyhow::Result<Option<Uuid>> {
    let key = f.account(account_id);
    Ok(
        resolution_repo::current_bindings(&f.db, f.tenant, std::slice::from_ref(&key))
            .await?
            .get(&key)
            .map(|b| b.person_id),
    )
}

fn bind_body(f: &Fixture, account_id: &str, person: Uuid) -> Value {
    json!({
        "bindings": [{"account": account_ref(f, account_id), "person_id": person.to_string()}],
        "comment": "http-live",
    })
}

#[tokio::test]
async fn binding_an_account_puts_the_named_person_in_force() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let automation_chose = f.person("wrong@http-live.test").await?;
    let operator_means = f.person("right@http-live.test").await?;
    f.bound_at("acct-bind", automation_chose, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &bind_body(&f, "acct-bind", operator_means),
    )
    .await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["applied"], 1);
    assert_eq!(body["items"][0]["outcome"], "applied");
    assert_eq!(
        bound_to(&f, "acct-bind").await?,
        Some(operator_means),
        "the operator's decision must be the binding in force"
    );
    Ok(())
}

#[tokio::test]
async fn binding_a_decision_already_recorded_applies_nothing_further() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("settled@http-live.test").await?;
    f.bound_at("acct-again", person, FIXTURE_REASON, 60).await?;

    let body = bind_body(&f, "acct-again", person);
    // The first call is the confirm act — recording the operator's agreement
    // with what automation chose — and it must APPLY, not report nothing to do.
    let (_, first) = post(app(&f, caller), "/v1/resolution/bind", &body).await?;
    assert_eq!(
        first["applied"], 1,
        "confirming a binding is a decision: {first}"
    );

    let (status, second) = post(app(&f, caller), "/v1/resolution/bind", &body).await?;

    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["applied"], 0);
    assert_eq!(second["already_decided"], 1);
    assert_eq!(second["items"][0]["outcome"], "already_decided");
    Ok(())
}

#[tokio::test]
async fn one_call_naming_an_account_twice_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let held_by = f.person("held-by@http-live.test").await?;
    let one = f.person("one@http-live.test").await?;
    let other = f.person("other@http-live.test").await?;
    // The seed names a THIRD person: were the first item written before the
    // refusal, the account would move, and an assertion naming the first item's
    // person could not tell that apart from nothing having happened.
    f.bound_at("acct-twice", held_by, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &json!({"bindings": [
            {"account": account_ref(&f, "acct-twice"), "person_id": one.to_string()},
            {"account": account_ref(&f, "acct-twice"), "person_id": other.to_string()},
        ]}),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        bound_to(&f, "acct-twice").await?,
        Some(held_by),
        "a refused call must leave the account where it was"
    );
    Ok(())
}

#[tokio::test]
async fn a_bind_naming_no_accounts_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;

    let (status, _) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &json!({"bindings": []}),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn binding_to_the_excluded_sentinel_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("sentinel-target@http-live.test").await?;
    f.bound_at("acct-sentinel", person, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &bind_body(&f, "acct-sentinel", EXCLUDED_PERSON),
    )
    .await?;

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the sentinel is reachable only through the exclude verb: {body}"
    );
    Ok(())
}

#[tokio::test]
async fn binding_to_a_person_the_tenant_never_had_is_not_found() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let stranger = f
        .in_another_tenant()
        .person("stranger@http-live.test")
        .await?;
    let holder = f.person("holder@http-live.test").await?;
    f.bound_at("acct-stranger", holder, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &bind_body(&f, "acct-stranger", stranger),
    )
    .await?;

    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(bound_to(&f, "acct-stranger").await?, Some(holder));
    Ok(())
}

#[tokio::test]
async fn a_caller_without_the_admin_grant_cannot_correct() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("not-admin@http-live.test").await?;
    let person = f.person("untouched@http-live.test").await?;
    f.bound_at("acct-guarded", person, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &bind_body(&f, "acct-guarded", caller),
    )
    .await?;

    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        bound_to(&f, "acct-guarded").await?,
        Some(person),
        "a refused caller must not have moved anything"
    );
    Ok(())
}

#[tokio::test]
async fn merge_moves_every_account_of_the_absorbed_person() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let absorbed = f.person("absorbed@http-live.test").await?;
    let survivor = f.person("survivor@http-live.test").await?;
    f.bound_at("acct-m1", absorbed, FIXTURE_REASON, 60).await?;
    f.bound_at("acct-m2", absorbed, FIXTURE_REASON, 60).await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/merge",
        &json!({
            "source_person_id": absorbed.to_string(),
            "target_person_id": survivor.to_string(),
        }),
    )
    .await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["applied"], 2, "both accounts move, not just one");
    assert_eq!(bound_to(&f, "acct-m1").await?, Some(survivor));
    assert_eq!(bound_to(&f, "acct-m2").await?, Some(survivor));
    Ok(())
}

#[tokio::test]
async fn merging_a_person_with_themselves_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("alone@http-live.test").await?;

    let (status, _) = post(
        app(&f, caller),
        "/v1/resolution/merge",
        &json!({
            "source_person_id": person.to_string(),
            "target_person_id": person.to_string(),
        }),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn detach_moves_the_account_to_the_person_the_answer_names() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let was = f.person("shared-identity@http-live.test").await?;
    f.bound_at("acct-detach", was, FIXTURE_REASON, 60).await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/detach",
        &json!({"account": account_ref(&f, "acct-detach")}),
    )
    .await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    let minted: Uuid = body["new_person_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("detach named no person: {body}"))?
        .parse()?;
    assert_ne!(
        minted, was,
        "a detach that returns the old person moved nothing"
    );
    assert_eq!(
        bound_to(&f, "acct-detach").await?,
        Some(minted),
        "the account must hold the person the answer named"
    );
    Ok(())
}

#[tokio::test]
async fn excluding_an_account_binds_it_to_the_sentinel() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let looked_human = f.person("ci-bot@http-live.test").await?;
    f.bound_at("acct-bot", looked_human, FIXTURE_REASON, 60)
        .await?;

    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/exclude",
        &json!({"account": account_ref(&f, "acct-bot")}),
    )
    .await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        bound_to(&f, "acct-bot").await?,
        Some(EXCLUDED_PERSON),
        "an excluded account answers with the sentinel, not its former person"
    );
    Ok(())
}

#[tokio::test]
async fn a_correction_names_its_operator_in_the_journal() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("journalled@http-live.test").await?;
    f.bound_at("acct-journal", person, FIXTURE_REASON, 60)
        .await?;
    let target = f.person("journal-target@http-live.test").await?;

    post(
        app(&f, caller),
        "/v1/resolution/bind",
        &bind_body(&f, "acct-journal", target),
    )
    .await?;

    // The trail an operator is answerable by: who decided, and what they asked
    // for. Nothing else records the author of a correction.
    let trail = ops_repo::corrections_for_account(
        &f.db,
        f.tenant,
        crate::api::resolution::RESOLUTION_OP,
        SOURCE_TYPE,
        f.source_id,
        "acct-journal",
        10,
    )
    .await?;

    assert_eq!(
        trail.len(),
        1,
        "the call must leave exactly one journal row"
    );
    assert_eq!(trail[0].author_person_id, caller);
    let request: Value = serde_json::from_str(
        trail[0]
            .request_json
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("journal row carries no request"))?,
    )?;
    assert_eq!(request["verb"], "operator-bind");
    assert_eq!(request["target_person_id"], target.to_string());
    Ok(())
}

#[tokio::test]
async fn a_bulk_call_moves_every_account_it_names() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let automation_chose = f.person("bulk-wrong@http-live.test").await?;
    let first = f.person("bulk-first@http-live.test").await?;
    let second = f.person("bulk-second@http-live.test").await?;
    f.bound_at("acct-bulk-1", automation_chose, FIXTURE_REASON, 60)
        .await?;
    f.bound_at("acct-bulk-2", automation_chose, FIXTURE_REASON, 60)
        .await?;

    // A prepared matching table is submitted as ONE call, and the outcomes come
    // back by position — the shape the whole bulk contract exists for.
    let (status, body) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &json!({"bindings": [
            {"account": account_ref(&f, "acct-bulk-1"), "person_id": first.to_string()},
            {"account": account_ref(&f, "acct-bulk-2"), "person_id": second.to_string()},
        ]}),
    )
    .await?;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["applied"], 2, "a bulk call applies every item: {body}");
    assert_eq!(bound_to(&f, "acct-bulk-1").await?, Some(first));
    assert_eq!(
        bound_to(&f, "acct-bulk-2").await?,
        Some(second),
        "the second item is not the first item's echo"
    );
    Ok(())
}

#[tokio::test]
async fn a_bulk_call_beyond_the_ceiling_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("ceiling@http-live.test").await?;
    let bindings: Vec<Value> = (0..=super::resolution::MAX_BULK_ITEMS)
        .map(|i| json!({"account": account_ref(&f, &format!("acct-{i}")), "person_id": person.to_string()}))
        .collect();

    let (status, _) = post(
        app(&f, caller),
        "/v1/resolution/bind",
        &json!({"bindings": bindings}),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}

#[tokio::test]
async fn a_merge_naming_the_excluded_sentinel_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("merge-sentinel@http-live.test").await?;

    for body in [
        json!({"source_person_id": EXCLUDED_PERSON.to_string(), "target_person_id": person.to_string()}),
        json!({"source_person_id": person.to_string(), "target_person_id": EXCLUDED_PERSON.to_string()}),
    ] {
        let (status, answer) = post(app(&f, caller), "/v1/resolution/merge", &body).await?;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "the sentinel on either side would move every excluded account: {answer}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn each_verb_journals_the_decision_it_made() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let absorbed = f.person("verb-absorbed@http-live.test").await?;
    let survivor = f.person("verb-survivor@http-live.test").await?;
    let holder = f.person("verb-holder@http-live.test").await?;
    f.bound_at("acct-verb-merge", absorbed, FIXTURE_REASON, 60)
        .await?;
    f.bound_at("acct-verb-detach", holder, FIXTURE_REASON, 60)
        .await?;
    f.bound_at("acct-verb-exclude", holder, FIXTURE_REASON, 60)
        .await?;

    post(
        app(&f, caller),
        "/v1/resolution/merge",
        &json!({
            "source_person_id": absorbed.to_string(),
            "target_person_id": survivor.to_string(),
        }),
    )
    .await?;
    post(
        app(&f, caller),
        "/v1/resolution/detach",
        &json!({"account": account_ref(&f, "acct-verb-detach")}),
    )
    .await?;
    post(
        app(&f, caller),
        "/v1/resolution/exclude",
        &json!({"account": account_ref(&f, "acct-verb-exclude")}),
    )
    .await?;

    // The reason code is what makes a binding's history explain itself without
    // joining the operations log, so a verb that journals another verb's name
    // corrupts both records at once.
    for (account, verb) in [
        ("acct-verb-merge", "operator-merge"),
        ("acct-verb-detach", "operator-detach"),
        ("acct-verb-exclude", "operator-exclude"),
    ] {
        let trail = ops_repo::corrections_for_account(
            &f.db,
            f.tenant,
            crate::api::resolution::RESOLUTION_OP,
            SOURCE_TYPE,
            f.source_id,
            account,
            10,
        )
        .await?;
        assert_eq!(trail.len(), 1, "{account} must have journalled once");
        let request: Value = serde_json::from_str(
            trail[0]
                .request_json
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("{account}: journal row carries no request"))?,
        )?;
        assert_eq!(request["verb"], verb, "{account} journalled the wrong verb");
    }
    Ok(())
}

#[tokio::test]
async fn a_correction_without_a_caller_is_unauthenticated() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("no-caller@http-live.test").await?;
    f.bound_at("acct-anon", person, FIXTURE_REASON, 60).await?;

    let (status, _) = post(
        app(&f, Uuid::nil()),
        "/v1/resolution/bind",
        &bind_body(&f, "acct-anon", person),
    )
    .await?;

    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(bound_to(&f, "acct-anon").await?, Some(person));
    Ok(())
}

// ── /v1/{roles,person-roles,visibility} ─────────────────────
//
// The catalogue and the two grant surfaces. Two of their rules are enforced by
// a single conditional UPDATE rather than by a read-then-write, so only a live
// database can say whether they hold: a role in use cannot be deleted, and the
// last admin of a tenant cannot be revoked.

async fn delete(app: Router, uri: &str) -> anyhow::Result<(StatusCode, Value)> {
    let req = Request::builder()
        .method("DELETE")
        .uri(uri)
        .body(Body::empty())?;
    let resp = app.oneshot(req).await?;
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await?;
    let payload = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    Ok((status, payload))
}

/// A role nobody else's case can collide with.
async fn a_role(f: &Fixture, caller: Uuid, name: &str) -> anyhow::Result<Uuid> {
    let unique = format!("{name}-{}", Uuid::now_v7().simple());
    let (status, body) = post(app(f, caller), "/v1/roles", &json!({"name": unique})).await?;
    anyhow::ensure!(status == StatusCode::CREATED, "creating {unique}: {body}");
    let id = body["role_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no role_id in {body}"))?;
    Ok(id.parse()?)
}

/// Remove a role this case created. `roles` has no tenant column, so one left
/// behind is in every tenant's catalogue and in every later run's listing —
/// the fixture's fresh tenant does not isolate it.
async fn forget_role(f: &Fixture, role_id: Uuid) -> anyhow::Result<()> {
    let removed = roles_repo::try_delete_if_unused(&f.db, role_id).await?;
    anyhow::ensure!(removed == 1, "role {role_id} outlived its case");
    Ok(())
}

#[tokio::test]
async fn a_created_role_is_listed_and_a_second_of_that_name_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let name = format!("reviewer-{}", Uuid::now_v7().simple());

    let (created, body) = post(app(&f, caller), "/v1/roles", &json!({"name": name})).await?;
    assert_eq!(created, StatusCode::CREATED, "{body}");

    let (_, listed) = get(app(&f, caller), "/v1/roles").await?;
    let names: Vec<&str> = listed["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no items in {listed}"))?
        .iter()
        .filter_map(|r| r["name"].as_str())
        .collect();
    assert!(
        names.contains(&name.as_str()),
        "{name} missing from {listed}"
    );

    let (again, _) = post(app(&f, caller), "/v1/roles", &json!({"name": name})).await?;
    assert_eq!(
        again,
        StatusCode::CONFLICT,
        "the catalogue holds one of a name"
    );

    let role_id = body["role_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no role_id in {body}"))?;
    forget_role(&f, role_id.parse()?).await
}

#[tokio::test]
async fn a_role_nobody_holds_can_be_deleted_and_one_in_use_cannot() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let unused = a_role(&f, caller, "unused").await?;
    let held = a_role(&f, caller, "held").await?;
    let holder = f.person("role-holder@http-live.test").await?;

    let (granted, body) = post(
        app(&f, caller),
        "/v1/person-roles",
        &json!({"person_id": holder.to_string(), "role_id": held.to_string()}),
    )
    .await?;
    assert_eq!(granted, StatusCode::CREATED, "{body}");
    let assignment: Uuid = body["person_role_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no person_role_id in {body}"))?
        .parse()?;

    let (gone, _) = delete(app(&f, caller), &format!("/v1/roles/{unused}")).await?;
    assert_eq!(gone, StatusCode::NO_CONTENT);

    // The guard is a conditional UPDATE, not a read-then-write: a role somebody
    // holds must not be removable however the two calls interleave.
    let (refused, answer) = delete(app(&f, caller), &format!("/v1/roles/{held}")).await?;
    assert_eq!(
        refused,
        StatusCode::CONFLICT,
        "a role with a live assignment was deleted: {answer}"
    );
    assert_eq!(
        answer["context"]["reason"], "role_in_use",
        "the refusal must name itself: {answer}"
    );
    assert!(
        roles_repo::get_by_id(&f.db, held).await?.is_some(),
        "and it must still be in the catalogue"
    );

    // `unused` is gone already; `held` only becomes removable once nobody has it.
    person_roles_repo::soft_delete(&f.db, f.tenant, assignment, Some("http-live cleanup")).await?;
    forget_role(&f, held).await
}

#[tokio::test]
async fn the_last_admin_of_a_tenant_cannot_be_revoked() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("sole-admin@http-live.test").await?;
    let grant = grant_admin(&f, caller).await?;

    let (refused, answer) = delete(app(&f, caller), &format!("/v1/person-roles/{grant}")).await?;

    assert_eq!(
        refused,
        StatusCode::CONFLICT,
        "revoking the only admin locks the tenant out of its own operator surface: {answer}"
    );
    assert_eq!(
        answer["context"]["reason"], "last_admin_protected",
        "the refusal must name itself: {answer}"
    );
    let (status, me) = get(app(&f, caller), "/v1/me").await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        role_names(&me)?,
        vec!["admin".to_owned()],
        "the caller must still hold it"
    );
    Ok(())
}

#[tokio::test]
async fn an_admin_who_is_not_the_last_one_can_be_revoked() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("first-admin@http-live.test").await?;
    grant_admin(&f, caller).await?;
    let second = f.person("second-admin@http-live.test").await?;
    let second_grant = grant_admin(&f, second).await?;

    let (status, _) = delete(app(&f, caller), &format!("/v1/person-roles/{second_grant}")).await?;

    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "the guard protects the last admin, not every admin"
    );
    let (_, me) = get(app(&f, second), "/v1/me").await?;
    assert!(
        role_names(&me)?.is_empty(),
        "and the revoked assignment stops counting"
    );
    Ok(())
}

#[tokio::test]
async fn a_revoked_assignment_cannot_be_revoked_again() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("revoker@http-live.test").await?;
    grant_admin(&f, caller).await?;
    let other = f.person("revokee@http-live.test").await?;
    let grant = grant_admin(&f, other).await?;

    let (first, _) = delete(app(&f, caller), &format!("/v1/person-roles/{grant}")).await?;
    let (second, _) = delete(app(&f, caller), &format!("/v1/person-roles/{grant}")).await?;

    assert_eq!(first, StatusCode::NO_CONTENT);
    assert_eq!(
        second,
        StatusCode::NOT_FOUND,
        "an assignment that is already gone is not there to revoke"
    );
    Ok(())
}

#[tokio::test]
async fn a_visibility_grant_reaches_the_visible_set_and_leaves_it_when_revoked() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let viewer = f.person("viewer@http-live.test").await?;
    let target = f.person("target@http-live.test").await?;

    assert!(
        !f.can_see(viewer, target).await?,
        "nothing connects them to begin with"
    );

    let (created, body) = post(
        app(&f, caller),
        "/v1/visibility",
        &json!({
            "viewer_person_id": viewer.to_string(),
            "viewed_person_id": target.to_string(),
        }),
    )
    .await?;
    assert_eq!(created, StatusCode::CREATED, "{body}");
    let grant_id = body["visibility_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no visibility_id in {body}"))?;

    assert!(
        f.can_see(viewer, target).await?,
        "a grant the endpoint wrote must be one the visible set honours"
    );

    let (revoked, _) = delete(app(&f, caller), &format!("/v1/visibility/{grant_id}")).await?;

    assert_eq!(revoked, StatusCode::NO_CONTENT);
    assert!(
        !f.can_see(viewer, target).await?,
        "and revoking it must take the reach away again"
    );
    Ok(())
}

#[tokio::test]
async fn another_tenants_visibility_grant_is_neither_listed_nor_revocable() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let other = f.in_another_tenant();
    let their_admin = other.person("their-admin@http-live.test").await?;
    other.make_admin(their_admin).await?;
    let their_viewer = other.person("their-viewer@http-live.test").await?;

    let (created, body) = post(
        app(&other, their_admin),
        "/v1/visibility",
        &json!({"viewer_person_id": their_viewer.to_string()}),
    )
    .await?;
    assert_eq!(created, StatusCode::CREATED, "{body}");
    let theirs = body["visibility_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no visibility_id in {body}"))?;

    let (_, listed) = get(app(&f, caller), "/v1/visibility").await?;
    let ids: Vec<&str> = listed["items"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("no items in {listed}"))?
        .iter()
        .filter_map(|g| g["visibility_id"].as_str())
        .collect();
    assert!(!ids.contains(&theirs), "another tenant's grant was listed");

    let (status, _) = delete(app(&f, caller), &format!("/v1/visibility/{theirs}")).await?;

    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "and it must not be reachable for revocation either"
    );
    assert!(
        visibility_repo::get_by_id(&other.db, other.tenant, theirs.parse()?)
            .await?
            .is_some_and(|g| g.valid_to.is_none()),
        "the grant must still stand in the tenant that made it"
    );
    Ok(())
}

/// Every route behind `require_admin`, as one table — all 22 of them. Each
/// handler calls the gate separately, so a gate removed from one of them is
/// invisible to a case that only drives another.
///
/// INVARIANT: this must stay level with the `require_admin` call sites in
/// `api/`. Nothing derives it, so a gated route added and not listed here is a
/// gate no test drives.
fn operator_routes(f: &Fixture, person: Uuid) -> Vec<(&'static str, String, Option<Value>)> {
    let some = Uuid::now_v7();
    let account = json!({"account": account_ref(f, "acct-gate")});
    vec![
        (
            "POST",
            "/v1/resolution/bind".to_owned(),
            Some(bind_body(f, "acct-gate", person)),
        ),
        (
            "POST",
            "/v1/resolution/merge".to_owned(),
            Some(json!({
                "source_person_id": person.to_string(), "target_person_id": some.to_string(),
            })),
        ),
        (
            "POST",
            "/v1/resolution/detach".to_owned(),
            Some(account.clone()),
        ),
        ("POST", "/v1/resolution/exclude".to_owned(), Some(account)),
        ("GET", "/v1/resolution/attention".to_owned(), None),
        ("GET", "/v1/resolution/accounts".to_owned(), None),
        (
            "GET",
            format!(
                "/v1/resolution/accounts/{SOURCE_TYPE}/{}/acct-gate",
                f.source_id
            ),
            None,
        ),
        (
            "GET",
            format!("/v1/resolution/persons/{person}/accounts"),
            None,
        ),
        ("GET", "/v1/persons?q=anything".to_owned(), None),
        ("GET", "/v1/persons-seed".to_owned(), None),
        ("GET", format!("/v1/persons-seed/{some}"), None),
        ("GET", "/v1/persons-sync".to_owned(), None),
        ("GET", format!("/v1/persons-sync/{some}"), None),
        (
            "POST",
            "/v1/roles".to_owned(),
            Some(json!({"name": "gate-probe"})),
        ),
        ("GET", "/v1/roles".to_owned(), None),
        ("DELETE", format!("/v1/roles/{some}"), None),
        (
            "POST",
            "/v1/person-roles".to_owned(),
            Some(json!({
                "person_id": person.to_string(), "role_id": roles_repo::ADMIN_ROLE_ID.to_string(),
            })),
        ),
        ("GET", "/v1/person-roles".to_owned(), None),
        ("DELETE", format!("/v1/person-roles/{some}"), None),
        (
            "POST",
            "/v1/visibility".to_owned(),
            Some(json!({"viewer_person_id": person.to_string()})),
        ),
        ("GET", "/v1/visibility".to_owned(), None),
        ("DELETE", format!("/v1/visibility/{some}"), None),
    ]
}

async fn send(
    app: Router,
    method: &str,
    uri: &str,
    body: Option<&Value>,
) -> anyhow::Result<StatusCode> {
    let builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(b) => builder
            .header("content-type", "application/json")
            .body(Body::from(b.to_string()))?,
        None => builder.body(Body::empty())?,
    };
    Ok(app.oneshot(req).await?.status())
}

#[tokio::test]
async fn every_operator_route_refuses_a_caller_without_the_admin_grant() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = f.person("gate-plain@http-live.test").await?;
    f.bound_at("acct-gate", caller, FIXTURE_REASON, 60).await?;

    for (method, uri, body) in operator_routes(&f, caller) {
        let status = send(app(&f, caller), method, &uri, body.as_ref()).await?;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {uri} let a non-admin through"
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_admin_of_another_tenant_is_refused_here() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    // The grant is a row in THIS tenant, not a property of the person. Somebody
    // who administers one tenant is an ordinary caller in every other, and the
    // gate reads the row scoped to the tenant the gateway named.
    let other = f.in_another_tenant();
    let caller = other.person("admin-elsewhere@http-live.test").await?;
    other.make_admin(caller).await?;
    f.bound_at("acct-gate", caller, FIXTURE_REASON, 60).await?;

    for (method, uri, body) in operator_routes(&f, caller) {
        let status = send(app(&f, caller), method, &uri, body.as_ref()).await?;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{method} {uri} accepted an admin of another tenant"
        );
    }
    Ok(())
}

#[tokio::test]
async fn every_operator_route_refuses_a_caller_the_gateway_did_not_name() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let person = f.person("gate-anon@http-live.test").await?;
    f.bound_at("acct-gate", person, FIXTURE_REASON, 60).await?;

    for (method, uri, body) in operator_routes(&f, person) {
        let status = send(app(&f, Uuid::nil()), method, &uri, body.as_ref()).await?;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{method} {uri} answered a caller with no identity"
        );
    }
    Ok(())
}

/// The four correction verbs, each with a body its extractor accepts — so a
/// case below varies ONE thing and the refusal is about that thing.
fn correction_verbs(f: &Fixture, person: Uuid) -> Vec<(&'static str, Value)> {
    let account = json!({"account": account_ref(f, "acct-extractor")});
    vec![
        (
            "/v1/resolution/bind",
            bind_body(f, "acct-extractor", person),
        ),
        (
            "/v1/resolution/merge",
            json!({
                "source_person_id": person.to_string(),
                "target_person_id": Uuid::now_v7().to_string(),
            }),
        ),
        ("/v1/resolution/detach", account.clone()),
        ("/v1/resolution/exclude", account),
    ]
}

#[tokio::test]
async fn a_correction_body_that_will_not_parse_is_refused_in_the_canonical_shape() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("extractor@http-live.test").await?;
    f.bound_at("acct-extractor", person, FIXTURE_REASON, 60)
        .await?;

    for (uri, _) in correction_verbs(&f, person) {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from("{not json"))?;
        let resp = app(&f, caller).oneshot(req).await?;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert!(
            content_type.starts_with("application/problem+json"),
            "{uri} answered {content_type} — the console reads the problem \
             document to say what went wrong, and gets nothing from a plain \
             extractor rejection"
        );
        assert_eq!(
            body["context"]["field_violations"][0]["field"], "body",
            "{uri}: {body}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_correction_without_a_json_content_type_is_refused_on_the_media_type() -> TestResult {
    // Held before the extractor swap too — axum's own `Json` refuses on the
    // media type as well. Kept as the guard that the swap did not trade the
    // 415 away for a parse attempt, not as a rule this change introduced.
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("media-type@http-live.test").await?;
    f.bound_at("acct-extractor", person, FIXTURE_REASON, 60)
        .await?;

    for (uri, body) in correction_verbs(&f, person) {
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .body(Body::from(body.to_string()))?;
        let resp = app(&f, caller).oneshot(req).await?;

        assert_eq!(
            resp.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "{uri} parsed a body that declared no media type"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_comment_past_the_cap_is_refused_before_the_correction_applies() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;
    let person = f.person("comment-cap@http-live.test").await?;
    f.bound_at("acct-extractor", person, FIXTURE_REASON, 60)
        .await?;

    let oversize = "c".repeat(501);
    for (uri, mut body) in correction_verbs(&f, person) {
        body["comment"] = Value::String(oversize.clone());
        let (status, answer) = post(app(&f, caller), uri, &body).await?;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {answer}");
        assert_eq!(
            answer["context"]["field_violations"][0]["field"], "comment",
            "{uri}: {answer}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_account_search_needle_past_the_cap_is_refused() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let caller = operator(&f).await?;

    // `/v1/persons` and `/v1/visible-persons` both bound their needle at 200
    // characters; this listing handed the raw one to the evidence reader.
    let needle = "n".repeat(201);
    let (status, body) = get(
        app(&f, caller),
        &format!("/v1/resolution/accounts?q={needle}"),
    )
    .await?;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["context"]["field_violations"][0]["field"], "q",
        "{body}"
    );
    Ok(())
}
