# ADR-0010: Materialized SCD2 Cache for Person Parent/Child Edges (`org_chart`)

**ID**: `cpt-insightspec-adr-0010-org-chart-cache`

**Status:** Accepted

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Materialized SCD2 edge table, two-source rebuild, no stubs (chosen)](#materialized-scd2-edge-table-two-source-rebuild-no-stubs-chosen)
  - [Single-source rebuild from parent_person_id only](#single-source-rebuild-from-parentpersonid-only)
  - [Synthesise stub persons for unresolved parent_emails](#synthesise-stub-persons-for-unresolved-parentemails)
  - [Live recursive CTE against persons](#live-recursive-cte-against-persons)
  - [AFTER INSERT trigger maintaining edges in real time](#after-insert-trigger-maintaining-edges-in-real-time)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

cyberfabric/cyber-insight#348 calls for a per-person organisational
view that returns the parent and direct subordinates for any
`person_id`, and ultimately a recursive subchart endpoint
(`GET /v1/subchart/{person_id}?depth=N`). The data lives in `persons`
as either:

- `value_type='parent_person_id'` — already-resolved Insight UUIDs,
  written by a reconciliation service that does not yet exist in the
  codebase; on the current pipeline no row of this kind is ever
  produced.
- `value_type='parent_email'` — raw supervisor email from the
  connector (notably BambooHR `supervisorEmail`), unresolved.

Querying either ad-hoc per request has two problems:

- A recursive walk against `persons` must dedupe latest-per-partition
  inside the recursion, which compounds query cost at every level.
- Direct-subordinate lookups (`children of X within source S`) require
  scanning `parent_person_id` observations on the textual `value_id`
  index — workable for one hop, slow when called repeatedly during a
  Phase-3 subchart traversal.

Phase 1 is the storage layer that the Phase-2 endpoint enrichment and
Phase-3 subchart can both build on without re-doing the partition
math or the email resolution each call. No API surface changes in
Phase 1.

## Decision Drivers

- Each `(insight_tenant_id, insight_source_type, insight_source_id)`
  is a separate tree — Alice's manager in BambooHR is not the same
  edge as her channel admin in Slack, and the storage must keep them
  distinct.
- Edges change over time (people move under new managers); historical
  queries "who was X's parent on date T" are a stated consumer need.
- The reconciliation service that would write `parent_person_id`
  observations does not exist yet; in the current pipeline the only
  signal for org-chart is `parent_email`. A rebuild that consumes
  `parent_person_id` only would produce zero edges in production.
- The rebuild's source (`persons`) is append-only with latest-per-
  partition semantics (ADR-0003), so a derived cache stays
  deterministic.
- The Phase-3 endpoint needs a depth-bounded recursive walk; the
  cache shape should make `WHERE parent_person_id = X AND
  valid_to IS NULL` an index seek, not a scan.
- The seed pipeline already rebuilds `account_person_map` via the
  same two-table-swap pattern; symmetry beats invention.

## Considered Options

- A materialized SCD2 edge table (`org_chart`) rebuilt by the
  Python seeder from two sources: `parent_person_id` observations
  (future-proof) UNION `parent_email` observations resolved via JOIN
  to current email-bearers. Unresolved parent_emails are skipped, no
  stubs synthesised.
- The same edge table but rebuilt from `parent_person_id` only,
  waiting for the reconciliation service to materialise.
- Same edge table but synthesise stub persons for unresolved
  parent_emails so every parent_email yields an edge.
- A live recursive CTE against `persons` with no materialized cache.
- An AFTER INSERT trigger on `persons` that maintains the cache row
  by row.

## Decision Outcome

Adopt `org_chart`: a materialized SCD2 cache of direct edges,
rebuilt from `persons` by the Python seeder
(`seed-persons-from-identity-input.py` step 9) using the same
two-table-swap pattern as `account_person_map`. The rebuild sources
edges from a UNION of two queries:

1. **Source 1 — `parent_person_id` observations.** Reserved for the
   reconciliation service that will resolve `parent_email` →
   `person_id` and write the result as a `parent_person_id`
   observation. Currently zero rows in production; the path stays
   live so the rebuild becomes useful for downstream callers as soon
   as the reconciliation service ships.

2. **Source 2 — `parent_email` observations resolved by JOIN.** For
   every `parent_email` row, find the person within the same tenant
   whose latest `email` observation matches (lowercase, trimmed). The
   JOIN's right side picks one `person_id` per (tenant, email) via
   `ROW_NUMBER() OVER (...) WHERE rn=1` ordered by `created_at DESC`
   so pending-iresolution accumulation cannot break the UNIQUE key on
   `org_chart`. Source 1 takes precedence when both sources
   have a row for the same partition (NOT EXISTS guard in Source 2).

**Source 1 precedence is per-partition, not per-period.** The
NOT EXISTS guard in Source 2 checks for *any* `parent_person_id`
observation on the partition `(tenant, child, source_type, source_id)`
— including superseded ones. If a future reconciliation service
writes a `parent_person_id` at time T_resolve and then later writes
a corrected one at T_correct, Source 2's `parent_email` rows for
the same partition are **all** excluded, even periods that predate
T_resolve. This is acceptable for Phase 1 because Source 1 is empty
in the current pipeline; once reconciliation ships, the guard
should be tightened to per-interval (intersect `parent_email`
periods with the **gaps** in Source 1 coverage rather than excluding
the partition wholesale). Tracked as a Phase-2/3 follow-up; not a
correctness concern today.

**Deactivation handling — active intervals.** Source 2's `valid_to`
is intersected with the child's ACTIVE INTERVALS as derived from
`value_type='status'` observations on the same (tenant, source_type,
source_id, person_id) partition. An active interval starts at an
observation marking the child Active and ends at the next observation
marking the child Inactive/Terminated (or NULL if the child is still
active). Each Active->Inactive->Active cycle in the source data
produces an additional `org_chart` row for the same
(child, parent, source) — the SCD2 history honestly reflects the
re-activation rather than papering over the gap.

The rebuild CTE pipeline:

1. `state_log` — every `value_type='status'` observation tagged
   Active(1)/Inactive(0), with `LAG()` to collapse consecutive
   duplicates.
2. `state_transitions` — only rows where the state actually changed.
3. `active_intervals` — one row per Active transition; `interval_end`
   = the next transition's `created_at` (always Inactive because
   duplicates were collapsed) or NULL if still active.
4. `default_active` — synthetic [-inf, +inf) interval for children
   that have ZERO `value_type='status'` observations. Phase-1
   assumption: a connector emitting `parent_email` without emitting
   `status` is implicitly treating every employee as active.
5. `pe_periods` — `parent_email` observations with `LEAD()`-derived
   end times.
6. Final INSERT joins `pe_periods × active_intervals` per child on
   interval overlap; `valid_from = GREATEST(pe_from, ai_start)`,
   `valid_to` = LEAST of the two interval ends (treating NULL as
   +infinity). Disjoint intervals yield separate rows.

**Source dependency on `value_type='status'`.** For deactivation to
close edges, the source must emit `value_type='status'` with values
matching `Active`/`Inactive`/`Terminated` (case-sensitive plus the
lower-case spellings). Today only BambooHR (`bronze_bamboohr.employees.status`)
does. Other sources contributing edges in the future SHOULD emit
status; if they don't, their persons fall through to `default_active`
and edges never auto-close. The dbt model for a new source must
include `{'field': '<status_field>', 'value_type': 'status', ...}`
in its `identity_inputs_from_history` call to enrol in deactivation
handling.

**Multi-parent extensibility.** Phase 1 enforces single-parent per
(tenant, source_type, source_id, child) at the schema level. If a
future source emits multiple supervisors per employee in the same
source instance (matrix orgs, dotted-line + solid-line managers),
the schema can be promoted to multi-parent by one ALTER:
`ALTER TABLE org_chart DROP PRIMARY KEY, ADD PRIMARY KEY
(insight_tenant_id, insight_source_type, insight_source_id,
child_person_id, parent_person_id, valid_from)`. The read API
(`IPersonsReader.GetCurrentParentsAsync` / `GetCurrentChildrenAsync`)
already returns a list, so the contract does not change; only the
list length grows. No source today produces this, so the change is
not in Phase 1 scope.

**Cycle detection — two-hop, warn-not-fail.** After the rebuild
swap, the seeder counts pairs of CURRENT edges `(A -> B)` and
`(B -> A)` on the same `(tenant, source_type, source_id)` partition
via a self-join over `valid_to IS NULL` rows. A non-zero count
emits a `WARN` line in the seeder log but does NOT fail the
pipeline — a single bad row in HR data should not block the
remaining edges. Deeper cycles (`A -> B -> C -> A`) are not
detected here; the Phase-3 `/v1/subchart/{person_id}?depth=N`
endpoint will bound recursion by `depth` so unbounded walks
become structurally impossible regardless of source data quality.

Schema (`Migrations/003_org_chart.sql`):

```sql
CREATE TABLE org_chart (
    insight_tenant_id   BINARY(16) NOT NULL,
    insight_source_type VARCHAR(30) NOT NULL,
    insight_source_id   BINARY(16) NOT NULL,
    child_person_id     BINARY(16) NOT NULL,
    parent_person_id    BINARY(16) NOT NULL,
    author_person_id    BINARY(16) NOT NULL,
    reason              VARCHAR(50) NOT NULL,
    valid_from          TIMESTAMP(6) NOT NULL,
    valid_to            TIMESTAMP(6) NULL,
    PRIMARY KEY (insight_tenant_id, insight_source_type, insight_source_id,
                 child_person_id, valid_from),
    CONSTRAINT chk_no_self_loop CHECK (child_person_id <> parent_person_id),
    INDEX idx_current_parent   (insight_tenant_id, insight_source_type, insight_source_id,
                                child_person_id, valid_to),
    INDEX idx_current_children (insight_tenant_id, insight_source_type, insight_source_id,
                                parent_person_id, valid_to),
    INDEX idx_child_any_source  (insight_tenant_id, child_person_id, valid_to),
    INDEX idx_parent_any_source (insight_tenant_id, parent_person_id, valid_to),
    INDEX idx_valid_from (insight_tenant_id, valid_from)
);
```

Phase 1 invariant: at most one CURRENT parent per
`(tenant, source_type, source_id, child)`. This matches the BambooHR
/ Zoom / Slack reality (each source emits one parent per employee at
a time) and is enforced by the PK-and-`valid_to` combination. Phase
1.5 multi-parent (matrix orgs) would relax this by adding
`parent_person_id` to the PK; the rest of the schema and the read
API stay unchanged.

**BambooHR ordering in step 5.** BambooHR is processed before all
other sources when assigning `person_id` to new accounts. BambooHR
carries the canonical `supervisorEmail` field, so its accounts must
enter `persons` ahead of downstream connectors that share the same
email. The within-run email-automerge dict (`email_to_new_person`)
sees the BambooHR-minted `person_id` first, and Zoom/Slack/etc
accounts sharing the same email attach to it instead of minting
their own UUIDs. Alphabetical order already places `bamboohr` first
today, but making the rule explicit guards against future
source_type names that would sort earlier (e.g. an `airtable`
connector).

**No stub persons.** When a `parent_email` value does not match any
current email-bearer in the same tenant, the rebuild skips the row
silently and counts it in the post-rebuild log. We do not synthesise
a stub person carrying only the email observation. Rationale:

- Stubs would survive past the moment the real person enters
  `persons` (e.g. via a later BambooHR sync that backfills the
  missing employee), and we would need additional logic to merge
  the stub into the real person on the next rebuild — that mirrors
  the operator-review work ADR-0002 explicitly defers.
- The org-chart consumer can tolerate missing edges: the parent
  simply does not appear in the response until a future seed run
  finds the email-bearer.
- The diagnostic count surfaces ingestion gaps without polluting
  `persons` with synthetic rows that operators would have to clean
  up later.

**ADR-0002 not superseded.** The seeder's pending-iresolution branch
remains unchanged: new non-BambooHR accounts whose email already
exists in `persons` still get a fresh `person_id` tagged with
`reason='pending-iresolution'`. Identity fragmentation across
sources accumulates until the future operator-resolution flow lands;
that is acceptable for the org-chart workstream because BambooHR is
the org-chart anchor and BambooHR-first ordering ensures the parent
edges land against BambooHR-canonical `person_id`s rather than
pending-iresolution duplicates.

### Consequences

- On a first-run cluster the rebuild now produces a meaningful
  number of edges immediately — every BambooHR employee with a
  `supervisorEmail` that resolves to another BambooHR employee
  becomes a current edge in `org_chart`.
- Source 1 currently contributes zero rows but the path is wired;
  when the reconciliation service ships, edges it writes via
  `parent_person_id` automatically take precedence over Source 2 by
  the NOT EXISTS guard, without any rebuild code changes.
- Stubs are not synthesised. If a BambooHR employee's
  `supervisorEmail` points at someone outside BambooHR (e.g. an
  external advisor) and that email is never observed elsewhere, the
  edge will be skipped and surface in the warn-log line of the
  seeder. Operators see ingestion gaps rather than silent
  half-truths.
- The .NET service does not own the rebuild path; it only reads via
  `IPersonsReader.GetCurrentParentsAsync` and `GetCurrentChildrenAsync`,
  backed by `SqlOrgChart` SELECTs over the `idx_current_parent` /
  `idx_current_children` indexes.
- SCD2 history is preserved indefinitely. A future GC policy may
  trim closed edges older than retention `T`; not in Phase 1 scope.
- Drift between `persons` and `org_chart` is possible if a
  writer bypasses the rebuild path. Mitigation: a periodic Argo
  CronWorkflow can run the rebuild as an integrity check (out of
  Phase-1 scope; CronWorkflow definition is a follow-up).

### Confirmation

Confirmed by integration tests in
`Insight.Identity.Tests.Integration/OrgChartTests.cs`:

Reader correctness (six baseline tests):
- `GetCurrentParents_returns_one_edge_per_source_instance` — a person
  reporting in BambooHR and Zoom returns both edges.
- `GetCurrentParents_returns_empty_when_no_parents` — empty list
  rather than null when no edge exists.
- `GetCurrentParents_excludes_historical_edges` — SCD2 close+open of
  the same `(tenant, source, child)` partition returns only the
  open row.
- `GetCurrentParents_is_tenant_scoped` — identical UUIDs across two
  tenants do not leak.
- `GetCurrentChildren_returns_all_direct_reports_across_sources` —
  one parent with reports in two source instances surfaces both.
- `GetCurrentChildren_returns_empty_when_leaf` — a person with no
  reports returns empty.

SCD2 history with re-activation (three additional tests):
- `GetCurrentParents_returns_only_open_row_when_child_has_history` —
  child with one historical (T0,T2) row and one current (T3,NULL)
  row returns only the current one.
- `GetCurrentParents_returns_empty_when_child_only_has_historical_rows` —
  child deactivated without re-activation returns empty.
- `GetCurrentChildren_excludes_child_whose_latest_row_is_closed` —
  parent's subordinates list omits deactivated children.

The rebuild SQL itself is exercised end-to-end against the kind
cluster's MariaDB by re-running the seeder against the real
identity_inputs snapshot; the seeder's diagnostic block reports
parent_person_id / parent_email observation counts, current and
historical edge counts, the count of parent_emails skipped for
lack of an email-bearer, and the count of children that have only
historical edges (deactivated and not re-activated).

## Pros and Cons of the Options

### Materialized SCD2 edge table, two-source rebuild, no stubs (chosen)

- Good, because direct lookups are index-seeks: `(tenant, source,
  child, valid_to=NULL)` for parent, `(tenant, source, parent,
  valid_to=NULL)` for children.
- Good, because the email-resolution JOIN lives in the rebuild step
  rather than at read time — read paths stay fast even when the
  reconciliation service is still missing.
- Good, because Source 1 keeps the parent_person_id path live for
  the day the reconciliation service ships; no code change needed
  to activate it then.
- Good, because temporal as-of T queries are supported by the
  `idx_valid_from` index — no schema change needed in Phase 3+.
- Good, because the rebuild is deterministic and matches the
  `account_person_map` pattern operators already know.
- Bad, because the cache lags reality between rebuilds — acceptable
  for org-chart data that changes slowly (days/weeks).

### Single-source rebuild from parent_person_id only

- Good, because the rebuild SQL is simpler — no JOIN, no
  deduplication, no NOT EXISTS.
- Bad, because the reconciliation service does not exist in the
  codebase. With zero `parent_person_id` rows in `persons`, the
  cache is permanently empty until that service is built. No
  org-chart in the meantime.

### Synthesise stub persons for unresolved parent_emails

- Good, because every `parent_email` would yield an edge — no
  missing parents in the org-chart.
- Bad, because stubs persist past the moment the real person
  appears, and merging the stub into the real person on the next
  rebuild is the same problem ADR-0002 defers (operator-reviewed
  identity resolution). We would inherit the deferred work.
- Bad, because stubs pollute `persons` with rows that have
  exactly one observation and no real source provenance; analytics
  queries downstream would have to filter them out.

### Live recursive CTE against persons

- Good, because no second table to maintain — single source of
  truth.
- Bad, because every read does latest-per-partition arithmetic
  inside the recursion plus email resolution at every hop; query
  cost scales poorly past one or two hops.
- Bad, because the parent_email→email JOIN has to re-run for every
  request, making the Phase-3 subchart endpoint pay the resolution
  cost on every call.

### AFTER INSERT trigger maintaining edges in real time

- Good, because immediate consistency — no rebuild lag.
- Bad, because trigger logic lives in MariaDB SQL where it is
  harder to test, version, and observe than Python or C# code.
- Bad, because each writer (seed, future reconciliation service,
  operator flows) must respect trigger semantics or risk
  inconsistent edges; a single BULK INSERT with `INSERT IGNORE`
  could silently bypass trigger bodies on some MariaDB versions.
- Bad, because a drift-recovery story still requires a periodic
  rebuild anyway — the trigger only adds a second maintenance
  surface.

## More Information

- cyberfabric/cyber-insight#348 — parent issue (Phase 1, Phase 2,
  Phase 3 scope).
- ADR-0002 — Read From the MariaDB `persons` Table; pending-
  iresolution policy that this ADR explicitly does not change.
- ADR-0003 — latest-per-source-instance partition semantics on
  `persons` (the rebuild's source partitioning rule).
- ADR-0004 — lowercase email storage and lookup; the parent_email
  side of the resolution JOIN trims and lowercases for symmetry.
- ADR-0007 — `value_type` routing into `value_id` /
  `value_full_text` / `value` (the rebuild's SELECT shape).
- `seed-persons-from-identity-input.py` step 9 — the canonical
  rebuild SQL.

## Traceability

- [`cpt-insightspec-fr-identity-org-chart-table`](../PRD.md#materialised-parentchild-edge-cache)
- [`cpt-insightspec-fr-identity-org-chart-rebuild`](../PRD.md#rebuild-edges-from-persons-deterministically)
- [`cpt-insightspec-fr-identity-org-chart-read`](../PRD.md#read-current-parent-and-children-edges)
