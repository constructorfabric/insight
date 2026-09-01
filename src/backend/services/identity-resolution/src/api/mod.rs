//! HTTP API layer — shared state, route table, extractors.

pub(crate) mod canonical_json;
pub(crate) mod datetime;
pub mod error;
mod gate;
mod handlers;
#[cfg(test)]
mod http_live_tests;
mod listing;
pub mod me;
pub mod person_roles;
pub mod persons;
pub mod resolution;
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
    /// MariaDB connection pool (SeaORM) — reads the `persons` journal.
    pub db: DatabaseConnection,
    /// Gear config (`org_chart_source_type`, `clickhouse_*`, …).
    pub config: GearConfig,
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
    let api = build_operations(Router::new(), openapi)
        .layer(Extension(state))
        .layer(insight_http_metrics::ServerMetricsLayer::new(
            "identity-resolution",
        ));

    host_router.merge(api)
}

/// Title/version/description of the emitted document. Kept in step with the
/// `openapi` block of `config/insight.yaml`, which the live gear reads: the two
/// describe the same surface and should not disagree.
fn openapi_info() -> OpenApiInfo {
    OpenApiInfo {
        title: "Insight Identity Resolution API".to_owned(),
        version: "1.0.0".to_owned(),
        description: Some(
            "Person identity for the product: profile resolution, the operator \
             correction surface over account-to-person bindings, org-chart reads, \
             roles and visibility, plus the persons-seed and persons-sync \
             operation journals. The API Gateway mounts this service at \
             /api/identity."
                .to_owned(),
        ),
        servers: Vec::new(),
    }
}

/// Build the identity-resolution `OpenAPI` document **offline** — no
/// `AppState`, DB or HTTP listener. Backs the `identity-resolution openapi`
/// subcommand (committed-doc regeneration + drift gate), reusing the exact
/// `build_operations` route table the live gear serves, so the two cannot
/// diverge.
///
/// # Errors
///
/// Returns an error if the registry cannot assemble the document.
pub fn openapi_document() -> anyhow::Result<utoipa::openapi::OpenApi> {
    let openapi = OpenApiRegistryImpl::new();
    let _ = build_operations(Router::new(), &openapi);

    openapi
        .build_openapi(&openapi_info())
        .map_err(|e| anyhow::anyhow!("failed to build identity-resolution OpenAPI document: {e}"))
}

