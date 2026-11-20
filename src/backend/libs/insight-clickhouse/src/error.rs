#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("clickhouse error: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),

    #[error("query timed out")]
    Timeout,

    #[error("invalid query: {0}")]
    InvalidQuery(String),

    /// The database this connection is configured for is missing, or its
    /// engine contradicts the configured topology. Carries its remedy.
    #[error("warehouse topology: {0}")]
    Topology(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_messages_are_stable() {
        assert_eq!(Error::Timeout.to_string(), "query timed out");
        assert_eq!(
            Error::InvalidQuery("bad table".into()).to_string(),
            "invalid query: bad table"
        );
    }
}
