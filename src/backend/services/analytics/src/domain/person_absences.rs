use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::domain::metric_results::compiler::{account_id_expr, account_source_uuid_expr};
use crate::domain::metric_results::{CompiledQuery, ValidatedMetricResultsRequest};
use crate::infra::metrics::QueryKind;
use crate::infra::query::fetch_json_rows;

const ABSENCE_FETCH_TIMEOUT_SECS: u64 = 5;

#[derive(Debug, Deserialize, Serialize, utoipa::ToSchema)]
pub struct PersonAbsenceContext {
    pub person_id: String,
    pub period_overlap: bool,
    pub compare_to_overlap: Option<bool>,
}

pub(crate) async fn load(
    client: &insight_clickhouse::Client,
    request: &ValidatedMetricResultsRequest,
) -> Vec<PersonAbsenceContext> {
    let Some(query) = compile(request) else {
        return Vec::new();
    };

    let context = fetch_json_rows(
        client,
        &query.sql,
        &query.params,
        QueryKind::AbsenceContext,
        "metric-results:absence-context",
    );

    match tokio::time::timeout(Duration::from_secs(ABSENCE_FETCH_TIMEOUT_SECS), context).await {
        Ok(Ok(context)) => context,
        Ok(Err(_)) => Vec::new(),
        Err(_) => {
            tracing::warn!("absence context load timed out");
            Vec::new()
        }
    }
}

