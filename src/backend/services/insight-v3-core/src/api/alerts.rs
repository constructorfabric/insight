//! Alert rule endpoints: what an administrator writes, reads and turns on
//! or off, and the notifications a rule's checks have owed.

use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, Http, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::domain::alerts::rule::{AlertRule, Notification};
use crate::domain::alerts::rules::AlertsError;
use crate::domain::alerts::{Number, Outcome, RuleDraft, RuleError, UnknownReason};
use crate::domain::definition::{DefinitionName, MAX_PAGE_LIMIT, Page};

#[resource_error("gts.cf.insight.insight_v3_core.alerts.v1~")]
struct AlertApiError;

impl ApiErrors for AlertApiError {
    fn invalid_field(field: &str, detail: String) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "INVALID")
            .create()
    }

    fn timed_out(detail: &str) -> CanonicalError {
        Self::deadline_exceeded(detail).create()
    }

    fn name_taken(name: &str) -> CanonicalError {
        Self::already_exists(format!("`{name}` is already taken"))
            .with_resource(name)
            .create()
    }

    fn missing(resource: &str, detail: String) -> CanonicalError {
        Self::not_found(detail).with_resource(resource).create()
    }

    fn oversized(field: &str, detail: &str) -> CanonicalError {
        Self::invalid_argument()
            .with_field_violation(field, detail, "TOO_LARGE")
            .create()
    }
}

/// The query string on a list: what to look for, in a name or a metric, and
/// which page to answer with.
#[derive(Debug, Default, Deserialize)]
struct Search {
    #[serde(default)]
    q: String,
    limit: Option<u64>,
    offset: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct Paged {
    limit: Option<u64>,
    offset: Option<u64>,
}

/// Turning a rule on or off names the revision it expects to change.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
struct EnabledRequest {
    expected_revision: u32,
}

impl toolkit::api::api_dto::RequestApiDto for EnabledRequest {}

impl toolkit::api::api_dto::ResponseApiDto for RuleResponse {}
impl toolkit::api::api_dto::ResponseApiDto for NotificationsPage {}
impl toolkit::api::api_dto::ResponseApiDto for DestinationsResponse {}

/// One page of a rule's notifications, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct NotificationsPage {
    notifications: Vec<NotificationResponse>,
    limit: u64,
    offset: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct DestinationsResponse {
    destinations: Vec<DestinationResponse>,
}

/// A rule as a reader is shown it.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct RuleResponse {
    name: String,
    metric: String,
    column: String,
    operator: String,
    threshold: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    range: Option<String>,
    interval_secs: u32,
    destination: String,
    enabled: bool,
    revision: u32,
    state: StateResponse,
    created_at: String,
    updated_at: String,
}

/// What the latest checks left: when, what was found, and whether the
/// last valid check saw the condition met.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct StateResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    last_evaluated_at: Option<String>,
    /// `breach`, `no_breach` or `unknown`; absent before the first check.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_outcome: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_valid_breached: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    breached_since: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct NotificationResponse {
    id: String,
    rule_revision: u32,
    metric: String,
    column: String,
    operator: String,
    threshold: serde_json::Value,
    value: serde_json::Value,
    evaluated_at: String,
    destination: String,
    /// `pending` or `cancelled`.
    status: &'static str,
    created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
struct DestinationResponse {
    name: String,
    provider: String,
}

pub(crate) fn rule_response(rule: &AlertRule) -> RuleResponse {
    RuleResponse {
        name: rule.name.as_str().to_owned(),
        metric: rule.spec.metric.as_str().to_owned(),
        column: rule.spec.column.clone(),
        operator: rule.spec.condition.operator.as_str().to_owned(),
        threshold: rule.spec.condition.threshold.to_json(),
        range: rule.spec.range.clone(),
        interval_secs: rule.spec.interval_secs,
        destination: rule.spec.destination.clone(),
        enabled: rule.enabled,
        revision: rule.revision,
        state: StateResponse {
            last_evaluated_at: rule.state.last_evaluated_at.map(|at| at.to_rfc3339()),
            last_outcome: rule.state.last_outcome.as_ref().map(Outcome::as_str),
            last_reason: rule.state.last_reason().map(UnknownReason::as_str),
            last_value: rule
                .state
                .last_outcome
                .as_ref()
                .and_then(Outcome::value)
                .map(Number::to_json),
            last_valid_breached: rule.state.last_valid_breached,
            breached_since: rule.state.breached_since.map(|at| at.to_rfc3339()),
        },
        created_at: rule.created_at.to_rfc3339(),
        updated_at: rule.updated_at.to_rfc3339(),
    }
}

