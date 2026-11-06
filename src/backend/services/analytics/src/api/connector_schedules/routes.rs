use axum::Router;
use axum::http::StatusCode;
use toolkit::api::{
    OpenApiRegistry, OperationBuilder, ParamSpec, ResponseHeaderSpec, ResponseHeaderType,
};

use crate::domain::connector_schedules::dto::{ScheduleRequest, ScheduleResponse};

const PATH: &str = "/v1/connectors/{connector}/{source_id}/schedule";

pub(super) fn register(mut router: Router, openapi: &dyn OpenApiRegistry) -> Router {
    router = OperationBuilder::get(PATH)
        .operation_id("analytics_api.connector_schedules.get")
        .summary("Requested and recorded schedules of a connector instance (admin)")
        .authenticated()
        .no_license_required()
        .path_param("connector", "Connector name")
        .path_param("source_id", "Connector instance source id")
        .json_response_with_schema::<ScheduleResponse>(openapi, StatusCode::OK, "Schedule state; stored requests remain readable after instance removal with acknowledged observations")
        .response_header(etag())
        .standard_errors(openapi)
        .error_503(openapi)
        .handler(super::get)
        .register(router, openapi);

    router = OperationBuilder::put(PATH)
        .operation_id("analytics_api.connector_schedules.put")
        .summary("Request connector schedules (admin)")
        .authenticated()
        .no_license_required()
        .path_param("connector", "Connector name")
        .path_param("source_id", "Connector instance source id")
        .param(
            ParamSpec::header("If-Match")
                .required(true)
                .description("Single quoted decimal revision, e.g. \"0\""),
        )
        .json_request::<ScheduleRequest>(openapi, "One to ten five-field UTC cron expressions")
        .json_response_with_schema::<ScheduleResponse>(openapi, StatusCode::OK, "Schedule state")
        .response_header(etag())
        .standard_errors(openapi)
        .error_409(openapi)
        .error_503(openapi)
        .problem_response(
            openapi,
            StatusCode::PRECONDITION_FAILED,
            "Stale schedule revision; context.revision and ETag give the current revision",
        )
        .response_header(etag())
        .problem_response(
            openapi,
            StatusCode::PRECONDITION_REQUIRED,
            "If-Match required",
        )
        .problem_response(
            openapi,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Content-Type application/json required",
        )
        .handler(super::put)
        .register(router, openapi);

    router = OperationBuilder::delete(PATH)
        .operation_id("analytics_api.connector_schedules.delete")
        .summary("Clear requested connector schedules (admin)")
        .authenticated()
        .no_license_required()
        .path_param("connector", "Connector name")
        .path_param("source_id", "Connector instance source id")
        .param(
            ParamSpec::header("If-Match")
                .required(true)
                .description("Single quoted decimal revision, e.g. \"0\""),
        )
        .json_response_with_schema::<ScheduleResponse>(openapi, StatusCode::OK, "Schedule state after clearing; stored requests can also be cleared after instance removal")
        .response_header(etag())
        .standard_errors(openapi)
        .error_409(openapi)
        .error_503(openapi)
        .problem_response(
            openapi,
            StatusCode::PRECONDITION_FAILED,
            "Stale schedule revision; context.revision and ETag give the current revision",
        )
        .response_header(etag())
        .problem_response(
            openapi,
            StatusCode::PRECONDITION_REQUIRED,
            "If-Match required",
        )
        .handler(super::delete)
        .register(router, openapi);

    router
}

fn etag() -> ResponseHeaderSpec {
    ResponseHeaderSpec::new(
        "ETag",
        "Single quoted decimal schedule revision",
        ResponseHeaderType::String,
    )
}
