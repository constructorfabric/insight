# ADR-0007: `value_type` Routing for Identity Reads

**ID**: `cpt-insightspec-adr-0007-value-type-routing`



<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Three-column routing by `value_type` family (chosen)](#three-column-routing-by-valuetype-family-chosen)
  - [Single `value` column](#single-value-column)
  - [Per-`value_type` table](#per-valuetype-table)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**Status:** Accepted

## Context and Problem Statement

The `persons` schema splits the value across three columns by
`value_type`:

- `value_id VARCHAR(320) COLLATE utf8mb4_bin` — strict byte
  comparison, hot-path index target.
- `value_full_text VARCHAR(512) COLLATE utf8mb4_unicode_ci` —
  case-insensitive search, room for FULLTEXT.
- `value TEXT` — catch-all, indexed only via `value_hash`.

The serving path must agree with the seed pipeline on which `value_type`
lands in which column, otherwise lookups will miss rows the seed
wrote.

## Decision Drivers

- Identifier-shaped `value_type`s need exact byte equality on a hot
  index for sub-millisecond lookup.
- Free-form attributes (display name, department) need
  case-insensitive search; FULLTEXT may become useful later.
- The seed writer and the service reader must share one routing table
  to avoid silent miss-routes.

## Considered Options

- Single `value` column with composite indexes per `value_type` —
  one routing, one column.
- Three-column routing by `value_type` family (chosen).
- Per-`value_type` table (one table per attribute kind).

## Decision Outcome

The shared routing table:

| Column | `value_type`s |
|---|---|
| `value_id` | `id`, `email`, `username`, `employee_id`, `parent_email`, `parent_id`, `parent_person_id` |
| `value_full_text` | `display_name`, `first_name`, `last_name`, `department`, `division`, `job_title`, `status` |
| `value` (catch-all) | anything else (custom attributes, future types) |

The service reads `value_effective` (the generated coalesce of the
three columns), so it does not need to know the routing for read; the
routing matters only for writes (the seed) and for lookups that
filter by `value_id` (the email resolution path).

### Consequences

- The seed pipeline's `VALUE_TYPES_FOR_VALUE_ID` and
  `VALUE_TYPES_FOR_VALUE_FULL_TEXT` constants are kept in lockstep
  with this table. A future change to the table requires touching
  both the Python seeder and this ADR.
- `parent_person_id` is stored as the canonical 36-char string form
  so the email-lookup-by-`value_id` SQL works for it without special
  casing. `BINARY(16)` would have required a different lookup path.
- `employee_id` migrated from the catch-all into `value_id` because
  it's an identifier; the seed lowercases nothing for it (it's
  numeric or alphanumeric).

### Confirmation

Confirmed by integration tests that seed each canonical `value_type`
and assert the assembled response surfaces them at the right field.
The seed-side routing is locked behind unit tests over
`route_value_type_to_column` in
the persons-seed.

## Pros and Cons of the Options

### Three-column routing by `value_type` family (chosen)

- Good, because identifier-shaped lookups stay on a covered byte
  index.
- Good, because free-form attributes get an appropriate collation
  without compromising the identifier path.
- Good, because `value_effective` lets readers stay routing-blind.
- Bad, because writers must respect the routing — drift between
  seed and reader silently misses rows.

### Single `value` column

- Good, simpler schema.
- Bad, because there is no way to give identifier lookups
  `utf8mb4_bin` while keeping display-name search case-insensitive
  on the same column.
- Bad, because catch-all `TEXT` cannot be fully indexed without a
  prefix limit that creates collision risk for long values.

### Per-`value_type` table

- Good, because each attribute family gets a purpose-built schema.
- Bad, because adding a `value_type` means adding a table — schema
  churn.
- Bad, because cross-attribute queries (assembler) require unions
  across many tables.

## More Information

- `docs/domain/identity-resolution/specs/DESIGN.md` §"Table:
  persons" carries the canonical column reference.

## Traceability

- [`cpt-insightspec-fr-identity-lookup-resolve-by-email`](../PRD.md#resolve-email-to-person_id)
- [`cpt-insightspec-fr-identity-lookup-hydrate`](../PRD.md#hydrate-person-attributes)
- [`cpt-insightspec-fr-identity-lookup-parent`](../PRD.md#surface-parent-attributes-when-present)
- [`cpt-insightspec-actor-seed-pipeline`](../PRD.md#seed-pipeline)
