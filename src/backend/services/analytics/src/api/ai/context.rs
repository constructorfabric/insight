//! `/v1/ai/context` — the entries people write for the model to read.
//!
//! A person owns their own entries; the organisation's are an admin's to write
//! and everyone's to read, because they shape every explanation in the tenant.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait as _, ColumnTrait, Condition, ConnectionTrait, EntityTrait, Order,
    PaginatorTrait as _, QueryFilter, QueryOrder, Set, TransactionError, TransactionTrait as _,
};
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use super::super::error::AiError;
use super::super::{AppState, require_admin};
use super::{admin_only_context, ensure_enabled, read_error, write_error};
use crate::domain::ai::dto::{
    ContextEntryResponse, ContextListResponse, CreateContextRequest, Prose, Scope, Title,
    UpdateContextRequest,
};
use crate::domain::ai::prompt::Entry;
use crate::infra::db::entities::ai_context_entries as entries;
use crate::migration::ai_assist_schema;

/// `GET /v1/ai/context` — the caller's own entries and the organisation's.
pub async fn list_context(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    let rows = visible_rows(&state, ctx.subject_tenant_id(), ctx.subject_id()).await?;

    Ok(Json(ContextListResponse {
        items: rows.into_iter().map(to_response).collect(),
    }))
}

/// `POST /v1/ai/context` — add an entry to one scope.
pub async fn create_context(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Json(req): Json<CreateContextRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;
    if req.scope == Scope::Tenant {
        require_admin(&state, &headers, admin_only_context).await?;
    }

    let title = parse_title(&req.title)?;
    let body = parse_body(&req.body)?;

    let tenant = ctx.subject_tenant_id();
    let owner = owner_of(req.scope, ctx.subject_id());
    let now = Utc::now();
    let row = entries::ActiveModel {
        id: Set(Uuid::now_v7()),
        insight_tenant_id: Set(tenant),
        scope: Set(req.scope.as_str().to_owned()),
        person_id: Set(owner),
        title: Set(title),
        body: Set(body),
        created_at: Set(now),
        updated_at: Set(now),
    };

    // Insert first, then count what the scope now holds, and roll back when
    // that is over the cap.
    //
    // INVARIANT: the count must NOT lock. Checking the cap under `FOR UPDATE`
    // before inserting reads a range that holds no rows for a scope's first
    // entry, so InnoDB gap-locks to the index supremum; two tenants adding
    // their first entry at the same moment then deadlock on the insert
    // intention and one caller gets a 500. Verified against MariaDB's own
    // deadlock report.
    //
    // The cost is that two writes racing on a scope holding exactly the last
    // free slot can both commit, leaving one entry over. That is a courtesy
    // limit on how much context a prompt carries, not an invariant anything
    // depends on — and a scope of 21 is cheaper than a failed first write.
    let stored = state
        .db
        .transaction::<_, entries::Model, CanonicalError>(move |tx| {
            Box::pin(async move {
                let stored = row
                    .insert(tx)
                    .await
                    .map_err(|e| write_error(&e, "context insert"))?;
                refuse_when_scope_is_full(tx, tenant, req.scope, owner).await?;
                Ok(stored)
            })
        })
        .await
        .map_err(unwrap_transaction_error)?;

    Ok((StatusCode::CREATED, Json(to_response(stored))))
}

/// `PATCH /v1/ai/context/{id}` — edit one entry the caller may write.
pub async fn update_context(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateContextRequest>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    let existing = writable_row(&state, &ctx, &headers, id).await?;
    let now = Utc::now();

    let mut row: entries::ActiveModel = existing.into();
    if let Some(title) = req.title.as_deref() {
        row.title = Set(parse_title(title)?);
    }
    if let Some(body) = req.body.as_deref() {
        row.body = Set(parse_body(body)?);
    }
    row.updated_at = Set(now);

    let stored = row
        .update(&state.db)
        .await
        .map_err(|e| write_error(&e, "context update"))?;

    Ok(Json(to_response(stored)))
}

