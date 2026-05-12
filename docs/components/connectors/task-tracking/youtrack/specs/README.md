# YouTrack Connector — Specs Folder

This folder is the canonical home for the YouTrack connector's specification artifacts.

## Artifacts in this folder

| File | Status | What it covers |
|---|---|---|
| [`PRD.md`](./PRD.md) | **accepted** | Bronze-layer FRs / NFRs / actors / use cases. §5.1–§5.5 are fully specified for the implemented connector. §13 lists out-of-scope (future) capabilities. |
| [`DESIGN.md`](./DESIGN.md) | **accepted** | Bronze-layer architecture, components, sequences, database schemas. Covers manifest, 10 streams, identity stamping, K8s Secret contract, Builder-UI compatibility constraints. |
| [`DECOMPOSITION.md`](./DECOMPOSITION.md) | **proposed** | Ten-feature decomposition. §2.1–§2.4 (Bronze) are implemented and traceability-reconciled. §2.5–§2.10 (Silver/Enrich) remain forward-looking — see "Future scope" below. |
| [`ADR/`](./ADR/) | **3 ADRs accepted, 2 planned** | Connector-level Architecture Decision Records. The three Bronze-now ADRs are below; the two Enrich ADRs will land with feature §2.6. |

### Bronze-now ADRs (accepted)

- [`ADR/ADR-001-project-scoped-custom-fields.md`](./ADR/ADR-001-project-scoped-custom-fields.md) — Per-project substream for custom-field registry (vs global API).
- [`ADR/ADR-002-activitiespage-cursor-pagination.md`](./ADR/ADR-002-activitiespage-cursor-pagination.md) — Cursor (`afterCursor`/`hasAfter`) pagination on `youtrack_issue_history` (vs offset).
- [`ADR/ADR-003-no-whitelist-full-ingestion.md`](./ADR/ADR-003-no-whitelist-full-ingestion.md) — Ingest every project the token can reach (vs allowlist in K8s Secret).

### Future ADRs (planned, will land with §2.6)

- **Enrich ADR-001** — activitiesPage event-sourcing with backward replay.
- **Enrich ADR-002** — Multi-value backward replay semantics (id-first dedup with JSON fallback).
- **Inherited from Jira silver** (apply unchanged; not duplicated here):
  - `silver/jira/specs/ADR/ADR-001-rust-single-binary.md` — Rust single binary for enrich.
  - `silver/jira/specs/ADR/ADR-002-core-io-split.md` — core / io module split inside one crate.
  - `silver/jira/specs/ADR/ADR-003-ddl-owned-by-dbt.md` — Rust binary writes data only; dbt owns DDL via `on-run-start` macro.
  - `silver/jira/specs/ADR/ADR-004-cursorless-incremental.md` — full-replay-per-issue incremental strategy.
  - `silver/jira/specs/ADR/ADR-005-event-id-traceability.md` — `(insight_source_id, id_readable, event_id, field_id)` audit grain.
  - `silver/jira/specs/ADR/ADR-006-event-kind-column.md` — `event_kind ∈ {changelog, synthetic_initial}` discriminator.

## Current implementation status

