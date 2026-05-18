# ADR-0004: Lowercase Emails on Storage and Lookup

**ID**: `cpt-insightspec-adr-0004-lowercase-email-lookup`



<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Lowercase on write and on lookup (chosen)](#lowercase-on-write-and-on-lookup-chosen)
  - [Switch collation to case-insensitive](#switch-collation-to-case-insensitive)
  - [Wrap lookup with LOWER on both sides](#wrap-lookup-with-lower-on-both-sides)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**Status:** Superseded by ADR-0011 (2026-05-18).

> **Superseded.** This ADR chose "lowercase on write + lowercase at
> the boundary" because the `persons.value_id` column was
> `utf8mb4_bin`. ADR-0011 swaps the column collation to
> `utf8mb4_unicode_ci`, which makes the comparison case-insensitive
> at the storage layer. Boundary-side lowercase and the former
> routing-lowercase FR are no longer a contract; the FR was removed
> alongside ADR-0011. This file is retained for historical context
> and the rejected-options analysis that informed ADR-0011's
> decision matrix (the "Switch collation" option below is exactly
> what ADR-0011 ended up adopting).

## Context and Problem Statement

`persons.value_id` is `VARCHAR(320) COLLATE utf8mb4_bin`. The `_bin`
collation makes byte equality the only equality, so
`Alice@Example.COM` and `alice@example.com` would not match. The
service needs a deterministic email-lookup behaviour that keeps the
`idx_value_id` covered index fast.

## Decision Drivers

- The hot-path index (`idx_value_id`) must stay covered — a SARG-able
  equality predicate is required.
- Operators expect emails to be case-insensitive in practice (even
  though RFC 5321 leaves the local part case-sensitive).
- The lookup behaviour must be deterministic across producers.

## Considered Options

- Switch the collation to `utf8mb4_general_ci` for case-insensitive
  matching.
- Wrap the lookup in `LOWER(value_id) = LOWER(@email)` —
  case-insensitive at query time but defeats the index.
- Lowercase on write and on lookup; preserve the original case in
  `display_name` (or in a future column) when needed.

## Decision Outcome

Lowercase on write and on lookup. The seed already lowercases via
`LOWER(TRIM())` when checking the existing-email set, and the service
applies `ToLowerInvariant()` to the lookup parameter before binding.

### Consequences

- Original casing is lost from `value_id` (it is preserved on
  `display_name` rows, which use `utf8mb4_unicode_ci`).
- The seed must lowercase before insert; that contract is enforced by
  ADR documentation, not by a CHECK constraint, so a future writer
  must follow the convention.
- A lint or test on the seed that asserts all email value_ids are
  lowercased on disk is a possible follow-up.

### Confirmation

Confirmed by `PersonLookupServiceTests.LowercaseEmail` (unit) and
`PersonsEndpointTests.MixedCaseEmail` (integration) — both seed mixed-
case observations and verify the lookup with a different case still
returns the assembled record.

## Pros and Cons of the Options

### Lowercase on write and on lookup (chosen)

- Good, because the `idx_value_id` covered index stays hot — single
  equality lookup on the same byte form.
- Good, because the seed and the reader share one transformation rule
  (`LOWER(TRIM(...))`).
- Good, because behaviour is deterministic — no implicit collation
  rules to reason about.
- Bad, because original casing is lost on `value_id`. Recoverable from
  `display_name` when needed.

### Switch collation to case-insensitive

- Good, because no writer-side discipline required.
- Bad, because case-insensitive collations (utf8mb4-general-ci, or the
  Unicode-correct utf8mb4-unicode-ci) have measurably slower equality
  on long strings and disable some plan optimisations.
- Bad, because utf8mb4-general-ci is not Unicode-correct for
  non-ASCII case mapping; utf8mb4-unicode-ci is the modern choice but
  is even slower.

### Wrap lookup with LOWER on both sides

- Good, because no writer-side discipline required.
- Bad, because the predicate is non-SARG-able — every row in the
  partition is scanned. With 50k persons this becomes the bottleneck
  for NFR-latency.

## More Information

- RFC 5321 §2.4 — local-part case sensitivity caveat (in practice
  case-insensitive).
- MariaDB collation reference — `utf8mb4_bin` is byte equality.

## Traceability

- [`cpt-insightspec-fr-identity-lookup-resolve-by-email`](../PRD.md#resolve-email-to-person_id)
- [`cpt-insightspec-nfr-identity-latency`](../PRD.md#p95-lookup-latency)
- [`cpt-insightspec-adr-0011-persons-relax-uniqueness-and-collation`](0011-persons-relax-uniqueness-and-collation.md) — supersedes this ADR.
