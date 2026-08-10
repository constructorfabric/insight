//! HTTP API layer — shared state, route table, extractors.

pub mod attribute_reconcile;
pub(crate) mod canonical_json;
pub(crate) mod datetime;
pub mod error;
mod gate;
mod handlers;
pub mod person_attributes;
pub mod person_roles;
pub mod policy_publish;
pub mod roles;
pub mod seed;
pub mod subchart;
pub mod sync;
pub mod visibility;
pub mod visible_persons;

use std::sync::Arc;

use axum::Extension;
use axum::Router;
use axum::http::StatusCode;
use sea_orm::DatabaseConnection;
use toolkit::api::{OpenApiInfo, OpenApiRegistry, OpenApiRegistryImpl, OperationBuilder};

use crate::config::GearConfig;
use crate::domain::profile;

/// Shared application state, injected into handlers via `Extension`.
#[derive(Clone)]
pub struct AppState {
    /// MariaDB connection pool (SeaORM) — reads `persons` / `account_person_map`.
    pub db: DatabaseConnection,
    /// Gear config (`org_chart_source_type`, `clickhouse_*`, …).
    pub config: GearConfig,
    /// Host shutdown signal, so request-spawned runs can end their journal row
    /// instead of vanishing mid-write.
    pub cancel: tokio_util::sync::CancellationToken,
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
    // Internal, SERVICE-ONLY S2S resolvers — TWO SEPARATE routes so the
    // login-bootstrap (external id) and the authenticator's admin `__override`
    // view-as feature (email) can never be confused for one another via a
    // shared dispatch parameter. Registered as raw routes so they stay out of
    // the generated OpenAPI (matching the .NET `.ExcludeFromDescription()`);
    // auth is still enforced by the host gateway and `SecurityContext` is
    // injected by the host authn pipeline, same as every other route. Each
    // handler itself gates on `subject_type == "service"`.
    let router = router.route(
        "/internal/persons/by-external-id",
        axum::routing::get(handlers::internal_person_by_external_id),
    );
    let router = router.route(
        "/internal/persons/by-email-override",
        axum::routing::get(handlers::internal_person_by_email_override),
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

    // Persons-seed operations journal (read-only; the seed itself runs via the
    // `seed` CLI subcommand — CronJob / manual Job, see `crate::seed_runner`).
    // Admin-gated: caller = gateway-JWT subject, must hold the `admin` role in
    // the tenant.
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

    // Persons-sync journal (read-only, admin-gated): the sync itself is
    // CLI-only (`identity-resolution sync` — see crate::sync_runner), same
    // trigger model as the seed after #1690.
    let router = OperationBuilder::get("/v1/persons-sync/{id}")
        .operation_id("identity_resolution.persons_sync.get")
        .summary("Get a persons-sync operation")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<sync::PersonsSyncOperationResponse>(
            openapi,
            StatusCode::OK,
            "Operation status",
        )
        .standard_errors(openapi)
        .handler(sync::get_persons_sync)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/persons-sync")
        .operation_id("identity_resolution.persons_sync.list")
        .summary("List persons-sync operations")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<sync::PersonsSyncListResponse>(
            openapi,
            StatusCode::OK,
            "Operations",
        )
        .standard_errors(openapi)
        .handler(sync::list_persons_sync)
        .register(router, openapi);

    // Person-attribute registry (admin-gated; discovery is CLI-only, see
    // `crate::attribute_reconcile_runner`).
    let router = OperationBuilder::get("/v1/person-attributes")
        .operation_id("identity_resolution.person_attributes.list")
        .summary("List discovered person attributes with their current policy (admin)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<person_attributes::PersonAttributeListResponse>(
            openapi,
            StatusCode::OK,
            "Person attributes",
        )
        .standard_errors(openapi)
        .handler(person_attributes::list_person_attributes)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/person-attributes/{id}")
        .operation_id("identity_resolution.person_attributes.get")
        .summary("One person attribute with its current policy (admin)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<person_attributes::PersonAttributeResponse>(
            openapi,
            StatusCode::OK,
            "Person attribute",
        )
        .standard_errors(openapi)
        .handler(person_attributes::get_person_attribute)
        .register(router, openapi);

    let router = OperationBuilder::put("/v1/person-attributes/{id}/policy")
        .operation_id("identity_resolution.person_attributes.put_policy")
        .summary("Append the next policy revision of a person attribute (admin)")
        .authenticated()
        .no_license_required()
        .json_request::<person_attributes::PolicyUpdateRequest>(openapi, "Next policy revision")
        .json_response_with_schema::<person_attributes::PersonAttributeResponse>(
            openapi,
            StatusCode::OK,
            "Person attribute with the appended policy",
        )
        .standard_errors(openapi)
        .handler(person_attributes::put_person_attribute_policy)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/person-attributes-reconcile/{id}")
        .operation_id("identity_resolution.person_attributes_reconcile.get")
        .summary("Poll one attribute-reconcile operation")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<attribute_reconcile::AttributeReconcileOperationResponse>(
            openapi,
            StatusCode::OK,
            "Operation status",
        )
        .standard_errors(openapi)
        .handler(attribute_reconcile::get_attribute_reconcile)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/person-attributes-reconcile")
        .operation_id("identity_resolution.person_attributes_reconcile.list")
        .summary("List attribute-reconcile operations")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<attribute_reconcile::AttributeReconcileListResponse>(
            openapi,
            StatusCode::OK,
            "Operations",
        )
        .standard_errors(openapi)
        .handler(attribute_reconcile::list_attribute_reconcile)
        .register(router, openapi);

    let router = OperationBuilder::post("/v1/person-attributes-policy-publish")
        .operation_id("identity_resolution.person_attributes_policy_publish.create")
        .summary("Trigger a policy-snapshot publish (admin)")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<policy_publish::PolicyPublishOperationResponse>(
            openapi,
            StatusCode::ACCEPTED,
            "Publish accepted; poll the operation",
        )
        .standard_errors(openapi)
        .handler(policy_publish::create_policy_publish)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/person-attributes-policy-publish/{id}")
        .operation_id("identity_resolution.person_attributes_policy_publish.get")
        .summary("Poll one policy-publish operation")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<policy_publish::PolicyPublishOperationResponse>(
            openapi,
            StatusCode::OK,
            "Operation status",
        )
        .standard_errors(openapi)
        .handler(policy_publish::get_policy_publish)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/person-attributes-policy-publish")
        .operation_id("identity_resolution.person_attributes_policy_publish.list")
        .summary("List policy-publish operations")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<policy_publish::PolicyPublishListResponse>(
            openapi,
            StatusCode::OK,
            "Operations",
        )
        .standard_errors(openapi)
        .handler(policy_publish::list_policy_publish)
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

    let router = OperationBuilder::get("/v1/subchart/{person_id}")
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
        .register(router, openapi);

    OperationBuilder::post("/v1/visible-persons")
        .operation_id("identity_resolution.visible_persons.create")
        .summary("Filter person ids to the ones the caller may see")
        .authenticated()
        .no_license_required()
        .json_request::<visible_persons::VisiblePersonsRequest>(openapi, "Person ids to check")
        .json_response_with_schema::<visible_persons::VisiblePersonsResponse>(
            openapi,
            StatusCode::OK,
            "Visible subset",
        )
        .standard_errors(openapi)
        .handler(visible_persons::filter_visible_persons)
        .register(router, openapi)
}

fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Identity Resolution API".to_owned(),
        version: "1.0.0".to_owned(),
        description: Some(
            "Identity governance service: person profiles, roles and \
             visibility administration, persons-seed/sync journals, and the \
             person-attribute registry. The API Gateway mounts this service \
             at /api/identity-resolution."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

/// Build the identity-resolution `OpenAPI` document **offline** — no
/// `AppState`, DB, or HTTP listener. Backs the `openapi` subcommand
/// (committed-doc regeneration + drift gate), reusing the exact
/// `build_operations` route table the live gear serves, so the two can never
/// diverge.
///
/// # Errors
///
/// Returns an error when the registry cannot assemble the document.
pub fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let openapi = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &openapi);
    openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build identity-resolution OpenAPI document: {e}"))
}

#[cfg(test)]
mod openapi_tests {
    /// The `openapi` subcommand's input: the document must build offline.
    #[test]
    fn openapi_document_builds_offline() {
        let doc = super::openapi_document();
        assert!(doc.is_ok(), "document must build: {:?}", doc.err());
    }
}