pub(crate) fn notification_response(notification: &Notification) -> NotificationResponse {
    NotificationResponse {
        id: notification.id.simple().to_string(),
        rule_revision: notification.rule_revision,
        metric: notification.metric.as_str().to_owned(),
        column: notification.column.clone(),
        operator: notification.condition.operator.as_str().to_owned(),
        threshold: notification.condition.threshold.to_json(),
        value: notification.value.to_json(),
        evaluated_at: notification.evaluated_at.to_rfc3339(),
        destination: notification.destination.clone(),
        status: notification.status.as_str(),
        created_at: notification.created_at.to_rfc3339(),
    }
}

fn name_param() -> ParamSpec {
    ParamSpec::path("name").description("Alert name")
}

fn query_param(name: &str, param_type: &str, description: &str) -> ParamSpec {
    ParamSpec::query(name)
        .description(description)
        .param_type(param_type)
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let registered = [
        list_route(openapi),
        put_route(openapi),
        get_route(openapi),
        delete_route(openapi),
        enable_route(openapi),
        disable_route(openapi),
        notifications_route(openapi),
        destinations_route(openapi),
    ];

    registered.into_iter().fold(router, |router, route| {
        router.merge(route.layer(Extension(Arc::clone(state))))
    })
}

fn page_params(skipped: &str) -> [ParamSpec; 2] {
    [
        query_param(
            "limit",
            "integer",
            &format!("Page size, 1 to {MAX_PAGE_LIMIT}"),
        ),
        query_param("offset", "integer", skipped),
    ]
}

fn list_route(openapi: &dyn OpenApiRegistry) -> Router {
    let [limit, offset] = page_params("Names to skip");

    OperationBuilder::get("/v1/alerts")
        .operation_id("insight_v3_core.alerts.list")
        .summary("List alert names, or the ones matching ?q=")
        .anonymous()
        .exposed()
        .param(query_param(
            "q",
            "string",
            "Text to look for in a name or in the metric it watches",
        ))
        .param(limit)
        .param(offset)
        .json_response(StatusCode::OK, "One page of names, and how many match")
        .error_400(openapi)
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_alerts)
        .register(Router::new(), openapi)
}

fn put_route(openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::put("/v1/alerts/{name}")
        .operation_id("insight_v3_core.alerts.put")
        .summary("Create an alert, or replace one at its expected revision")
        .anonymous()
        .exposed()
        .param(name_param())
        .json_request::<RuleDraft>(openapi, "The rule")
        .json_response_with_schema::<RuleResponse>(openapi, StatusCode::OK, "The rule as stored")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(put_alert)
        .register(Router::new(), openapi)
}

