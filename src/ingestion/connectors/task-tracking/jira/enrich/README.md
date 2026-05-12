# jira-enrich

Rust binary that materializes `silver.task_tracker_field_history` for the Jira source.

Design docs: [`docs/components/connectors/task-tracking/silver/jira/specs/`](../../../../../../docs/components/connectors/task-tracking/silver/jira/specs/).

## Scope

Inseparable part of the Jira connector. Owns exactly one Silver table:

- `silver.task_tracker_field_history` (Jira rows only).

Everything else — including `task_tracker_field_metadata` and all supporting tables — is produced by dbt models at `src/ingestion/connectors/task-tracking/jira/dbt/`.

## Development

Two build profiles:

```bash
# Core only — pure enrich logic; builds on rustc ≥ 1.80.
cargo test
cargo clippy -- -D warnings
cargo fmt

# Full binary with ClickHouse I/O — requires rustc ≥ 1.89.
cargo build --features io --release
```

The `io` module is feature-gated because `clickhouse = "0.14"` needs rustc 1.89+.
Local `cargo test` stays fast and green on older toolchains; CI and the Docker build
use `--features io` for the real binary.

Integration tests against a real ClickHouse (via testcontainers) will live under
`tests/` and require `--features io`.

## Structure

```
src/
├── main.rs         -- CLI + wiring (orchestrates reader → core → writer)
├── core/           -- pure enrich logic; zero I/O; unit-tested
│   ├── mod.rs       process_issue, apply_delta, reverse_delta, emit_*_row
│   ├── types.rs     Delta, FieldMeta, DeltaEvent, IssueSnapshot, FieldHistoryRecord
│   ├── jira.rs      bronze row → Delta converter (Sprint, Labels, Assignee, ...)
│   └── tests.rs     unit tests — 23 passing
└── io/             -- ClickHouse reader/writer/schema validation (feature = "io")
    ├── mod.rs       IoError type
    ├── ch_client.rs connection config
    ├── schema.rs    startup schema validation
    ├── reader.rs    per-issue HWM + last-state + bronze events/snapshots
    └── writer.rs    batched INSERT into task_tracker_field_history
```

## Running locally

Not runnable yet — skeleton only. The full CLI:

```
jira-enrich \
  --insight-source-id jira-alpha \
  --clickhouse-host localhost \
  --clickhouse-port 9000 \
  --batch-size 10000
```

Credentials via env: `CLICKHOUSE_PASSWORD`.
