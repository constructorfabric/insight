use std::sync::{Arc, LazyLock};

use axum::Json;
use axum::extract::{Extension, Query};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use toolkit_canonical_errors::CanonicalError;

use super::error::IngestionError;
use super::{ADMIN_ONLY, AppState, require_admin};

const VIEW: &str = "insight.bronze_insert_events";

const MAX_POINTS: usize = 50_000;

const MAX_CONCURRENT_READS: usize = 4;
const ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
static READ_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(MAX_CONCURRENT_READS));

pub(crate) fn aggregation_settings() -> [(&'static str, String); 2] {
    [
        ("max_rows_to_group_by", MAX_POINTS.to_string()),
        ("group_by_overflow_mode", "break".to_owned()),
    ]
}

/// Bucket width. The set is closed and validated server-side: the view is a
/// `merge()` over every bronze database, so an operator-supplied interval
/// expression would be a direct route into that scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum Grain {
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "1s")]
    Second,
}

impl Grain {
    const FIFTEEN_MINUTES: &'static str = "15m";
    const SECOND: &'static str = "1s";

    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, CanonicalError> {
        match raw.unwrap_or(Self::FIFTEEN_MINUTES) {
            Self::FIFTEEN_MINUTES => Ok(Self::FifteenMinutes),
            Self::SECOND => Ok(Self::Second),
            _ => Err(violation(
                "grain",
                "expected one of: 15m, 1s",
                "UNSUPPORTED",
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::FifteenMinutes => Self::FIFTEEN_MINUTES,
            Self::Second => Self::SECOND,
        }
    }

    fn bucket_expr(self) -> &'static str {
        match self {
            Self::FifteenMinutes => "toStartOfInterval(extracted_at, INTERVAL 15 MINUTE)",
            Self::Second => "toStartOfSecond(extracted_at)",
        }
    }

    pub(crate) fn max_span(self) -> Duration {
        match self {
            Self::FifteenMinutes => Duration::days(400),
            Self::Second => Duration::hours(2),
        }
    }

    fn default_span(self) -> Duration {
        match self {
            Self::FifteenMinutes => Duration::days(1),
            Self::Second => Duration::minutes(30),
        }
    }
}

/// What one plotted band counts. `Total` exists because the full-period trend
/// plots a single line: grouping it by connector would multiply 400 days of
/// 15-minute buckets by the connector count for a series the chart then sums
/// back down anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Series {
    Connector,
    Stream,
    Total,
}

impl Series {
    const CONNECTOR: &'static str = "connector";
    const STREAM: &'static str = "stream";
    const TOTAL: &'static str = "total";

    pub(crate) fn parse(raw: Option<&str>, scoped: bool) -> Result<Self, CanonicalError> {
        let default = if scoped {
            Self::STREAM
        } else {
            Self::CONNECTOR
        };
        match raw.unwrap_or(default) {
            Self::CONNECTOR => Ok(Self::Connector),
            Self::STREAM => Ok(Self::Stream),
            Self::TOTAL => Ok(Self::Total),
            _ => Err(violation(
                "series",
                "expected one of: connector, stream, total",
                "UNSUPPORTED",
            )),
        }
    }

    fn key_expr(self) -> &'static str {
        match self {
            Self::Connector => "connector",
            Self::Stream => "stream",
            Self::Total => "'all'",
        }
    }
}