/// Declare each operation via the toolkit `OperationBuilder` (records the route
/// + its OpenAPI spec + auth/error metadata).
#[expect(clippy::too_many_lines)] // one flat block per route — readability over splitting
fn build_operations(router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    // Internal, SERVICE-ONLY S2S resolvers — SEPARATE routes, one per question,
    // so the login bootstrap (by external id, or by roster address where the
    // IdP has no directory connector of its own) and the authenticator's admin
    // `__override` view-as feature can never be confused for one another via a
    // shared dispatch parameter. Registered as raw routes so they stay out of
    // the generated OpenAPI; auth is still enforced by the host gateway and `SecurityContext` is
    // injected by the host authn pipeline, same as every other route. Each
    // handler itself gates on `subject_type == "service"`.
    let router = router.route(
        "/internal/persons/by-external-id",
        axum::routing::get(handlers::internal_person_by_external_id),
    );
    let router = router.route(
        "/internal/persons/by-roster-email",
        axum::routing::get(handlers::internal_person_by_roster_email),
    );
    let router = router.route(
        "/internal/persons/by-email-override",
        axum::routing::get(handlers::internal_person_by_email_override),
    );
    let router = router.route(
        "/internal/persons/provision",
        axum::routing::post(handlers::internal_provision_person),
    );
    let router = router.route(
        "/internal/persons/active-roles",
        axum::routing::get(handlers::internal_person_active_roles),
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

    let router = OperationBuilder::post("/v1/profiles/batch")
        .operation_id("identity_resolution.profiles.batch")
        .summary("Resolve visible profiles by canonical person id")
        .authenticated()
        .no_license_required()
        .json_request::<profile::BatchProfilesRequest>(openapi, "Canonical people to resolve")
        .json_response_with_schema::<profile::BatchProfilesResponse>(
            openapi,
            StatusCode::OK,
            "Visible profiles",
        )
        .standard_errors(openapi)
        .handler(handlers::batch_profiles)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/me")
        .operation_id("identity_resolution.me.get")
        .summary("The caller's identity and active roles")
        .authenticated()
        .no_license_required()
        .json_response_with_schema::<me::MeResponse>(
            openapi,
            StatusCode::OK,
            "Who the gateway JWT identifies, with their active identity roles",
        )
        .standard_errors(openapi)
        .handler(me::get_me)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/persons")
        .operation_id("identity_resolution.persons.search")
        .summary("List the tenant's persons, narrowed by search terms (admin)")
        .authenticated()
        .query_param(
            "q",
            false,
            "Search terms, at most 8 (200 characters total); every \
             whitespace-separated term must match one of the person's current \
             identity values — email, username or display/first/last name \
             (case-insensitive substring), the same five the row's own label is \
             built from. Titles, statuses and the HR employee id are served on \
             the profile but not searched. A term that parses as a UUID names a \
             person id instead. Absent or blank lists every person of the \
             tenant.",
        )
        .query_param_typed(
            "limit",
            false,
            "Cap on returned persons (1..=100, default 20)",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "Opaque `next_cursor` from the previous page. Valid only for the \
             query that issued it: changing `q` restarts the listing.",
        )
        .no_license_required()
        .json_response_with_schema::<persons::PersonListResponse>(
            openapi,
            StatusCode::OK,
            "One page of persons, ordered by the name each row shows",
        )
        .standard_errors(openapi)
        .handler(persons::search_persons)
        .register(router, openapi);

    // Operator identity corrections (ADR-0003): each verb appends binding
    // observations authored by the caller. Admin-gated like the rest of the
    // operator surface; the handlers journal every call in `operations`.
    let router = OperationBuilder::post("/v1/resolution/bind")
        .operation_id("identity_resolution.resolution.bind")
        .summary("Bind accounts to persons (single or bulk; also confirms an automatic binding)")
        .authenticated()
        .no_license_required()
        .json_request::<resolution::BindRequest>(openapi, "Bindings to record")
        .json_response_with_schema::<resolution::CorrectionResponse>(
            openapi,
            StatusCode::OK,
            "Per-account outcomes",
        )
        .standard_errors(openapi)
        .handler(resolution::bind)
        .register(router, openapi);

    let router = OperationBuilder::post("/v1/resolution/merge")
        .operation_id("identity_resolution.resolution.merge")
        .summary("Merge two persons: rebind every account of the absorbed person")
        .authenticated()
        .no_license_required()
        .json_request::<resolution::MergeRequest>(openapi, "Persons to merge")
        .json_response_with_schema::<resolution::CorrectionResponse>(
            openapi,
            StatusCode::OK,
            "Per-account outcomes",
        )
        .standard_errors(openapi)
        .handler(resolution::merge)
        .register(router, openapi);

    let router = OperationBuilder::post("/v1/resolution/detach")
        .operation_id("identity_resolution.resolution.detach")
        .summary("Detach an account into a freshly minted person")
        .authenticated()
        .no_license_required()
        .json_request::<resolution::AccountRequest>(openapi, "Account to detach")
        .json_response_with_schema::<resolution::CorrectionResponse>(
            openapi,
            StatusCode::OK,
            "Outcome and the new person id",
        )
        .standard_errors(openapi)
        .handler(resolution::detach)
        .register(router, openapi);

    let router = OperationBuilder::post("/v1/resolution/exclude")
        .operation_id("identity_resolution.resolution.exclude")
        .summary("Exclude an account as not a person (bot, CI, service account)")
        .authenticated()
        .no_license_required()
        .json_request::<resolution::AccountRequest>(openapi, "Account to exclude")
        .json_response_with_schema::<resolution::CorrectionResponse>(
            openapi,
            StatusCode::OK,
            "Outcome",
        )
        .standard_errors(openapi)
        .handler(resolution::exclude)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/resolution/attention")
        .operation_id("identity_resolution.resolution.attention")
        .summary("Accounts awaiting an operator decision, with the resolution rates")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "limit",
            false,
            "Cap on returned items (1..=1000, default 100); `items_truncated` \
             says whether it cut the list. The rates cover every observed \
             account whatever the cap — unless `truncated` is set, which means \
             the evidence read itself hit its safety ceiling and both the \
             rates and the queue describe only the accounts read before it.",
            "integer",
        )
        .json_response_with_schema::<resolution::AttentionResponse>(
            openapi,
            StatusCode::OK,
            "Queue items and rates",
        )
        .standard_errors(openapi)
        .handler(resolution::attention)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/resolution/accounts")
        .operation_id("identity_resolution.resolution.search_accounts")
        .summary("List the observed accounts and whose each one is")
        .authenticated()
        .no_license_required()
        .query_param_typed(
            "q",
            false,
            "Needle matched against every value the account's row shows, \
             case-insensitively: a substring of the current address, handle, id \
             or observed name (whole, or composed from parts). The source is the \
             exception — it matches a whole `_`/`-` separated segment from its \
             start, so `github` and `entra` list those connectors' accounts \
             while `hub` lists none. At most 200 characters. Absent or blank \
             lists every open account.",
            "string",
        )
        .query_param_typed(
            "limit",
            false,
            "Cap on returned accounts (1..=100, default 20)",
            "integer",
        )
        .query_param(
            "cursor",
            false,
            "Opaque `next_cursor` from the previous page. Valid only for the \
             query that issued it: changing `q` restarts the listing.",
        )
        .json_response_with_schema::<resolution::AccountSearchResponse>(
            openapi,
            StatusCode::OK,
            "One page of accounts with their holders",
        )
        .standard_errors(openapi)
        .handler(resolution::search_accounts)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/resolution/accounts/{source}/{source_id}/{account_id}")
        .operation_id("identity_resolution.resolution.account_binding")
        .summary("Current binding of an account and every decision behind it")
        .authenticated()
        .no_license_required()
        .path_param("source", "Connector type, e.g. `github`")
        .path_param("source_id", "Connector instance id")
        .path_param("account_id", "Account id within that connector instance")
        .json_response_with_schema::<resolution::AccountBindingResponse>(
            openapi,
            StatusCode::OK,
            "Binding and history",
        )
        .standard_errors(openapi)
        .handler(resolution::account_binding)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/resolution/persons/{person_id}/accounts")
        .operation_id("identity_resolution.resolution.person_accounts")
        .summary("Every account bound to a person, with the values behind each link")
        .authenticated()
        .no_license_required()
        .path_param("person_id", "Person whose accounts to list")
        .json_response_with_schema::<resolution::PersonAccountsResponse>(
            openapi,
            StatusCode::OK,
            "Accounts of the person",
        )
        .standard_errors(openapi)
        .handler(resolution::person_accounts)
        .register(router, openapi);

    // Persons-seed operations journal (read-only; the seed itself runs via the
    // `seed` CLI subcommand — CronJob / manual Job, see `crate::seed_runner`).
    // Admin-gated: caller = gateway-JWT subject, must hold the `admin` role in
    // the tenant.
    let router = OperationBuilder::get("/v1/persons-seed/{id}")
        .operation_id("identity_resolution.persons_seed.get")
        .summary("Get a persons-seed operation")
        .authenticated()
        .path_param("id", "Operation id")
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
        .query_param("status", false, "Filter by operation status")
        .query_param_typed(
            "limit",
            false,
            "Cap on returned operations (1..=500, default 50)",
            "integer",
        )
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
        .path_param("id", "Operation id")
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
        .query_param("status", false, "Filter by operation status")
        .query_param_typed(
            "limit",
            false,
            "Cap on returned operations (1..=500, default 50)",
            "integer",
        )
        .no_license_required()
        .json_response_with_schema::<sync::PersonsSyncListResponse>(
            openapi,
            StatusCode::OK,
            "Operations",
        )
        .standard_errors(openapi)
        .handler(sync::list_persons_sync)
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
        .path_param("id", "Role id")
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
        .query_param("person", false, "Only assignments of this person")
        .query_param("role", false, "Only assignments of this role")
        .query_param_typed(
            "active",
            false,
            "Only assignments still in force",
            "boolean",
        )
        .query_param_typed(
            "limit",
            false,
            "Cap on returned assignments (1..=500, default 50)",
            "integer",
        )
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
        .path_param("id", "Role assignment id")
        .json_request::<person_roles::RevokeReasonRequest>(openapi, "Why the assignment is revoked")
        .request_optional()
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
        .query_param("viewer", false, "Only grants held by this viewer")
        .query_param("viewed", false, "Only grants over this person")
        .query_param_typed("active", false, "Only grants still in force", "boolean")
        .query_param_typed(
            "limit",
            false,
            "Cap on returned grants (1..=500, default 50)",
            "integer",
        )
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
        .path_param("id", "Visibility grant id")
        .json_request::<visibility::RevokeReasonRequest>(openapi, "Why the grant is revoked")
        .request_optional()
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
        .query_param_typed(
            "depth",
            false,
            "Max descent depth (>= 0); capped at the server's maximum, which is also the default",
            "integer",
        )
        .query_param(
            "valid_at",
            false,
            "Point-in-time lens (ISO-8601 / RFC-3339); absent reads the current state",
        )
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
        .path_param("person_id", "Person the subtree is rooted at")
        .query_param_typed(
            "depth",
            false,
            "Max descent depth (>= 0); capped at the server's maximum, which is also the default",
            "integer",
        )
        .query_param(
            "valid_at",
            false,
            "Point-in-time lens (ISO-8601 / RFC-3339); absent reads the current state",
        )
        .no_license_required()
        .json_response_with_schema::<subchart::SubchartResponse>(
            openapi,
            StatusCode::OK,
            "Subchart",
        )
        .standard_errors(openapi)
        .handler(subchart::get_subchart)
        .register(router, openapi);

    let router = OperationBuilder::get("/v1/visible-persons")
        .operation_id("identity_resolution.visible_persons.list")
        .summary("List the org members the caller may see")
        .description(
            "The organisation's roster: persons a connector claims as an account \
             holder. An address seen only in someone else's data — a commit author \
             nobody here holds — is an identity the journal carries, not a member, \
             and is left out. The POST sibling confirms visibility over any person \
             id, so it answers for a wider set than this lists.",
        )
        .authenticated()
        .query_param(
            "q",
            false,
            "Whitespace-separated terms; absent browses the roster",
        )
        .query_param_typed("limit", false, "Page size (1..=500, default 50)", "integer")
        .query_param("cursor", false, "Resume token from a previous page")
        .no_license_required()
        .json_response_with_schema::<visible_persons::VisiblePersonsPageResponse>(
            openapi,
            StatusCode::OK,
            "One page of visible persons",
        )
        // Only what this operation can answer: a rejected `q`/`cursor` or an
        // unresolved tenant (400), no identified caller (401), a failed read
        // (500). It has no body to refuse, no resource to miss and no conflict.
        .error_400(openapi)
        .error_401(openapi)
        .error_500(openapi)
        .handler(visible_persons::list_visible_persons)
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

#[cfg(test)]
mod openapi_tests {
    use utoipa::openapi::path::ParameterIn;

    use super::*;

    #[test]
    fn the_document_builds_without_state_or_backends() -> anyhow::Result<()> {
        let document = openapi_document()?;

        assert!(
            document.paths.paths.contains_key("/v1/resolution/bind"),
            "the correction surface must be described: {:?}",
            document.paths.paths.keys().collect::<Vec<_>>()
        );

        Ok(())
    }

    #[test]
    fn the_internal_s2s_routes_stay_out_of_the_document() -> anyhow::Result<()> {
        let document = openapi_document()?;

        for path in document.paths.paths.keys() {
            assert!(
                !path.starts_with("/internal/"),
                "internal S2S route leaked into the published contract: {path}"
            );
        }

        Ok(())
    }

    /// A templated path whose parameters are undeclared is not a valid OpenAPI
    /// document: a generated client has nothing to fill `{source_id}` from.
    #[test]
    fn every_templated_path_declares_its_parameters() -> anyhow::Result<()> {
        let document = openapi_document()?;

        for (path, item) in &document.paths.paths {
            let templated = path.matches('{').count();
            if templated == 0 {
                continue;
            }

            let methods = [
                ("get", &item.get),
                ("post", &item.post),
                ("put", &item.put),
                ("patch", &item.patch),
                ("delete", &item.delete),
            ];

            for (method, operation) in methods {
                let Some(operation) = operation else { continue };

                let declared = operation.parameters.as_ref().map_or(0, |params| {
                    params
                        .iter()
                        .filter(|p| p.parameter_in == ParameterIn::Path)
                        .count()
                });

                assert_eq!(
                    declared, templated,
                    "{method} {path} templates {templated} parameter(s), declares {declared}"
                );
            }
        }

        Ok(())
    }
}
