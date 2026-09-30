# YouTrack transformation tests

These tests execute dbt against a disposable ClickHouse and reset its YouTrack
Bronze tables and observation models. Never point them at a shared database.

Use the versions in `scripts/bootstrap-db/pins.env`. Install `dbt-core`,
`dbt-clickhouse`, `clickhouse-connect`, `pytest` and `pyyaml` in a test environment.
Set `CLICKHOUSE_HOST`, `CLICKHOUSE_HTTP_PORT`, `CLICKHOUSE_USER`,
`CLICKHOUSE_PASSWORD` and `YOUTRACK_TEST_DISPOSABLE=yes`, then run:

```sh
pytest src/ingestion/dbt/tests/youtrack/transform
```

The scenarios cover cardinality changes, opaque event IDs, same-instant chains,
partial history, nulls and empty sets, late events, replacement in Silver,
removed/re-added links, retired fields, identity observations and time units.