fn parse_scope(raw: Option<&str>) -> Result<Option<String>, CanonicalError> {
    let Some(scope) = raw.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let shaped = scope.len() <= 128
        && scope.starts_with("bronze_")
        && scope
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !shaped {
        return Err(violation(
            "scope",
            "expected a bronze database name, e.g. bronze_bamboohr",
            "MALFORMED",
        ));
    }
    Ok(Some(scope.to_owned()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub(crate) from: DateTime<Utc>,
    pub(crate) to: DateTime<Utc>,
}

impl Window {
    pub(crate) fn resolve(
        from: Option<&str>,
        to: Option<&str>,
        grain: Grain,
        now: DateTime<Utc>,
    ) -> Result<Self, CanonicalError> {
        let to = match to {
            Some(raw) => parse_instant("to", raw)?,
            None => now,
        };
        let from = match from {
            Some(raw) => parse_instant("from", raw)?,
            None => to - grain.default_span(),
        };
        if from >= to {
            return Err(violation("from", "must be earlier than `to`", "INVALID"));
        }
        if to - from > grain.max_span() {
            return Err(violation(
                "from",
                &format!(
                    "window wider than {} supports ({} days max at this grain)",
                    grain.as_str(),
                    grain.max_span().num_days().max(1),
                ),
                "OUT_OF_RANGE",
            ));
        }
        Ok(Self { from, to })
    }

    fn bound(instant: DateTime<Utc>) -> String {
        instant.to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

fn parse_instant(field: &str, raw: &str) -> Result<DateTime<Utc>, CanonicalError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|_| violation(field, "expected an RFC 3339 instant", "MALFORMED"))
}

fn violation(field: &str, description: &str, reason: &str) -> CanonicalError {
    IngestionError::invalid_argument()
        .with_field_violation(field, description, reason)
        .create()
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct IngestionIntensityQuery {
    /// Bucket width: `15m` (default) or `1s`.
    pub grain: Option<String>,
    /// Bronze database to scope to, e.g. `bronze_bamboohr`. Absent = org-wide.
    pub scope: Option<String>,
    /// What one band counts: `connector`, `stream` or `total`. Defaults to
    /// `stream` when `scope` is given, `connector` otherwise.
    pub series: Option<String>,
    /// Inclusive lower bound, RFC 3339. Defaults to one grain-span before `to`.
    pub from: Option<String>,
    /// Exclusive upper bound, RFC 3339. Defaults to now.
    pub to: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, clickhouse::Row, utoipa::ToSchema)]
pub struct IngestionPoint {
    /// Bucket start as `YYYY-MM-DD HH:MM:SS`, always UTC — the reader's own
    /// zone would re-cut buckets the server already decided.
    pub bucket: String,
    /// Connector slug, stream name, or `all`, per the resolved `series`.
    pub key: String,
    pub rows: u64,
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct IngestionIntensityResponse {
    /// Echoed resolved, not as asked: the caller may have pinned neither bound.
    pub grain: Grain,
    pub series: Series,
    pub from: String,
    pub to: String,
    /// The `source_database` the read was scoped to; absent when org-wide.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// The group cap clipped the tail: the window is too wide for this grain
    /// and series to plot honestly. Never silently true — the UI says so.
    pub truncated: bool,
    pub points: Vec<IngestionPoint>,
}
impl toolkit::api::api_dto::ResponseApiDto for IngestionIntensityResponse {}

/// `rows` and `key` are back-quoted: both are keywords in some parser
/// positions, and an alias is not the place to find out which.
pub(crate) fn intensity_sql(grain: Grain, series: Series, scoped: bool) -> String {
    let bucket = grain.bucket_expr();
    let key = series.key_expr();
    let scope_clause = if scoped {
        " AND source_database = ?"
    } else {
        ""
    };
    let limit = MAX_POINTS + 1;
    // `toString(DateTime64)` formats in the SERVER's timezone. The column is
    // timezone-less, the struct calls the result UTC and the chart appends a
    // `Z` to it, so a cluster running on anything but UTC would slide every
    // bucket by its offset. The zone is named here rather than assumed.
    format!(
        "SELECT toString({bucket}, 'UTC') AS bucket, {key} AS `key`, count() AS `rows` \
         FROM {VIEW} \
         WHERE extracted_at >= parseDateTime64BestEffort(?, 3) \
         AND extracted_at < parseDateTime64BestEffort(?, 3){scope_clause} \
         GROUP BY bucket, `key` \
         ORDER BY bucket ASC, `key` ASC \
         LIMIT {limit}"
    )
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IntensityPlan {
    pub(crate) grain: Grain,
    pub(crate) series: Series,
    pub(crate) scope: Option<String>,
    pub(crate) window: Window,
}

impl IntensityPlan {
    pub(crate) fn resolve(
        query: &IngestionIntensityQuery,
        now: DateTime<Utc>,
    ) -> Result<Self, CanonicalError> {
        let grain = Grain::parse(query.grain.as_deref())?;
        let scope = parse_scope(query.scope.as_deref())?;
        let series = Series::parse(query.series.as_deref(), scope.is_some())?;
        let window = Window::resolve(query.from.as_deref(), query.to.as_deref(), grain, now)?;
        Ok(Self {
            grain,
            series,
            scope,
            window,
        })
    }
}

pub async fn get_ingestion_intensity(
    Extension(state): Extension<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<IngestionIntensityQuery>,
) -> Result<impl IntoResponse, CanonicalError> {
    require_admin(&state, &headers, admin_only).await?;

    let IntensityPlan {
        grain,
        series,
        scope,
        window,
    } = IntensityPlan::resolve(&query, Utc::now())?;

    let from = Window::bound(window.from);
    let to = Window::bound(window.to);
    // INVARIANT: the permit is held across the `.await` below, so it bounds the
    // reads actually in flight rather than the calls that started one.
    let _permit = acquire_read_permit().await?;

    let mut request = state
        .ch
        .query(&intensity_sql(grain, series, scope.is_some()));
    for (setting, value) in aggregation_settings() {
        request = request.with_setting(setting, value);
    }
    request = request.bind(from.clone()).bind(to.clone());
    if let Some(ref database) = scope {
        request = request.bind(database.clone());
    }

    let mut points = request
        .fetch_all::<IngestionPoint>()
        .await
        .map_err(read_error)?;
    let truncated = points.len() >= MAX_POINTS;
    points.truncate(MAX_POINTS);

    Ok(Json(IngestionIntensityResponse {
        grain,
        series,
        from,
        to,
        scope,
        truncated,
        points,
    }))
}

async fn acquire_read_permit() -> Result<tokio::sync::SemaphorePermit<'static>, CanonicalError> {
    tokio::time::timeout(ACQUIRE_TIMEOUT, READ_SEMAPHORE.acquire())
        .await
        .map_err(|_| {
            tracing::warn!(
                capacity = MAX_CONCURRENT_READS,
                available = READ_SEMAPHORE.available_permits(),
                "ingestion intensity read capacity exhausted"
            );
            read_busy()
        })?
        .map_err(|_| read_busy())
}

fn read_busy() -> CanonicalError {
    IngestionError::resource_exhausted("Ingestion intensity read capacity is busy.")
        .with_quota_violation("ingestion intensity reads", "concurrency limit reached")
        .with_quota_violation_retry_after_seconds(ACQUIRE_TIMEOUT.as_secs())
        .create()
}

fn admin_only() -> CanonicalError {
    IngestionError::permission_denied()
        .with_reason(ADMIN_ONLY)
        .create()
}

#[expect(clippy::needless_pass_by_value, reason = "used directly as map_err")]
fn read_error(error: clickhouse::error::Error) -> CanonicalError {
    tracing::error!(error = %error, "ingestion intensity query failed");
    CanonicalError::internal("failed to read ingestion intensity").create()
}

#[cfg(test)]
mod tests;