fn get_route(openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/v1/alerts/{name}")
        .operation_id("insight_v3_core.alerts.get")
        .summary("Read an alert and what its latest check found")
        .anonymous()
        .exposed()
        .param(name_param())
        .json_response_with_schema::<RuleResponse>(
            openapi,
            StatusCode::OK,
            "The rule and its state",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(get_alert)
        .register(Router::new(), openapi)
}

fn delete_route(openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::delete("/v1/alerts/{name}")
        .operation_id("insight_v3_core.alerts.delete")
        .summary("Remove an alert, its schedule and its notifications")
        .anonymous()
        .exposed()
        .param(name_param())
        .no_content_response(StatusCode::NO_CONTENT, "Alert removed")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(delete_alert)
        .register(Router::new(), openapi)
}

fn enable_route(openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/v1/alerts/{name}/enable")
        .operation_id("insight_v3_core.alerts.enable")
        .summary("Turn an alert's checks on")
        .anonymous()
        .exposed()
        .param(name_param())
        .json_request::<EnabledRequest>(openapi, "The revision this expects to change")
        .json_response_with_schema::<RuleResponse>(openapi, StatusCode::OK, "The rule as stored")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(enable_alert)
        .register(Router::new(), openapi)
}

fn disable_route(openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::post("/v1/alerts/{name}/disable")
        .operation_id("insight_v3_core.alerts.disable")
        .summary("Turn an alert's checks off and withdraw what it has not sent")
        .anonymous()
        .exposed()
        .param(name_param())
        .json_request::<EnabledRequest>(openapi, "The revision this expects to change")
        .json_response_with_schema::<RuleResponse>(openapi, StatusCode::OK, "The rule as stored")
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_409(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(disable_alert)
        .register(Router::new(), openapi)
}

fn notifications_route(openapi: &dyn OpenApiRegistry) -> Router {
    let [limit, offset] = page_params("Notifications to skip");

    OperationBuilder::get("/v1/alerts/{name}/notifications")
        .operation_id("insight_v3_core.alerts.notifications")
        .summary("The notifications an alert's checks have owed, newest first")
        .anonymous()
        .exposed()
        .param(name_param())
        .param(limit)
        .param(offset)
        .json_response_with_schema::<NotificationsPage>(
            openapi,
            StatusCode::OK,
            "One page of notifications",
        )
        .error_400(openapi)
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_notifications)
        .register(Router::new(), openapi)
}

fn destinations_route(openapi: &dyn OpenApiRegistry) -> Router {
    OperationBuilder::get("/v1/alert-destinations")
        .operation_id("insight_v3_core.alerts.destinations")
        .summary("The destinations an alert may send to")
        .anonymous()
        .exposed()
        .json_response_with_schema::<DestinationsResponse>(
            openapi,
            StatusCode::OK,
            "Every configured destination, by name",
        )
        .error_403(openapi)
        .error_500(openapi)
        .handler(list_destinations)
        .register(Router::new(), openapi)
}

fn denied() -> CanonicalError {
    AlertApiError::permission_denied()
        .with_reason(crate::api::ADMIN_ONLY)
        .create()
}

/// The alerts of this installation, or the refusal that it has none.
fn alerts(
    state: &AppState,
) -> Result<crate::domain::alerts::rules::AlertRules<'_>, CanonicalError> {
    state.alerts().ok_or_else(|| {
        AlertApiError::failed_precondition()
            .with_precondition_violation(
                "alerts",
                "alerts are not enabled on this installation",
                "disabled",
            )
            .create()
    })
}

pub(crate) fn alerts_error(error: AlertsError) -> CanonicalError {
    match error {
        AlertsError::Invalid(source) => rule_error(&source),
        AlertsError::MetricMissing(name) => {
            AlertApiError::invalid_field("metric", format!("metric `{name}` was not found"))
        }
        AlertsError::NotFound(name) => {
            AlertApiError::missing(&name, format!("alert `{name}` was not found"))
        }
        AlertsError::NameTaken(name) => AlertApiError::already_exists(format!(
            "alert `{name}` already exists; send expected_revision to replace it"
        ))
        .with_resource(&name)
        .create(),
        AlertsError::Conflict {
            name,
            current,
            expected,
        } => AlertApiError::failed_precondition()
            .with_precondition_violation(
                "expected_revision",
                format!("alert `{name}` is at revision {current}, not {expected}"),
                "revision",
            )
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        AlertsError::TooMany(limit) => AlertApiError::failed_precondition()
            .with_precondition_violation(
                "name",
                format!("this installation allows at most {limit} alerts"),
                "limit",
            )
            .with_override(Http::status_code(StatusCode::CONFLICT.as_u16()))
            .create(),
        AlertsError::Store(source) => {
            tracing::error!(error = ?source, "alert store operation failed");
            CanonicalError::internal("alert store operation failed").create()
        }
        AlertsError::Definitions(source) => AlertApiError::definition_store_error(source),
        AlertsError::Schedule(source) => {
            tracing::error!(error = ?source, "the alert schedule could not be written");
            CanonicalError::internal("the alert schedule could not be written").create()
        }
    }
}

fn rule_error(error: &RuleError) -> CanonicalError {
    let field = match error {
        RuleError::Metric => "metric",
        RuleError::Column => "column",
        RuleError::Threshold => "threshold",
        RuleError::Range(_) => "range",
        RuleError::Interval { .. } => "interval_secs",
        RuleError::Destination(_) => "destination",
    };

    AlertApiError::invalid_field(field, error.to_string())
}