fn compile(request: &ValidatedMetricResultsRequest) -> Option<CompiledQuery> {
    let ids = request.entity.person_ids()?;
    if ids.is_empty() {
        return None;
    }

    let mut params = vec![request.to.to_string(), request.from.to_string()];
    let comparison = request.compare_to.map_or_else(
        || "CAST(NULL AS Nullable(Bool))".to_owned(),
        |window| {
            params.extend([window.to.to_string(), window.from.to_string()]);
            "toBool(max(a.start_date <= toDate(?) AND a.end_date >= toDate(?)))".to_owned()
        },
    );
    params.push(
        ids.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    let earliest = request
        .compare_to
        .map_or(request.from, |window| request.from.min(window.from));
    let latest = request
        .compare_to
        .map_or(request.to, |window| request.to.max(window.to));
    params.extend([latest.to_string(), earliest.to_string()]);
    let tenant_filter = if request.enforce_tenant_scope {
        params.push(request.tenant_id.to_string());
        "AND a.insight_tenant_id = ?"
    } else {
        ""
    };
    let source_id = account_source_uuid_expr("a");
    let account_id = account_id_expr("a");
    let sql = format!(
        "SELECT toString(binding.person_id) AS person_id,
            toBool(max(a.start_date <= toDate(?) AND a.end_date >= toDate(?))) AS period_overlap,
            {comparison} AS compare_to_overlap
         FROM silver.class_person_absences AS a FINAL
         INNER JOIN identity.account_assignment AS binding
             ON binding.source_type = a.account_source_type
            AND binding.source_id = {source_id}
            AND binding.account_id = {account_id}
         WHERE toString(binding.person_id) IN splitByChar(',', ?)
           AND a.start_date <= toDate(?) AND a.end_date >= toDate(?)
           {tenant_filter}
         GROUP BY binding.person_id
         HAVING period_overlap OR coalesce(compare_to_overlap, false)
         ORDER BY person_id
         SETTINGS max_execution_time = 5, max_rows_to_read = 1000000,
             max_bytes_to_read = 67108864, max_result_rows = 5000,
             max_result_bytes = 1048576, result_overflow_mode = 'throw'"
    );
    Some(CompiledQuery { sql, params })
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use uuid::Uuid;

    use super::{PersonAbsenceContext, compile};
    use crate::domain::metric_results::ValidatedEntitySelection;
    use crate::domain::metric_results::ValidatedMetricResultsRequest;

    type R = Result<(), Box<dyn std::error::Error>>;

    const CLICKHOUSE_URL_VAR: &str = "INTEGRATION_TESTS_CLICKHOUSE_URL";
    const ABSENCE_MIGRATION: &str = include_str!(
        "../../../../../ingestion/scripts/migrations/20260916000000_person-absences.sql"
    );

    fn warehouse_client() -> Option<insight_clickhouse::Client> {
        let url = std::env::var(CLICKHOUSE_URL_VAR).unwrap_or_default();
        if url.is_empty() {
            eprintln!("skipping: {CLICKHOUSE_URL_VAR} not set");
            return None;
        }

        let mut config = insight_clickhouse::Config::new(url, "default");
        if let (Ok(user), Ok(password)) = (
            std::env::var("INTEGRATION_TESTS_CLICKHOUSE_USER"),
            std::env::var("INTEGRATION_TESTS_CLICKHOUSE_PASSWORD"),
        ) && !user.is_empty()
        {
            config = config.with_auth(user, password);
        }

        Some(insight_clickhouse::Client::new(config))
    }

    async fn prepare_warehouse(client: &insight_clickhouse::Client) -> R {
        for statement in [
            "CREATE DATABASE IF NOT EXISTS identity",
            "CREATE DATABASE IF NOT EXISTS silver",
            "CREATE TABLE IF NOT EXISTS identity.account_assignment
             (source_type String, source_id UUID, account_id String, person_id UUID,
              created_at DateTime64(6))
             ENGINE = MergeTree ORDER BY (source_type, source_id, account_id)",
            ABSENCE_MIGRATION,
        ] {
            client.query(statement).execute().await?;
        }
        Ok(())
    }

    fn request() -> Result<ValidatedMetricResultsRequest, Box<dyn std::error::Error>> {
        Ok(ValidatedMetricResultsRequest {
            tenant_id: Uuid::from_u128(1),
            entity: ValidatedEntitySelection::Person {
                ids: vec![Uuid::from_u128(2)],
            },
            from: NaiveDate::parse_from_str("2025-05-01", "%Y-%m-%d")?,
            to: NaiveDate::parse_from_str("2025-05-31", "%Y-%m-%d")?,
            compare_to: None,
            metrics: Vec::new(),
            enforce_tenant_scope: true,
        })
    }

    #[test]
    fn only_authorized_person_ids_and_bound_dates_enter_the_query() -> R {
        let query = compile(&request()?).ok_or("missing query")?;
        assert!(query.sql.contains("FINAL"));
        assert!(query.sql.contains("a.insight_tenant_id = ?"));
        assert!(query.sql.contains("CAST(NULL AS Nullable(Bool))"));
        assert_eq!(query.sql.matches('?').count(), query.params.len());
        assert_eq!(query.params[0], "2025-05-31");
        assert_eq!(query.params[1], "2025-05-01");
        assert_eq!(query.params[2], Uuid::from_u128(2).to_string());
        Ok(())
    }

    #[test]
    fn response_contains_only_identity_and_overlap_indicators() -> R {
        let value = serde_json::to_value(PersonAbsenceContext {
            person_id: Uuid::from_u128(2).to_string(),
            period_overlap: true,
            compare_to_overlap: None,
        })?;
        assert_eq!(value.as_object().ok_or("not an object")?.len(), 3);
        assert!(value["compare_to_overlap"].is_null());
        Ok(())
    }

    #[test]
    fn tenant_requests_do_not_load_person_context() -> R {
        let mut request = request()?;
        request.entity = ValidatedEntitySelection::Tenant {
            id: request.tenant_id,
        };
        assert!(compile(&request).is_none());
        Ok(())
    }

    #[test]
    fn comparison_window_is_bound_independently() -> R {
        let mut request = request()?;
        request.compare_to = Some(crate::domain::metric_results::DateWindow {
            from: NaiveDate::parse_from_str("2025-04-01", "%Y-%m-%d")?,
            to: NaiveDate::parse_from_str("2025-04-30", "%Y-%m-%d")?,
        });
        let query = compile(&request).ok_or("missing query")?;
        assert_eq!(query.sql.matches('?').count(), query.params.len());
        assert_eq!(&query.params[2..4], ["2025-04-30", "2025-04-01"]);
        assert_eq!(&query.params[5..7], ["2025-05-31", "2025-04-01"]);
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_absence_response_returns_empty_after_client_deadline() -> R {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let client =
            insight_clickhouse::Client::new(insight_clickhouse::Config::new(url, "default"));
        let started = tokio::time::Instant::now();

        let context = super::load(&client, &request()?).await;

        assert!(context.is_empty());
        assert_eq!(
            started.elapsed(),
            std::time::Duration::from_secs(super::ABSENCE_FETCH_TIMEOUT_SECS)
        );
        drop(listener);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires isolated ClickHouse; set INTEGRATION_TESTS_CLICKHOUSE_URL to enable"]
    async fn warehouse_resolves_only_requested_people_and_overlapping_intervals() -> R {
        let Some(client) = warehouse_client() else {
            return Ok(());
        };
        prepare_warehouse(&client).await?;

        let mut request = request()?;
        request.tenant_id = Uuid::new_v4();
        let person = Uuid::new_v4();
        request.entity = ValidatedEntitySelection::Person { ids: vec![person] };
        let source = Uuid::new_v4().to_string();
        client
            .query(
                "INSERT INTO identity.account_assignment
             (source_type, source_id, account_id, person_id, created_at)
             SELECT 'bamboohr', toUUID(UUIDNumToString(sipHash128(?))),
                    'account-example', toUUID(?), now64(6)",
            )
            .bind(source.as_str())
            .bind(person.to_string())
            .execute()
            .await?;

        for (tenant, account_source, account, from, to) in [
            (
                request.tenant_id,
                source.as_str(),
                " ACCOUNT-EXAMPLE ",
                "2025-05-31",
                "2025-06-02",
            ),
            (
                request.tenant_id,
                "other-source",
                "account-example",
                "2025-04-01",
                "2025-04-30",
            ),
            (
                request.tenant_id,
                source.as_str(),
                "unmapped-account",
                "2025-04-01",
                "2025-04-30",
            ),
            (
                Uuid::new_v4(),
                source.as_str(),
                "account-example",
                "2025-04-01",
                "2025-04-30",
            ),
        ] {
            client
                .query(
                    "INSERT INTO silver.class_person_absences
                 SELECT ?, ?, 'bamboohr', ?, ?, toDate(?), toDate(?)",
                )
                .bind(tenant.to_string())
                .bind(Uuid::new_v4().to_string())
                .bind(account_source)
                .bind(account)
                .bind(from)
                .bind(to)
                .execute()
                .await?;
        }

        let context = super::load(&client, &request).await;
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].person_id, person.to_string());
        assert!(context[0].period_overlap);
        assert_eq!(context[0].compare_to_overlap, None);

        request.compare_to = Some(crate::domain::metric_results::DateWindow {
            from: NaiveDate::parse_from_str("2025-04-01", "%Y-%m-%d")?,
            to: NaiveDate::parse_from_str("2025-04-30", "%Y-%m-%d")?,
        });
        let context = super::load(&client, &request).await;
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].compare_to_overlap, Some(false));

        request.compare_to = Some(crate::domain::metric_results::DateWindow {
            from: request.from,
            to: request.to,
        });
        request.from = NaiveDate::parse_from_str("2025-07-01", "%Y-%m-%d")?;
        request.to = NaiveDate::parse_from_str("2025-07-31", "%Y-%m-%d")?;
        let context = super::load(&client, &request).await;
        assert_eq!(context.len(), 1);
        assert!(!context[0].period_overlap);
        assert_eq!(context[0].compare_to_overlap, Some(true));

        request.compare_to = None;
        assert!(super::load(&client, &request).await.is_empty());
        request.from = NaiveDate::parse_from_str("2025-05-01", "%Y-%m-%d")?;
        request.to = NaiveDate::parse_from_str("2025-05-31", "%Y-%m-%d")?;
        request.entity = ValidatedEntitySelection::Person {
            ids: vec![Uuid::new_v4()],
        };
        assert!(super::load(&client, &request).await.is_empty());
        Ok(())
    }
}
