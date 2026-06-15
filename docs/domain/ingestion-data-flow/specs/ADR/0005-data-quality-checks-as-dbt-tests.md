---
status: accepted
date: 2026-06-09
decision-makers: aleksandr.barkhatov
---

# Data-quality checks as dbt tests emitted to the central log store

<!-- toc -->
<!-- /toc -->

**ID**: `cpt-dataflow-adr-data-quality-checks`

## Context and Problem Statement

The pipeline transforms raw connector data into silver tables and gold views. Data defects — a duplicated ingestion source inflating a metric, an impossible per-day duration, a join fan-out in a gold view — are silent: they reach dashboards without anyone noticing until a human spots a wrong number. We had singular SQL assertions in the dbt project, but nothing ran them after the initial authoring, and there was no place for their results to land. We need defects to surface routinely, in a structured form, without blocking ingestion when something looks wrong.

## Decision Drivers

- Defects must be caught on a schedule, not only when a person investigates.
- A finding must never freeze the pipeline on a stale snapshot.
- One catalog, one place to add a check — no parallel systems to keep in sync.
- Checks must cover both silver tables and gold views with the same mechanism.
- Results must be queryable and alertable by the central log store that already owns querying and notifications.

## Considered Options

- **A. dbt tests, results emitted as structured log lines.** Every check is a dbt test (`severity=warn`, `store_failures`); the scheduled run executes the catalog with `--log-format json` and a python post-step prints one JSON finding per check to stdout.
- **B. A dedicated backend endpoint with its own SQL check catalog.** A service holds a hand-written list of checks, runs them on request, and returns findings, persisting them to its own table.
- **C. A standalone SQL-runner job** with its own catalog and its own emitter, separate from dbt.

## Decision Outcome

Chosen option: **A**.

- **Checks are dbt tests.** Singular tests already are "SQL that returns violating rows"; a test passes when it returns none. Silver tests read the silver tables; gold tests read the `insight` views through a registered `gold` source. The view being built outside dbt is irrelevant to a read-only test, so one catalog covers both layers.
- **Findings are structured logs.** The runner executes the catalog with `--log-format json` (matching the dbt-run convention so the log collector parses dbt's events uniformly), then a python post-step reads dbt's `run_results.json` and `manifest.json` and prints one **top-level JSON** finding per check to stdout — `check_id`, `domain`, `category`, `gate`, `tier`, `status`, `rows_violating`, `duration_ms`, `audit_relation`, `remediation`, discriminated by `event="data_quality_finding"`. Built from artifacts rather than dbt's logger because dbt would otherwise nest the line inside its own log envelope. Low-cardinality fields are intended as log labels; the central log store handles query and alerting. No new table or endpoint owns findings.
- **Non-blocking.** Every data-quality check sets `severity=warn` (and `store_failures=true`) in its own config, so a data finding exits cleanly; only an operational error (e.g. the warehouse unreachable) fails the run. A check can be escalated to `severity=error` individually when a violation should block. Untagged structural tests keep dbt's default `error` severity, so build integrity stays strict.
- **Detail kept for drill-down.** `store_failures` writes each check's violating rows to an audit table; `audit_relation` in the finding points at it. The log line stays small; full rows are fetched on demand.
- **Silver/gold only — never bronze.** A check reads only the silver and gold layers, which exist regardless of the connector set (silver placeholders + migration-created gold views, per `cpt-dataflow` fresh-cluster bring-up). Bronze is per-connector and may be absent, so reading it would make a check error on tenants lacking that connector. With silver/gold-only, a missing connector class simply yields an empty table and a clean pass — the catalog is connector-agnostic with no per-tenant logic. Bronze/ingestion-correctness tests (e.g. silver-to-bronze traceability) are a separate suite run under `dbt build`, not the scheduled catalog.
- **Opt-in catalog.** Only tests tagged `data_quality` are monitored; the scheduled run selects them by tag. The generic structural tests (`not_null`/`unique`/`relationships`) and any environment-specific assertions stay under `dbt build` for build integrity and are deliberately excluded, so the operational catalog is intentional rather than "every test that exists".
- **Scheduled, decoupled.** A CronWorkflow runs the tagged catalog on its own schedule, independent of the per-connector pipeline, so a finding can never delay ingestion.

`gate` (build behavior, warn/error), `tier` (human triage importance), and `status` (the run outcome) are kept distinct rather than collapsed into one severity field, so "how the build reacts" and "how alarming this is" can differ.

### Consequences

- Good: one catalog; adding a check is adding a dbt test, nothing else.
- Good: silver and gold checked the same way.
- Good: findings flow to the system that already does querying and notifications, with no bespoke storage.
- Good: never blocks ingestion by default.
- Bad: findings live as logs (retention-bounded), not a durable fact store — acceptable for an operational signal; durable history can be added later from the audit tables if needed.
- Bad: the silver/gold-only rule and the per-check config conventions (`tags`, `severity`, `store_failures`) rely on review discipline rather than automated enforcement; acceptable while the catalog is small.

### Confirmation

- A scheduled run emits a finding per check; a deliberately violated invariant appears with `status=warn` and a non-zero `rows_violating`.
- `store_failures` populates the audit relation named in the finding.

## Future: when silver/gold stop being guaranteed

Today silver/gold are present regardless of the connector set, which is why the silver/gold-only rule needs no existence guards. If the platform moves to a plugin model where installing a connector creates its silver/gold relations on demand (so they can be genuinely absent), the extension point is small and additive: a `requires`-based skip guard — a check declares the relations it needs, a macro compiles it to a no-op when any are absent (keeping the scheduled run green), and the emitter reports those as `status=skipped` with a reason. `status` already flows through the emitter unchanged, so no rework of the catalog or the flow is needed. This is deliberately **not** built now: it cannot fire while silver/gold are guaranteed, and the probe it needs depends on the plugin model's eventual shape (per-plugin schema vs. database vs. table).

## More Information

- Runner and finding emitter: `charts/insight/templates/ingestion/data-quality-test.yaml`.
- Gold source registration: `src/ingestion/silver/_shared/gold_sources.yml`.
- Catalog selector: `src/ingestion/dbt/selectors.yml` (`data_quality`).
- Schedule and runner: `charts/insight/templates/ingestion/data-quality-cron.yaml`, `data-quality-test.yaml`.
- Authoring guide: `src/ingestion/dbt/tests/README.md`.

## Traceability

- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **Sibling ADRs**:
  - `cpt-dataflow-adr-rmt-with-version-and-unique-key` (the grain/dedup contract several checks assert)