fn page_error(error: crate::domain::definition::PageError) -> CanonicalError {
    AlertApiError::invalid_field("limit", error.to_string())
}

async fn list_alerts(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(search): Query<Search>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let alerts = alerts(&state)?;

    let page = Page::parse(search.limit, search.offset).map_err(page_error)?;
    let found = alerts.page(&search.q, page).await.map_err(alerts_error)?;

    Ok(Json(serde_json::json!({
        "names": found.names,
        "total": found.total,
        "limit": page.limit(),
        "offset": page.offset(),
    }))
    .into_response())
}

async fn put_alert(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    let actor = crate::api::require_admin(&state, &headers, denied).await?;
    let alerts = alerts(&state)?;
    let Json(body) = body.map_err(|error| AlertApiError::unreadable_body(&error))?;

    let name = DefinitionName::parse(&name).map_err(AlertApiError::definition_error)?;
    let draft: RuleDraft = serde_json::from_value(body)
        .map_err(|error| AlertApiError::invalid_field("body", error.to_string()))?;

    let rule = alerts
        .put(&name, &draft, Some(actor))
        .await
        .map_err(alerts_error)?;

    Ok(Json(rule_response(&rule)).into_response())
}

async fn get_alert(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let alerts = alerts(&state)?;

    let name = DefinitionName::parse(&name).map_err(AlertApiError::definition_error)?;

    match alerts.get(&name).await {
        Ok(rule) => Ok(Json(rule_response(&rule)).into_response()),
        Err(AlertsError::NotFound(_)) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => Err(alerts_error(other)),
    }
}

async fn delete_alert(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let alerts = alerts(&state)?;

    let name = DefinitionName::parse(&name).map_err(AlertApiError::definition_error)?;

    match alerts.delete(&name).await {
        Ok(()) => Ok(StatusCode::NO_CONTENT.into_response()),
        Err(AlertsError::NotFound(_)) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => Err(alerts_error(other)),
    }
}

async fn enable_alert(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<EnabledRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    set_enabled(&state, &name, &headers, body, true).await
}

async fn disable_alert(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    body: Result<Json<EnabledRequest>, JsonRejection>,
) -> Result<Response, CanonicalError> {
    set_enabled(&state, &name, &headers, body, false).await
}

async fn set_enabled(
    state: &AppState,
    name: &str,
    headers: &axum::http::HeaderMap,
    body: Result<Json<EnabledRequest>, JsonRejection>,
    enabled: bool,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(state, headers, denied).await?;
    let alerts = alerts(state)?;
    let Json(request) = body.map_err(|error| AlertApiError::unreadable_body(&error))?;

    let name = DefinitionName::parse(name).map_err(AlertApiError::definition_error)?;

    let rule = alerts
        .set_enabled(&name, request.expected_revision, enabled)
        .await
        .map_err(alerts_error)?;

    Ok(Json(rule_response(&rule)).into_response())
}

async fn list_notifications(
    Extension(state): Extension<Arc<AppState>>,
    Path(name): Path<String>,
    headers: axum::http::HeaderMap,
    Query(paged): Query<Paged>,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let alerts = alerts(&state)?;

    let name = DefinitionName::parse(&name).map_err(AlertApiError::definition_error)?;
    let page = Page::parse(paged.limit, paged.offset).map_err(page_error)?;

    let listed = match alerts.notifications(&name, page).await {
        Ok(listed) => listed,
        Err(AlertsError::NotFound(_)) => return Ok(StatusCode::NOT_FOUND.into_response()),
        Err(other) => return Err(alerts_error(other)),
    };
    let notifications: Vec<NotificationResponse> =
        listed.iter().map(notification_response).collect();

    Ok(Json(NotificationsPage {
        notifications,
        limit: page.limit(),
        offset: page.offset(),
    })
    .into_response())
}

async fn list_destinations(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, denied).await?;
    let alerts = alerts(&state)?;

    let destinations: Vec<DestinationResponse> = alerts
        .destinations()
        .listed()
        .into_iter()
        .map(|(name, provider)| DestinationResponse {
            name: name.to_owned(),
            provider: provider.to_owned(),
        })
        .collect();

    Ok(Json(DestinationsResponse { destinations }).into_response())
}

#[cfg(test)]
mod tests;
