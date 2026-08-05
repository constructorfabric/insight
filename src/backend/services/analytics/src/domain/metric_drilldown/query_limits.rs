use clickhouse::query::Query;

use super::dto::{
    EVIDENCE_QUERY_MEMORY_BYTES, EVIDENCE_QUERY_READ_BYTES, EVIDENCE_QUERY_RESULT_BYTES,
    EVIDENCE_QUERY_TIMEOUT_SECS,
};

pub(crate) fn with_evidence_query_limits(query: Query) -> Query {
    query
        .with_setting(
            "max_execution_time",
            EVIDENCE_QUERY_TIMEOUT_SECS.to_string(),
        )
        .with_setting("max_memory_usage", EVIDENCE_QUERY_MEMORY_BYTES.to_string())
        .with_setting("max_bytes_to_read", EVIDENCE_QUERY_READ_BYTES.to_string())
        .with_setting("max_result_bytes", EVIDENCE_QUERY_RESULT_BYTES.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Building a query is lazy — no socket is opened, so the settings chain
    // runs connection-free.
    #[test]
    fn limits_apply_to_a_connection_free_query() {
        let client = clickhouse::Client::default().with_url("http://localhost:8123");
        let _query = with_evidence_query_limits(client.query("SELECT 1"));
    }
}
