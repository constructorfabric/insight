use std::time::Duration;

use async_trait::async_trait;
use clickhouse::Row;
use insight_clickhouse::{Client, Config};
use serde::Deserialize;

use crate::domain::attribute_reconcile::{DiscoveredField, DiscoveredFieldsReader};

const READ_TIMEOUT: Duration = Duration::from_mins(5);

const DISCOVER_SQL: &str = r"
    SELECT
        ifNull(insight_tenant_id, '')            AS tenant,
        ifNull(insight_source_type, '')          AS source_type,
        ifNull(insight_source_id, '')            AS source_instance,
        ifNull(field_id, '')                     AS field,
        toString(max(observed_at))               AS last_observed
    FROM silver.class_person_attribute_claims FINAL
    GROUP BY tenant, source_type, source_instance, field
    ORDER BY tenant, source_type, source_instance, field
";

const UNKNOWN_TABLE_TOKEN: &str = "UNKNOWN_TABLE";

#[derive(Debug, Row, Deserialize)]
struct FieldRow {
    tenant: String,
    source_type: String,
    source_instance: String,
    field: String,
    last_observed: String,
}

pub struct ClickHouseDiscoveredFieldsReader {
    client: Client,
}

impl ClickHouseDiscoveredFieldsReader {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn connect(url: &str, database: &str, user: &str, password: &str) -> Self {
        let mut config = Config::new(url, database).with_query_timeout(READ_TIMEOUT);
        if !user.is_empty() {
            config = config.with_auth(user, password);
        }
        Self::new(Client::new(config))
    }
}

pub enum DiscoverOutcome {
    Fields(Vec<DiscoveredField>),
    ClaimsRelationMissing,
}

#[async_trait]
impl DiscoveredFieldsReader for ClickHouseDiscoveredFieldsReader {
    async fn discover(&self) -> anyhow::Result<Vec<DiscoveredField>> {
        let rows: Vec<FieldRow> = self.client.query(DISCOVER_SQL).fetch_all().await?;
        Ok(rows.into_iter().map(map_row).collect())
    }
}

impl ClickHouseDiscoveredFieldsReader {
    pub async fn discover_or_missing(&self) -> anyhow::Result<DiscoverOutcome> {
        match self
            .client
            .query(DISCOVER_SQL)
            .fetch_all::<FieldRow>()
            .await
        {
            Ok(rows) => Ok(DiscoverOutcome::Fields(
                rows.into_iter().map(map_row).collect(),
            )),
            Err(err) if is_unknown_table(&err) => Ok(DiscoverOutcome::ClaimsRelationMissing),
            Err(err) => Err(err.into()),
        }
    }
}

// WORKAROUND: matched on the named token rather than the numeric code — a bare
// "Code: 60" substring also matches codes 600-609.
fn is_unknown_table(err: &clickhouse::error::Error) -> bool {
    matches!(err, clickhouse::error::Error::BadResponse(msg) if msg.contains(UNKNOWN_TABLE_TOKEN))
}

fn map_row(r: FieldRow) -> DiscoveredField {
    DiscoveredField {
        insight_tenant_id: r.tenant,
        insight_source_type: r.source_type,
        insight_source_id: r.source_instance,
        source_field_id: r.field,
        last_observed_at: r.last_observed,
    }
}
