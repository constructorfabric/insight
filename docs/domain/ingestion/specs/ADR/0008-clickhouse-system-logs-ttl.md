---
id: cpt-ingestion-adr-clickhouse-system-logs-ttl
status: accepted
date: 2026-05-22
---

# ADR-0008 — TTL for ClickHouse system.*_log tables


<!-- toc -->

- [Context](#context)
- [Decision](#decision)
- [Consequences](#consequences)
- [Migration](#migration)
- [Alternatives considered](#alternatives-considered)

<!-- /toc -->

## Context

ClickHouse 25.3 ships with system event logging enabled by default
(`query_log`, `query_thread_log`, `query_metric_log`, `trace_log`,
`part_log`, `metric_log`, `asynchronous_metric_log`, `text_log`,
`processors_profile_log`, `latency_log`, `error_log`). The tables
have no TTL by default; they grow unbounded.

On our dev cluster, ≈2 weeks of Airbyte ingestion + dbt rebuilds +
background merges produced **~113 000 active parts** of `query_log`
alone. ClickHouse continuously tried to consolidate them; each merge
created a `tmp_merge_*` directory equal in size to the source parts,
forcing the PVC up to **55 GB** and the WSL2 VHDX up to **336 GB**.
The VHDX growth filled the host disk before either system surfaced an
error in monitoring.

The trigger is not user query volume — even an idle cluster writes a
trace_log row per second. Without TTL the only retention mechanism is
manual `TRUNCATE`, which is not a sustainable answer.

## Decision

The bundled `helmfile/charts/clickhouse/` chart mounts a ConfigMap at
`/etc/clickhouse-server/config.d/system-logs-ttl.xml` that sets a
`<ttl>` element on every system.*_log table. Default retention is
**1 day**; tables, retention, and the on/off switch are values
(`systemLogsTtl.{enabled,days,logs}`).

ClickHouse reads `config.d/*.xml` on startup and applies the TTL to
newly-created log tables. It does **not** retroactively alter
already-existing tables (their schema is fixed at first creation).

## Consequences

- Fresh installs of this chart get bounded system-log growth out of
  the box — the disk-exhaustion incident cannot recur on a clean
  cluster.
- Existing clusters keep the old (no-TTL) schema until the operator
  runs `ALTER TABLE system.<name>_log MODIFY TTL event_date + INTERVAL
  1 DAY` once per table. The chart change alone is insufficient for
  existing data — manual one-time migration is required.
- Setting `days` higher than 1 (e.g. for debugging) is a values
  override; the chart does not assume the dev default fits every
  environment.
- The XML config layer is the standard CH extension point; subsequent
  log-related decisions (sampling, disabling specific logs, separate
  retention per log) extend the same ConfigMap without further chart
  surgery.
- `helm upgrade` on an already-running cluster adds a new volume
  mount to the StatefulSet pod template, which triggers a rolling
  pod replacement. Schedule the upgrade during a low-activity window
  to avoid aborting in-flight Airbyte syncs.

## Migration

For each cluster already running before this PR:

First inventory what's actually present (CH versions differ — some
tables may not exist):

```sql
SELECT name FROM system.tables
WHERE database = 'system' AND name LIKE '%\_log';
```

Then `MODIFY TTL` each one that exists:

```sql
ALTER TABLE system.query_log              MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.query_thread_log       MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.query_metric_log       MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.trace_log              MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.part_log               MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.metric_log             MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.asynchronous_metric_log MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.text_log               MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.processors_profile_log MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.latency_log            MODIFY TTL event_date + INTERVAL 1 DAY;
ALTER TABLE system.error_log              MODIFY TTL event_date + INTERVAL 1 DAY;
```

This is a one-time operation per cluster.

ClickHouse normalises the TTL expression to
`event_date + toIntervalDay(1)` in `SHOW CREATE TABLE` output
regardless of whether it was set via `config.d` XML (`INTERVAL 1 DAY
DELETE`) or `MODIFY TTL` SQL (`INTERVAL 1 DAY` — `DELETE` is the
default action and is optional). Both forms converge to the same
stored expression and the same 1-day retention.

## Alternatives considered

- **Disable system logs entirely** (`<query_log remove="1"/>`). Loses
  ad-hoc debugging traceability for a problem we've never had to
  trade off against. Rejected.
- **TTL via dbt post-hook or seed migration.** dbt has no access to
  the `system` database in the way it has to user databases; adding
  this would require operator credentials in dbt config, which is a
  larger blast radius than the chart-level config. Rejected.
- **Periodic external job that runs `TRUNCATE`.** Adds an
  operational artefact (CronWorkflow, alerting) for what is a
  one-line config in ClickHouse itself. Rejected.
