use clickhouse::Client;

/// Configuration for a ClickHouse connection.
#[derive(Debug, Clone)]
pub struct ChConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl ChConfig {
    #[must_use]
    pub fn http_url(&self) -> String {
        // The `clickhouse` crate drives the HTTP interface. Operators pass the HTTP
        // port explicitly; we honor it verbatim so non-default deployments (e.g. a
        // proxy on 8443) work without a code change.
        format!("http://{}:{}", self.host, self.port)
    }

    #[must_use]
    pub fn client(&self) -> Client {
        Client::default()
            .with_url(self.http_url())
            .with_user(&self.user)
            .with_password(&self.password)
            .with_database(&self.database)
            // Disable schema validation: `Client::insert()` with validation runs a pre-flight
            // `DESCRIBE TABLE` and uses `RowBinaryWithNamesAndTypes`. Our `FieldHistoryInsert`
            // struct matches the DDL column order exactly, so plain `RowBinary` is safe and
            // avoids the DESCRIBE round-trip.
            .with_validation(false)
            // Long-running cursors on multi-million-row tables hit CH's default HTTP send
            // timeouts (30s) and cause unexpected EOFs.
            .with_option("send_timeout", "3600")
            .with_option("receive_timeout", "3600")
    }
}
