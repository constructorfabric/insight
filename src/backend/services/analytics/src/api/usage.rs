use serde::Deserialize;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;

use super::ADMIN_ONLY;
use super::error::UsageError;
use crate::domain::date_window::{self, Window, WindowError};

mod breakdowns;
mod ingest;
mod sort;
mod summary;

pub use breakdowns::{
    UsageActionsResponse, UsagePagesResponse, UsagePeopleResponse, get_usage_actions,
    get_usage_pages, get_usage_people,
};
pub use ingest::{UsageIngestRequest, ingest_usage_events};
pub use summary::{UsageConfigResponse, UsageSummaryResponse, get_usage_config, get_usage_summary};

/// DDL owned by `scripts/migrations/20260816000000_usage-events.sql`; the
/// service holds INSERT and SELECT here, never CREATE.
const TABLE: &str = "product_usage.usage_events";

const PAGE_VIEW: &str = "page_view";

const SESSION_START: &str = "session_start";

const NIL_UUID: &str = "00000000-0000-0000-0000-000000000000";

/// Every read is bounded by the tenant and a pair of whole UTC days.
const WINDOW: &str =
    "tenant_id = toUUID(?) AND toDate(ts) >= toDate(?) AND toDate(ts) <= toDate(?)";

/// A record carrying no session id stores `''`, and every such row across every
/// person and day is that same value — one phantom visit if counted naively.
const VISITS: &str = "uniqExactIf(session_id, session_id != '')";

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UsageRangeQuery {
    /// Inclusive `YYYY-MM-DD` lower bound. Defaults to 30 days back.
    pub since: Option<String>,
    /// Inclusive `YYYY-MM-DD` upper bound. Defaults to today.
    pub until: Option<String>,
}

impl UsageRangeQuery {
    fn window(&self) -> Result<Window, CanonicalError> {
        parse_range(self.since.as_deref(), self.until.as_deref())
    }
}

fn parse_range(since: Option<&str>, until: Option<&str>) -> Result<Window, CanonicalError> {
    date_window::parse_window(since, until).map_err(range_violation)
}

fn range_violation(error: WindowError) -> CanonicalError {
    UsageError::invalid_argument()
        .with_field_violation(error.field(), error.description(), "INVALID")
        .create()
}

fn visitors() -> String {
    format!("uniqExactIf(person_id, person_id != toUUID('{NIL_UUID}'))")
}

struct WindowBinds {
    tenant: String,
    since: String,
    until: String,
}

impl WindowBinds {
    fn new(ctx: &SecurityContext, window: &Window) -> Self {
        Self {
            tenant: ctx.subject_tenant_id().to_string(),
            since: window.since.to_string(),
            until: window.until.to_string(),
        }
    }

    fn query(&self, ch: &insight_clickhouse::Client, sql: &str) -> clickhouse::query::Query {
        ch.query(sql)
            .bind(self.tenant.as_str())
            .bind(self.since.as_str())
            .bind(self.until.as_str())
    }
}

fn admin_only() -> CanonicalError {
    UsageError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "usage summary query failed");
    CanonicalError::internal("failed to read usage").create()
}

#[cfg(test)]
mod tests;
