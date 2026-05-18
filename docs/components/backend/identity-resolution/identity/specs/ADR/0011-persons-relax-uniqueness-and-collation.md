# ADR-0011: Persons Schema — Relax UNIQUE and Switch `value_id` Collation

**ID**: `cpt-insightspec-adr-0011-persons-relax-uniqueness-and-collation`

**Status:** Accepted

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Combined fix in one migration (chosen)](#combined-fix-in-one-migration-chosen)
  - [Drop UNIQUE entirely, no replacement](#drop-unique-entirely-no-replacement)
  - [Keep value_hash UNIQUE, add LOWER index for case-insensitive lookup](#keep-valuehash-unique-add-lower-index-for-case-insensitive-lookup)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

Two latent defects in the `persons` schema surfaced while implementing
the `person_parent_map` cache (cyberfabric/cyber-insight#348 Phase 1
in PR #477).

**Defect A — UNIQUE on `value_hash` silently drops state transitions.**
The original schema declared:

```sql
UNIQUE KEY uq_person_observation (
    insight_tenant_id, person_id, insight_source_type, insight_source_id,
    value_type, value_hash
)
```

where `value_hash = SHA2(value_effective)`. The intent was idempotency
on seeder re-runs ("same observation written twice → one row"). But
the same hash is produced for the same VALUE — regardless of WHEN it
was observed. So a `status` field transitioning Active → Inactive →
Active emits three legitimate rows in `identity_inputs` (dbt
`identity_inputs_from_history` macro), but the second `'Active'`
collides with the first on `value_hash` and is silently dropped by
the seeder's `INSERT IGNORE`. The `persons` table — which is
semantically an append-only observation log — therefore cannot record
return-to-prior-value events. Every value_type that may revisit a
previously observed value is affected.

**Defect B — `value_id` is case-sensitive (`utf8mb4_bin`).** Every
value_type routed into `value_id` (per ADR-0007: `id`, `email`,
`username`, `employee_id`, `parent_email`, `parent_id`,
`parent_person_id`) is compared byte-for-byte. A lookup for
`roman.mitasov@constructor.tech` does not match a stored
`Roman.Mitasov@constructor.tech` even though both refer to the same
person. The service partially mitigated this for the
`GET /v1/persons/{email}` happy path by calling
`email.Trim().ToLowerInvariant()` before the repository call
(ADR-0004), but the mitigation does not extend to:

- raw SQL callers (rebuild SQL in the seeder, future reconciliation
  service),
- other value_types stored in `value_id` (UUID strings, source
  account ids),
- inserts that lowercase the input but not the stored value.

The production lookup `https://insight-dev.constr.dev/api/identity/v1/persons/Roman.Mitasov%40constructor.tech`
returns 404 specifically because of this collation choice.

Both defects are corrections of original-design mistakes rather than
new features. Phase 1 of #348 surfaced them; this ADR records the
fix.

## Decision Drivers

- The persons table is semantically an append-only observation log.
  The schema must allow recording every observation event, including
  return-to-prior-value transitions.
- Seeder re-runs must remain idempotent — a replay of the same source
  data must not multiply rows.
- Email and UUID-string comparisons are conventionally case-insensitive
  across the cyberfabric platform. Forcing byte-exact comparison in
  one storage layer creates a fragile contract no caller expects.
- Existing indexes on `value_id` are covered (`idx_value_id`); any
  case-insensitive solution must not break the covered-index property.
- The fix must be applicable to existing dev and prod clusters via a
  DbUp migration — no manual operator step.

## Considered Options

- **Combined fix in one migration**: drop the value-hash-based UNIQUE
  and replace with a (..., created_at)-based UNIQUE; switch the
  `value_id` column collation from `utf8mb4_bin` to
  `utf8mb4_unicode_ci`. One ALTER block, one PR, one issue.
- **Drop UNIQUE entirely without replacement.** Simpler migration but
  loses re-run idempotency; the seeder would need explicit dedup
  logic.
- **Keep value_hash UNIQUE, add a functional `LOWER(value_id)` index
  for case-insensitive lookup.** Preserves the (broken) UNIQUE, adds
  index and changes SQL to `LOWER(value_id) = LOWER(@x)`. Does not
  fix Defect A.

## Decision Outcome

Adopt the combined fix in a single migration
`Migrations/004_persons_relax_constraints.sql`:

```sql
ALTER TABLE persons DROP INDEX uq_person_observation;

ALTER TABLE persons ADD UNIQUE KEY uq_person_observation (
    insight_tenant_id, person_id, insight_source_type, insight_source_id,
    value_type, created_at
);

ALTER TABLE persons MODIFY COLUMN value_id
    VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL;
```

The new UNIQUE uses `created_at` (a `TIMESTAMP(6)`, microsecond
precision) as the natural disambiguator: re-emission of the same
observation at the same `created_at` collapses via `INSERT IGNORE`,
distinct observations at distinct timestamps both persist.

The collation switch makes every `value_id` comparison
case-insensitive at the storage layer. The SQL `WHERE value_id = @x`
predicate becomes case-insensitive automatically; no application
code change is required. `value_full_text` is already
`utf8mb4_unicode_ci` (no change); `value` (TEXT) inherits the table
default `utf8mb4_unicode_ci` (no change); `value_hash` stays
`ascii_bin` because it is a SHA-256 hex digest and must remain
byte-compared. The hashes themselves are not recomputed by this
migration — `value_effective` is unchanged byte-for-byte, only the
comparison semantics change.

### Consequences

- **Phase 1.5 person_parent_map**: the two `[Fact(Skip = ...)]` tests
  in `Insight.Identity.Tests.Integration/ActiveIntervalsTests.cs`
  (added in PR #477) can drop the `Skip` attribute. The SCD2
  active-intervals CTE already handles re-activation correctly; the
  only blocker was Defect A. A follow-up commit on the
  person_parent_map branch unblocks them after this PR lands.
- **Production lookup fix**: `GET /v1/persons/Roman.Mitasov@...` and
  similar mixed-case requests start returning 200 against existing
  data — no re-seed required. Existing mixed-case stored values
  remain bit-exact in storage but become equal under the
  case-insensitive comparison.
- **Seeder idempotency preserved**: re-running the seeder against
  the same `identity_inputs` snapshot produces the same persons
  rows. The new UNIQUE collapses duplicate writes by `created_at`
  (which is set from `_synced_at` in the source data, stable across
  re-runs).
- **Behaviour change for non-email value_types in `value_id`**:
  comparisons for `id` / `username` / `employee_id` / `parent_id`
  also become case-insensitive. Source-native ids that intentionally
  differ only by case (rare to non-existent in practice — BambooHR
  employee ids are numeric, Zoom and Slack ids are case-insensitive
  by source contract) would now compare equal. We treat this as
  desired behaviour; surface as a known consequence rather than a
  regression.
- **ALTER COLUMN cost**: the value_id collation switch rewrites the
  column and rebuilds `idx_value_id`. On dev cluster (~12k rows)
  this is sub-second; on a production-sized table it is bounded by
  InnoDB rebuild time (minutes). The migration runs at service
  startup before traffic accepts, so user-visible impact is one
  startup window.
- **No data loss**: the migration is structural — no rows are
  deleted, no values rewritten.

### Confirmation

Confirmed by integration tests in
`Insight.Identity.Tests.Integration/PersonsSchemaTests.cs`:

- `Persons_allows_state_transition_with_same_value_at_different_created_at`
  — INSERT Active(T0), Inactive(T2), Active(T3) all succeed; persons
  has three rows. The old schema rejected the second Active.
- `Persons_value_id_comparison_is_case_insensitive` — store
  `'Roman.Mitasov@example.com'`, query `WHERE value_id = 'roman.mitasov@example.com'`,
  the stored row is returned.
- `Persons_insert_ignore_dedupes_on_same_created_at` — two INSERTs
  with identical `(tenant, person, source_type, source_id, value_type,
  created_at)` and INSERT IGNORE produce one row.
- `Persons_schema_no_unique_on_value_hash` — `information_schema.STATISTICS`
  introspection confirms the old `uq_person_observation` definition
  is gone and the new one is in place.

## Pros and Cons of the Options

### Combined fix in one migration (chosen)

- Good, because both defects are root-cause schema issues with a
  single migration footprint.
- Good, because the new UNIQUE preserves re-run idempotency without
  any application-level change.
- Good, because the collation switch fixes case-sensitivity for all
  value_types in `value_id` uniformly — no per-value-type carve-out.
- Good, because covered-index reads stay covered (no functional
  `LOWER()` index needed).
- Bad, because the ALTER COLUMN rewrites the table; one bounded
  startup-window cost in production.

### Drop UNIQUE entirely, no replacement

- Good, because the migration is minimal (one DROP INDEX).
- Bad, because re-running the seeder would duplicate every row each
  pass — the table would grow linearly with seed runs. Operators
  would need a wipe-then-reseed protocol or in-code dedup logic in
  the seeder.

### Keep value_hash UNIQUE, add LOWER index for case-insensitive lookup

- Good, because nothing about the storage changes; rollback is
  trivial.
- Bad, because Defect A (cannot record state transitions) is not
  fixed.
- Bad, because every read SQL must be updated to `LOWER(value_id) =
  LOWER(@x)`; future SQL authors must remember this contract.
- Bad, because the SHA-256 of two case-different values is different,
  so the original UNIQUE would still treat `'Roman.Mitasov@...'` and
  `'roman.mitasov@...'` as two distinct rows in persons, splitting
  one logical observation into two stored observations.

## More Information

- cyberfabric/cyber-insight#477 — the person_parent_map PR where both
  defects surfaced.
- ADR-0002 — Read From the MariaDB `persons` Table; the original
  schema decision this ADR amends.
- ADR-0004 — Lowercase Emails on Storage and Lookup; the
  application-level half of the original case-insensitivity
  mitigation, now superseded by the storage-level fix.
- ADR-0007 — `value_type` Routing; the routing rules that decide
  which value_types live in `value_id`.

## Traceability

- [`cpt-insightspec-fr-identity-schema-relax-uniqueness`](../PRD.md#schema-allows-recording-state-transitions)
- [`cpt-insightspec-fr-identity-schema-case-insensitive-value-id`](../PRD.md#value-comparisons-are-case-insensitive)
