//! HTTP API layer — shared state, route table, extractors.

pub(crate) mod canonical_json;
pub(crate) mod datetime;
pub mod error;
mod gate;
mod handlers;
pub mod person_roles;
pub mod roles;
pub mod seed;
pub mod subchart;
pub mod visibility;

use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::http::StatusCode;
use sea_orm::DatabaseConnection;
use tokio::sync::mpsc;
use toolkit::api::{OpenApiRegistry, OperationBuilder};

use crate::config::GearConfig;
use crate::domain::profile;

/// Shared application state, injected into handlers via `Extension`.
#[derive(Clone)]
pub struct AppState {
    /// MariaDB connection pool (SeaORM) — reads `persons` / `account_person_map`.
    pub db: DatabaseConnection,
    /// Gear config (`org_chart_source_type`, `clickhouse_*`, …).
    pub config: GearConfig,
    /// Sender to the persons-seed worker's job queue (POST enqueues here).
    pub seed_tx: mpsc::Sender<seed::PersonsSeedJob>,
}

/// Mount the identity-resolution routes onto the host's router.
///
/// Builds our endpoints on a fresh sub-router (so the `AppState` extension
/// scopes to our routes, not the host's `/health`/`/docs`), then merges it into
/// the host router. Gateway-JWT identity is enforced entirely by the host authn
/// pipeline: the `oidc-authn-plugin` verifies the ES256 gateway JWT and maps its
/// claims — including the single signed `tenant_id` -> `subject_tenant_id` — into
/// the request `SecurityContext` (`NGINX_BFF` R1). No bespoke tenant layer.
pub fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
) -> Router {
    let api = build_operations(Router::new(), openapi).layer(Extension(state));

    host_router.merge(api)
}

