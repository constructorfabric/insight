use std::sync::Arc;

use axum::Router;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};

use super::AppState;
use super::admission::{self, IngestAdmission};
use crate::tables::{TableError, TableName, TableStoreError};

#[resource_error("gts.cf.insight.insight_v3_core.tables.v1~")]
struct TableApiError;

pub(crate) fn register_routes(
    host_router: Router,
    openapi: &dyn OpenApiRegistry,
    state: Arc<AppState>,
    admission: IngestAdmission,
) -> Router {
    let api = OperationBuilder::put("/v1/tables/{table}")
        .operation_id("insight_v3_core.tables.create")
        .summary("Create a raw-data table")
        .anonymous()
        .exposed()
        .param(ParamSpec {
            name: "table".to_owned(),
            location: ParamLocation::Path,
            required: true,
            description: Some("Physical ClickHouse table name".to_owned()),
            param_type: "string".to_owned(),
            array: false,
        })
        .param(admission::instance_token_parameter())
        .no_content_response(StatusCode::NO_CONTENT, "Raw-data table exists")
        .error_400(openapi)
        .error_401(openapi)
        .error_429(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(create_table)
        .register(Router::new(), openapi)
        .layer(Extension(state));

    host_router.merge(admission::protect(api, admission))
}

async fn create_table(
    Path(table): Path<String>,
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    crate::api::require_admin(&state, &headers, || {
        TableApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await?;

    let table = TableName::parse(&table).map_err(table_error)?;

    state
        .tables()
        .create(&table)
        .await
        .map_err(table_store_error)?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn table_error(error: TableError) -> CanonicalError {
    TableApiError::invalid_argument()
        .with_field_violation("table", error.to_string(), "INVALID")
        .create()
}

fn table_store_error(error: TableStoreError) -> CanonicalError {
    match error {
        TableStoreError::Timeout => {
            TableApiError::deadline_exceeded("table creation timed out").create()
        }
        TableStoreError::ClickHouse(source) => {
            tracing::error!(error = ?source, "raw-data table creation failed");
            CanonicalError::internal("table creation failed").create()
        }
    }
}

#[cfg(test)]
mod tests;
