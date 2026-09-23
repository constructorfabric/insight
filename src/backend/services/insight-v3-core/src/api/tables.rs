//! The warehouse tables a metric may read: the catalogue the editor offers,
//! and the columns of one table.

use std::sync::Arc;

use axum::extract::{Extension, Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use toolkit::api::{OpenApiRegistry, OperationBuilder, ParamLocation, ParamSpec};
use toolkit_canonical_errors::{CanonicalError, resource_error};
use utoipa::ToSchema;

use super::AppState;
use super::errors::ApiErrors;
use crate::store::catalog::{CatalogError, TableSchema};

#[cfg(test)]
mod tests;

#[resource_error("gts.cf.insight.insight_v3_core.tables.v1~")]
struct TableApiError;

impl ApiErrors for TableApiError {
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
}

/// One table as the catalogue lists it: enough to name it in a metric.
#[derive(Debug, Serialize, ToSchema)]
struct TableEntry {
    database: String,
    table: String,
    /// bronze, silver, gold, identity or other, read off the database.
    layer: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
struct TableList {
    tables: Vec<TableEntry>,
    total: usize,
}

#[derive(Debug, Serialize, ToSchema)]
struct ColumnEntry {
    name: String,
    r#type: String,
}

/// One table with what a metric author needs to read it.
#[derive(Debug, Serialize, ToSchema)]
struct TableDetail {
    database: String,
    table: String,
    layer: &'static str,
    /// As the warehouse spells it. A replacing table is read through `FINAL`
    /// without the metric saying so.
    engine: String,
    columns: Vec<ColumnEntry>,
}

#[derive(Debug, Deserialize)]
struct Filter {
    #[serde(default)]
    database: String,
}

pub(crate) fn register_routes(
    router: Router,
    openapi: &dyn OpenApiRegistry,
    state: &Arc<AppState>,
) -> Router {
    let list = OperationBuilder::get("/v1/tables")
        .operation_id("insight_v3_core.tables.list")
        .summary("List the warehouse tables a metric may read")
        .anonymous()
        .exposed()
        .param(ParamSpec {
            name: "database".to_owned(),
            location: ParamLocation::Query,
            required: false,
            description: Some("Only this database's tables".to_owned()),
            param_type: "string".to_owned(),
            array: false,
        })
        .json_response(StatusCode::OK, "Every table, by database and name")
        .error_403(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(list_tables)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    let get = OperationBuilder::get("/v1/tables/{database}/{table}")
        .operation_id("insight_v3_core.tables.get")
        .summary("Read one warehouse table's columns and engine")
        .anonymous()
        .exposed()
        .param(path_param("database", "Database name"))
        .param(path_param("table", "Table name"))
        .json_response(StatusCode::OK, "The table, its engine and its columns")
        .error_403(openapi)
        .error_404(openapi)
        .error_500(openapi)
        .error_504(openapi)
        .handler(get_table)
        .register(Router::new(), openapi)
        .layer(Extension(Arc::clone(state)));

    router.merge(list).merge(get)
}

fn path_param(name: &str, description: &str) -> ParamSpec {
    ParamSpec {
        name: name.to_owned(),
        location: ParamLocation::Path,
        required: true,
        description: Some(description.to_owned()),
        param_type: "string".to_owned(),
        array: false,
    }
}

async fn list_tables(
    Extension(state): Extension<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Query(filter): Query<Filter>,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;

    let wanted = filter.database.trim();
    let tables: Vec<TableEntry> = state
        .catalog()
        .tables()
        .await
        .map_err(catalog_error)?
        .into_iter()
        .filter(|schema| wanted.is_empty() || schema.database == wanted)
        .map(entry)
        .collect();

    Ok(Json(TableList {
        total: tables.len(),
        tables,
    })
    .into_response())
}

async fn get_table(
    Extension(state): Extension<Arc<AppState>>,
    Path((database, table)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<Response, CanonicalError> {
    admin_only(&state, &headers).await?;

    let found = state
        .catalog()
        .find(&database, &table)
        .await
        .map_err(catalog_error)?;
    let Some(schema) = found else {
        return Err(
            TableApiError::not_found(format!("table `{database}.{table}` was not found"))
                .with_resource(format!("{database}.{table}"))
                .create(),
        );
    };

    Ok(Json(detail(schema)).into_response())
}

fn entry(schema: TableSchema) -> TableEntry {
    TableEntry {
        database: schema.database,
        table: schema.table,
        layer: schema.layer.name(),
    }
}

fn detail(schema: TableSchema) -> TableDetail {
    TableDetail {
        database: schema.database,
        table: schema.table,
        layer: schema.layer.name(),
        engine: schema.engine,
        columns: schema
            .columns
            .into_iter()
            .map(|column| ColumnEntry {
                name: column.name,
                r#type: column.kind,
            })
            .collect(),
    }
}

/// A catalogue that did not answer. Only the wait is the caller's to see;
/// what the warehouse said is logged, not answered.
fn catalog_error(error: CatalogError) -> CanonicalError {
    match error {
        CatalogError::Timeout => TableApiError::timed_out("the warehouse catalogue timed out"),
        CatalogError::ClickHouse(source) => {
            tracing::error!(error = %source, "the warehouse catalogue could not be read");
            CanonicalError::internal("the warehouse catalogue could not be read").create()
        }
    }
}

async fn admin_only(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<uuid::Uuid, CanonicalError> {
    crate::api::require_admin(state, headers, || {
        TableApiError::permission_denied()
            .with_reason(crate::api::ADMIN_ONLY)
            .create()
    })
    .await
}