/// Declare each operation via the toolkit `OperationBuilder` (records the route
/// + its OpenAPI spec + auth/error metadata).
#[allow(clippy::too_many_lines)] // one flat block per route — readability over splitting
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    // Internal, SERVICE-ONLY S2S lookup for the login bootstrap. Registered as a
    // raw route so it stays out of the generated OpenAPI (matching the .NET
    // `.ExcludeFromDescription()`); auth is still enforced by the host gateway
    // and `SecurityContext` is injected by the host authn pipeline, same as
    // every other route. The handler itself gates on `subject_type == "service"`.
    let router = router.route(
        "/internal/persons/by-email/{email}",
        axum::routing::get(handlers::internal_person_by_email),
    );

    let router = OperationBuilder::post("/v1/profiles")
        .operation_id("identity_resolution.profiles.resolve")
        .summary("Resolve a profile by email or source-native id")
        .authenticated()
        .no_license_required()
        .json_request::<profile::ResolveProfileRequest>(openapi, "Identity to resolve")
        .json_response_with_schema::<profile::ProfileResponse>(
            openapi,
            StatusCode::OK,
            "Resolved person",
        )
        .standard_errors(openapi)
        .handler(handlers::resolve_profile)
        .register(router, openapi);

    // Persons-seed (async job): enqueue + poll. Admin-gated: caller = gateway-JWT
    // subject, must hold the `admin` role in the tenant.
    let router = OperationBuilder::post("/v1/persons-seed")
        .operation_id("identity_resolution.persons_seed.create")
        .summary("Enqueue a persons-seed run (async)")
        .authenticated()
        .no_license_required()
        .json_request::<seed::PersonsSeedRequest>(openapi, "Seed options")
        .json_response_with_schema::<seed::PersonsSeedOperationResponse>(
            openapi,
            StatusCode::ACCEPTED,
            "Queued operation",
        )
        .standard_errors(openapi)
        .handler(seed::create_persons_seed)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/persons-seed/{id}")
        .operation_id("identity_resolution.persons_seed.get")
        .summary("Get a persons-seed operation")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<seed::PersonsSeedOperationResponse>(
            openapi,
            StatusCode::OK,
            "Operation status",
        )
        .standard_errors(openapi)
        .handler(seed::get_persons_seed)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/persons-seed")
        .operation_id("identity_resolution.persons_seed.list")
        .summary("List persons-seed operations")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<seed::PersonsSeedListResponse>(
            openapi,
            StatusCode::OK,
            "Operations",
        )
        .standard_errors(openapi)
        .handler(seed::list_persons_seed)
        .register(router, openapi);

    // Roles catalogue (admin-gated CRUD over the global `roles` table).
    let router = OperationBuilder::post("/v1/roles")
        .operation_id("identity_resolution.roles.create")
        .summary("Create a role (admin)")
        .authenticated()
        .no_license_required()
        .json_request::<roles::CreateRoleRequest>(openapi, "Role to create")
        .json_response_with_schema::<roles::RoleResponse>(
            openapi,
            StatusCode::CREATED,
            "Created role",
        )
        .standard_errors(openapi)
        .handler(roles::create_role)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/roles")
        .operation_id("identity_resolution.roles.list")
        .summary("List roles (admin)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<roles::RoleListResponse>(openapi, StatusCode::OK, "Roles")
        .standard_errors(openapi)
        .handler(roles::list_roles)
        .register(router, openapi);

    let router = OperationBuilder::delete("/v1/roles/{id}")
        .operation_id("identity_resolution.roles.delete")
        .summary("Delete a role (admin)")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Role deleted")
        .standard_errors(openapi)
        .handler(roles::delete_role)
        .register(router, openapi);

    // Person-roles junction (admin-gated grant / list / revoke assignments).
    let router = OperationBuilder::post("/v1/person-roles")
        .operation_id("identity_resolution.person_roles.create")
        .summary("Grant a role to a person (admin)")
        .authenticated()
        .no_license_required()
        .json_request::<person_roles::CreatePersonRoleRequest>(openapi, "Assignment to create")
        .json_response_with_schema::<person_roles::PersonRoleResponse>(
            openapi,
            StatusCode::CREATED,
            "Created assignment",
        )
        .standard_errors(openapi)
        .handler(person_roles::create_person_role)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/person-roles")
        .operation_id("identity_resolution.person_roles.list")
        .summary("List role assignments (admin)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<person_roles::PersonRoleListResponse>(
            openapi,
            StatusCode::OK,
            "Assignments",
        )
        .standard_errors(openapi)
        .handler(person_roles::list_person_roles)
        .register(router, openapi);

    let router = OperationBuilder::delete("/v1/person-roles/{id}")
        .operation_id("identity_resolution.person_roles.delete")
        .summary("Revoke a role assignment (admin)")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Assignment revoked")
        .standard_errors(openapi)
        .handler(person_roles::delete_person_role)
        .register(router, openapi);

    // Visibility grants (admin-gated create / list / revoke).
    let router = OperationBuilder::post("/v1/visibility")
        .operation_id("identity_resolution.visibility.create")
        .summary("Create a visibility grant (admin)")
        .authenticated()
        .no_license_required()
        .json_request::<visibility::CreateVisibilityRequest>(openapi, "Grant to create")
        .json_response_with_schema::<visibility::VisibilityResponse>(
            openapi,
            StatusCode::CREATED,
            "Created grant",
        )
        .standard_errors(openapi)
        .handler(visibility::create_visibility)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/visibility")
        .operation_id("identity_resolution.visibility.list")
        .summary("List visibility grants (admin)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<visibility::VisibilityListResponse>(
            openapi,
            StatusCode::OK,
            "Grants",
        )
        .standard_errors(openapi)
        .handler(visibility::list_visibility)
        .register(router, openapi);

    let router = OperationBuilder::delete("/v1/visibility/{id}")
        .operation_id("identity_resolution.visibility.delete")
        .summary("Revoke a visibility grant (admin)")
        .authenticated()
        .no_license_required()
        .no_content_response(StatusCode::NO_CONTENT, "Grant revoked")
        .standard_errors(openapi)
        .handler(visibility::delete_visibility)
        .register(router, openapi);

    // Org subchart (authenticated; visibility shapes what each caller sees).
    let router = OperationBuilder::get("/v1/subchart")
        .operation_id("identity_resolution.subchart.forest")
        .summary("Org forest the caller can see")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<subchart::SubchartForestResponse>(
            openapi,
            StatusCode::OK,
            "Visible forest",
        )
        .standard_errors(openapi)
        .handler(subchart::get_forest)
        .register(router, openapi);

    OperationBuilder::get("/v1/subchart/{person_id}")
        .operation_id("identity_resolution.subchart.get")
        .summary("Depth-bounded org subtree rooted at a person")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<subchart::SubchartResponse>(
            openapi,
            StatusCode::OK,
            "Subchart",
        )
        .standard_errors(openapi)
        .handler(subchart::get_subchart)
        .register(router, openapi)
}