The connector ships **Bronze-only** in PR [#227](https://github.com/cyberfabric/cyber-insight/pull/227): a Builder-UI-compatible declarative manifest with ten streams writing into `bronze_youtrack.*` ClickHouse tables. Verified against a live tenant: 76 492 records / 0 errors across all ten streams.

### Implemented (PR #227)

| DECOMPOSITION feature | Status | Files |
|---|---|---|
| §2.1 Bronze Airbyte Manifest Skeleton | ✅ implemented | `src/ingestion/connectors/task-tracking/youtrack/{connector.yaml,descriptor.yaml,dbt/schema.yml,README.md}`, `src/ingestion/secrets/connectors/youtrack.yaml.example` |
| §2.2 Bronze Directory Streams | ✅ implemented | `connector.yaml` streams `youtrack_projects`, `youtrack_user`, `youtrack_agiles`, `youtrack_sprints`, `youtrack_issue_link_types` |
| §2.3 Bronze Incremental Issues & Substreams | ✅ implemented (except `youtrack_issue_links` flat projection — deferred to §2.5) | `connector.yaml` streams `youtrack_issue`, `youtrack_issue_history`, `youtrack_comments`, `youtrack_worklogs`; `links_json` kept in `youtrack_issue` |
| §2.4 Project-Scoped Custom Field Ingestion | ✅ implemented | `connector.yaml` stream `youtrack_project_custom_fields` |

### Not implemented yet (future scope — separate PRs)

| DECOMPOSITION feature | Status | Why deferred |
|---|---|---|
| §2.5 dbt Connector-Level Staging | 🔲 pending | Required by §2.6 / §2.7 to feed the Rust replay engine. Independent of Bronze ingestion correctness. |
| §2.6 Rust `youtrack-enrich` — Core | 🔲 pending | Port of v2 `replay/*` algorithm to Rust. Donor code reference is the `monitor` repo. Large stand-alone deliverable. |
| §2.7 Rust `youtrack-enrich` — IO | 🔲 pending | ClickHouse client + main binary. Depends on §2.6. |
| §2.8 Argo Workflow & CLI Integration | 🔲 pending | `tt-enrich-youtrack-run.yaml` template + `ingestion-pipeline` branch + image build. Depends on §2.5 + §2.7. |
| §2.9 Silver Plug-In Verification | 🔲 pending | Verify `silver.class_task_*.source = 'youtrack'` rows exist. Depends on §2.5 + §2.7. |
| §2.10 Test Invariants & E2E Smoke | 🔲 pending | Reuse PR #205 invariants + YouTrack-specific Rust unit cases. Depends on §2.9 + §2.8. |

## Future scope — Stage C plan

The work below is **not in this PR's scope**. It is documented here so reviewers and follow-up implementers can pick it up without rebuilding context.

### Why staged delivery?

Bronze ingestion is independently valuable: operators can already query YouTrack data in ClickHouse via the `bronze_youtrack.*` tables. The Silver layer that turns those tables into source-agnostic productivity metrics is a separable concern with its own implementation cost (replay engine, Argo orchestration, dbt staging). Shipping Bronze first gives a faster feedback loop on connector correctness, lets us validate the Builder-UI compatibility approach against a real tenant, and unblocks downstream analytics teams who want raw YouTrack data without waiting for the full pipeline.

### Implementation roadmap (Stage C)

Each item below is a candidate follow-up PR. Order matters — features later in the list depend on features earlier in the list. Estimated effort (E:) is rough and assumes one implementer working with Phase 1 donor-code reference.

#### Phase C-1 — dbt Connector-Level Staging (DECOMPOSITION §2.5) — E: 2-3 days

Mirrors the Jira pattern from PR #205. Deliverable: nine SQL files in `src/ingestion/connectors/task-tracking/youtrack/dbt/`, plus an updated `dbt/schema.yml` source-block declaration and a flip of `descriptor.yaml` `dbt_select` from `""` to `tag:youtrack`.

- `youtrack__changelog_items.sql` — flatten `youtrack_issue_history.activities[]`; one row per `(issue_id, activity_id, field_id, added_item, removed_item)`. Respects v2 `applyBackward` semantics. Tag: `youtrack`. Material: `table`.
- `youtrack__issue_field_snapshot.sql` — current per-issue × per-field value from `youtrack_issue.customFields[]` + built-in fields. Tag: `youtrack`. Material: `table`. Input to the replay engine.
- `youtrack__task_comments.sql` — projection of `youtrack_comments` into the columns `class_task_comments` expects. Tags: `silver:class_task_comments`, `youtrack`. Material: `view`.
- `youtrack__task_worklogs.sql` — projection of `youtrack_worklogs`. Tags: `silver:class_task_worklogs`, `youtrack`. Material: `view`.
- `youtrack__task_users.sql` — projection of `youtrack_user`. Tags: `silver:class_task_users`, `youtrack`. Material: `view`.
- `youtrack__task_projects.sql` — projection of `youtrack_projects`. Tags: `silver:class_task_projects`, `youtrack`. Material: `view`.
- `youtrack__task_sprints.sql` — projection of `youtrack_sprints`. Tags: `silver:class_task_sprints`, `youtrack`. Material: `view`.
- `youtrack__task_field_metadata.sql` — projection of `youtrack_project_custom_fields` into `(field_id, name, is_multi, has_id, cardinality)` columns. Tags: `silver:class_task_field_metadata`, `youtrack`. Material: `view`.
- `youtrack__task_field_history.sql` — thin view over the Rust-written `staging.youtrack__task_field_history` table (DDL owned by the shared `create_task_field_history_staging` macro from PR #205). Tags: `silver:class_task_field_history`, `youtrack`. Material: `view`.
- `youtrack__bronze_promoted.sql` — bronze-promoted validator (per PR #363 contract). Empty result = pass. Tag: `bronze_promoted_test`.

#### Phase C-2 — Rust `youtrack-enrich` Core (DECOMPOSITION §2.6) — E: 1-2 weeks

Cargo package at `src/ingestion/connectors/task-tracking/youtrack/enrich/`. The core module ports the v2 `replay/*` TypeScript algorithm to Rust. Output schema matches `jira-enrich` so the silver `class_task_field_history` union works transparently.

- `Cargo.toml`, `Dockerfile`, `build.sh`, `README.md` — package scaffolding.
- `src/core/types.rs` — `YTActivityItem`, `YTIssue`, `IssueStateEntry`, `EventKind`, `ChangeSet<T>`, `FieldId` enum (CustomField / Builtin / TargetMember).
- `src/core/youtrack.rs` — port of `applyBackward` for single- and multi-value fields; port of `deriveFieldId` fallback chain.
- `src/core/mod.rs` — orchestration: `build_initial_state(issue)`, `replay(issue, activities) -> Vec<IssueStateEntry>`, `synthetic_initial` emission, `_seq` disambiguation for same-timestamp activities.
- `src/core/tests.rs` — unit tests covering every activity category and edge cases from v2 `applyBackward.test.ts` + `replayIssue.test.ts`.

Donor: `monitor` repo, `sources/youtrack/src/{youtrack/types.ts,youtrack/client.ts,replay/*}` (v2) — accessed by maintainers via Phase 1 research notes.

ADRs landed with this phase: **Enrich ADR-001** (event-sourcing with backward replay), **Enrich ADR-002** (multi-value backward semantics).

#### Phase C-3 — Rust `youtrack-enrich` IO (DECOMPOSITION §2.7) — E: 3-5 days

Mirrors Jira's IO layer.

- `src/io/ch_client.rs` — ClickHouse client with `with_validation(false)` (per `silver/jira/specs/ADR/ADR-002`), per-batch INSERT timeout (default 60 s configurable).
- `src/io/reader.rs` — batched SELECT from `staging.youtrack__changelog_items` and `staging.youtrack__issue_field_snapshot`; group by `issue_id`; stream `(YTIssue, Vec<YTActivityItem>)` to core.
- `src/io/writer.rs` — INSERT `IssueStateEntry` rows into `staging.youtrack__task_field_history` with tenant/source tagging.
- `src/io/schema.rs` — assert staging table schema matches expected columns (fail-fast).
- `src/io/mod.rs` — IO surface.
- `src/main.rs` — CLI: `--tenant`, `--issue-batch-size`, `--per-batch-timeout-secs`, `--log-progress-every-n`, `--dry-run`. ClickHouse creds from K8s Secret.
- `src/ingestion/run-tt-enrich-youtrack.sh` — shell wrapper mirroring `run-tt-enrich-jira.sh`.

#### Phase C-4 — Argo Workflow & CLI Integration (DECOMPOSITION §2.8) — E: 1-2 days

- `charts/insight/templates/ingestion/tt-enrich-youtrack-run.yaml` — new WorkflowTemplate, symmetric to `tt-enrich-jira-run.yaml`.
- Update the `ingestion-pipeline` template — add the `youtrack` branch; raise `airbyte-sync` poll deadline if first-time sync exceeds default.
- Update `src/ingestion/tools/toolbox/build.sh` — build `youtrack-enrich` image (add to connectors array or generalize).
- Verify `run-sync.sh youtrack <tenant>` submits the full pipeline.

#### Phase C-5 — Silver Plug-In Verification (DECOMPOSITION §2.9) — E: 1 day

- After C-1 and C-3 land, run `dbt run --select tag:silver` and verify every `class_task_*` table contains rows with `source = 'youtrack'`.
- Update `src/ingestion/silver/task-tracking/schema.yml` — add notes (only) for YouTrack-specific caveats (missing-email fallback, multi-value cardinality quirks). No new models, no column changes.

#### Phase C-6 — Test Invariants & E2E Smoke (DECOMPOSITION §2.10) — E: 2-3 days

- Rust unit tests — extend `src/ingestion/connectors/task-tracking/youtrack/enrich/src/core/tests.rs` with fixtures covering every activity category enumerated in Connector ADR-002.
- dbt tests — run `dbt test --select tag:task` and verify all 11 invariants from PR #205 pass for YouTrack rows.
- E2E smoke on test-tenant:
  1. Apply K8s Secret with test-tenant creds.
  2. Submit `./src/ingestion/run-sync.sh youtrack <tenant>`.
  3. Record bronze counts: `youtrack_issue`, `youtrack_issue_history`, `youtrack_comments`, `youtrack_worklogs`.
  4. Record silver counts: every `class_task_*` table — row count where `source = 'youtrack'`.
  5. Second run → bronze/silver idempotency (counts unchanged).
  6. Retry scenario — kill one Argo step mid-run, resume, verify final state.
- Write smoke-run report to a new `test-scenarios.md` in this folder (mirrors the Jira `test-scenarios.md`).

### Spec amendments expected with Stage C

When Stage C lands, the following spec edits should ship alongside the code:

- **PRD.md** — add §5.6 "Per-source dbt staging" (FRs for §2.5), §5.7 "Replay engine" (FRs for §2.6 / §2.7), §5.8 "Orchestration" (FRs for §2.8), §5.9 "Silver plug-in" (FRs for §2.9), §5.10 "Tests" (FRs for §2.10). Move §13 "Out of Scope" entries into §4.1 "In Scope" as features ship.
- **DESIGN.md** — add §2.1 principles for Rust core/io split, event-sourcing replay; add §2.2 constraints for Rust binary, ClickHouse client validation-false; add §3.2 components for every Rust module + dbt model + Argo template; add §3.6 sequences for replay batch loop + Argo branch.
- **ADR/** — add Enrich ADR-001 (event-sourcing) and Enrich ADR-002 (multi-value backward semantics).
- **DECOMPOSITION.md** — flip all `[ ]` in §2.5–§2.10 to `[x]`; remove "(planned)" annotations; promote `status: proposed` → `status: accepted`.
- **test-scenarios.md** — new file mirroring Jira's, with concrete bronze + silver count tables.

When all of the above is done, the YouTrack connector reaches full feature-parity with the Jira connector delivered by PR #205.
