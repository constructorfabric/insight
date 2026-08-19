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
use crate::config::{GearConfig, VisibilityPolicy};
use crate::domain::resolution::EXCLUDED_PERSON;
use crate::infra::db::test_fixture::{FIXTURE_REASON, Fixture, fixture_or_skip};
use crate::infra::db::{person_roles_repo, roles_repo};

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
    let caller = f.person("roster-caller@http-live.test").await?;
    let other = f.person("roster-other@http-live.test").await?;

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
async fn a_roster_cursor_resumes_after_the_page_it_was_issued_for() -> TestResult {
    let Some(f) = fixture_or_skip().await? else {
        return Ok(());
    };
    let first_person = f.person("roster-aaa@http-live.test").await?;
    let second_person = f.person("roster-bbb@http-live.test").await?;

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
    let caller = f.person("roster-cursor@http-live.test").await?;
    let _second = f.person("roster-cursor-2@http-live.test").await?;

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
