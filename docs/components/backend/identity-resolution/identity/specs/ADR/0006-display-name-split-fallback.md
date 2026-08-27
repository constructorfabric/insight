# ADR-0006: Display-Name Split Fallback for First/Last Name

**ID**: `cpt-insightspec-adr-0006-display-name-split-fallback`



<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Best-effort split inside the service (chosen)](#best-effort-split-inside-the-service-chosen)
  - [No fallback](#no-fallback)
  - [Connector-specific splitters](#connector-specific-splitters)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**Status:** Accepted

## Context and Problem Statement

BambooHR observations carry `displayName`, `firstName`, and `lastName`
as separate fields. Other connectors (Cursor, Claude Admin) only emit
`display_name`. The response schema needs `first_name` / `last_name`
populated for downstream callers regardless of which connector is the
source of truth.

## Decision Drivers

- Response shape must stay complete across connectors so callers don't
  have to special-case per source.
- The seed cannot back-fill missing fields — Cursor and Claude Admin
  don't provide them.
- The split must be deterministic and unit-testable so behaviour
  doesn't drift silently with library updates.

## Considered Options

- No fallback — return empty `first_name` / `last_name` when missing.
- Best-effort split inside the service (chosen).
- Connector-specific splitters resolved by `insight_source_type`.

## Decision Outcome

When the assembler finds no `first_name` or `last_name` observation,
it falls back to `DisplayNameSplitter.Split(displayName)`:

1. `"Last, First"` (comma-separated) → `(First, Last)`.
2. `"First Rest"` (space-separated) → `(First, Rest)` where `Rest`
   keeps any middle names.
3. Single token → `(token, "")`.
4. Empty / whitespace → `("", "")`.

### Consequences

- Names with multiple commas or unusual punctuation may split
  incorrectly. The split is unit-tested for the canonical formats
  but not for every edge case.
- A future PR may wire connector-specific splitters; the current
  shape is good enough for Phase 1.
- Callers needing authoritative first/last must read BambooHR
  observations directly via the API surface (Phase 2 might expose a
  source-priority knob).

### Confirmation

Confirmed by `DisplayNameSplitterTests` — covers all four cases with
the canonical inputs and an empty-string edge case. Integration test
`PersonsEndpointTests.NameFallbackFromDisplayName` seeds a person
with only `display_name` and asserts the response carries split
first/last names.

## Pros and Cons of the Options

### Best-effort split inside the service (chosen)

- Good, because the response shape stays complete across connectors.
- Good, because the rule is one place (`DisplayNameSplitter`) and is
  unit-tested.
- Good, because the heuristic covers BambooHR and Cursor shapes
  natively.
- Bad, because cultural names with commas, hyphens, or multiple
  surnames may split incorrectly.

### No fallback

- Good, because the response always reflects what was observed.
- Bad, because callers see empty fields for Cursor / Claude Admin
  records and have to implement their own splitter.

### Connector-specific splitters

- Good, because each shape is handled by a specialised rule.
- Bad, because adding a connector means adding a splitter — coupling
  the assembler to the source taxonomy.
- Bad, because most connectors use the same two shapes; the
  per-connector indirection adds complexity for little gain.

## More Information

- The display-name splitter carries
  the implementation; Anton's review comment on PR #398 prompted
  refactor of the helper into a generic `SplitAt`.

## Traceability

- [`cpt-insightspec-fr-identity-routing-name-split`](../PRD.md#display-name-split-fallback)
- [`cpt-insightspec-component-identity-domain`](../DESIGN.md#insightidentitydomain)