/// `DELETE /v1/ai/context/{id}` — remove one entry the caller may write.
pub async fn delete_context(
    Extension(state): Extension<Arc<AppState>>,
    Extension(ctx): Extension<SecurityContext>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, CanonicalError> {
    ensure_enabled(&state)?;

    let existing = writable_row(&state, &ctx, &headers, id).await?;

    entries::Entity::delete_by_id(existing.id)
        .exec(&state.db)
        .await
        .map_err(|e| write_error(&e, "context delete"))?;

    Ok(StatusCode::NO_CONTENT)
}

/// The two scopes the explain path reads, oldest first within each.
pub(crate) async fn prompt_entries(
    state: &AppState,
    tenant: Uuid,
    person: Uuid,
) -> Result<(Vec<Entry>, Vec<Entry>), CanonicalError> {
    let rows = visible_rows(state, tenant, person).await?;

    let mut tenant_entries = Vec::new();
    let mut person_entries = Vec::new();
    for row in rows {
        let entry = Entry {
            title: row.title,
            body: row.body,
        };
        if row.scope == Scope::Tenant.as_str() {
            tenant_entries.push(entry);
        } else {
            person_entries.push(entry);
        }
    }

    Ok((tenant_entries, person_entries))
}

async fn visible_rows(
    state: &AppState,
    tenant: Uuid,
    person: Uuid,
) -> Result<Vec<entries::Model>, CanonicalError> {
    entries::Entity::find()
        .filter(entries::Column::InsightTenantId.eq(tenant))
        .filter(
            Condition::any()
                .add(entries::Column::Scope.eq(Scope::Tenant.as_str()))
                .add(
                    Condition::all()
                        .add(entries::Column::Scope.eq(Scope::Person.as_str()))
                        .add(entries::Column::PersonId.eq(person)),
                ),
        )
        .order_by(entries::Column::CreatedAt, Order::Asc)
        .all(&state.db)
        .await
        .map_err(|e| read_error(&e, "context list"))
}

/// The row this caller is allowed to change, or why they may not.
async fn writable_row(
    state: &AppState,
    ctx: &SecurityContext,
    headers: &HeaderMap,
    id: Uuid,
) -> Result<entries::Model, CanonicalError> {
    let row = entries::Entity::find_by_id(id)
        .filter(entries::Column::InsightTenantId.eq(ctx.subject_tenant_id()))
        .one(&state.db)
        .await
        .map_err(|e| read_error(&e, "context read"))?
        .ok_or_else(|| {
            AiError::not_found("context entry not found")
                .with_resource(id.to_string())
                .create()
        })?;

    if row.scope == Scope::Tenant.as_str() {
        require_admin(state, headers, admin_only_context).await?;
        return Ok(row);
    }

    if row.person_id == Some(ctx.subject_id()) {
        return Ok(row);
    }

    Err(AiError::not_found("context entry not found")
        .with_resource(id.to_string())
        .create())
}

async fn refuse_when_scope_is_full(
    db: &impl ConnectionTrait,
    tenant: Uuid,
    scope: Scope,
    owner: Option<Uuid>,
) -> Result<(), CanonicalError> {
    let mut condition = Condition::all()
        .add(entries::Column::InsightTenantId.eq(tenant))
        .add(entries::Column::Scope.eq(scope.as_str()));
    if let Some(owner) = owner {
        condition = condition.add(entries::Column::PersonId.eq(owner));
    }

    let count = entries::Entity::find()
        .filter(condition)
        .count(db)
        .await
        .map_err(|e| read_error(&e, "context count"))?;

    // Counted after the insert, so the caller's own row is included.
    if count > ai_assist_schema::MAX_ENTRIES_PER_SCOPE {
        return Err(AiError::invalid_argument()
            .with_field_violation(
                "scope",
                "how many entries this scope holds",
                format!(
                    "already holds {} entries, the most one scope may have",
                    ai_assist_schema::MAX_ENTRIES_PER_SCOPE
                ),
            )
            .create());
    }

    Ok(())
}

/// A transaction reports either our own refusal or a database failure; the
/// caller only ever sees a canonical error either way.
fn unwrap_transaction_error(error: TransactionError<CanonicalError>) -> CanonicalError {
    match error {
        TransactionError::Transaction(inner) => inner,
        TransactionError::Connection(e) => write_error(&e, "context transaction"),
    }
}

fn owner_of(scope: Scope, caller: Uuid) -> Option<Uuid> {
    match scope {
        Scope::Tenant => None,
        Scope::Person => Some(caller),
    }
}

fn parse_title(raw: &str) -> Result<String, CanonicalError> {
    Title::parse(raw)
        .map(Title::into_inner)
        .map_err(|rejected| {
            AiError::invalid_argument()
                .with_field_violation("title", "the entry title", rejected.reason())
                .create()
        })
}

fn parse_body(raw: &str) -> Result<String, CanonicalError> {
    Prose::parse(raw)
        .map(Prose::into_inner)
        .map_err(|rejected| {
            AiError::invalid_argument()
                .with_field_violation("body", "the entry body", rejected.reason())
                .create()
        })
}

fn to_response(row: entries::Model) -> ContextEntryResponse {
    ContextEntryResponse {
        id: row.id.to_string(),
        scope: Scope::parse(&row.scope).unwrap_or(Scope::Person),
        title: row.title,
        body: row.body,
        updated_at: row.updated_at.to_rfc3339(),
    }
}
