---
status: accepted
date: 2026-04-06
---

# ADR-0001: Shared `unmapped` Table for Identity and Person Domains

**ID**: `cpt-ir-adr-shared-unmapped`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A: Single shared `unmapped` table](#option-a-single-shared-unmapped-table)
  - [Option B: Separate `person_unmapped` table in person domain](#option-b-separate-personunmapped-table-in-person-domain)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

Both the identity-resolution domain and the person domain need to track unresolvable observations from `identity_inputs`. Identity-resolution tracks alias-level unmapped records (e.g., an email that cannot be linked to a person). The person domain tracks person-attribute-level unmapped records (e.g., a `display_name` change for a `source_account_id` that cannot be matched to an existing person). Should these be stored in one table or two?

## Decision Drivers

* Both record types originate from the same source: the shared `identity_inputs` table filled by connectors
* Both record types have identical structure: `insight_tenant_id`, `insight_source_id`, `insight_source_type`, `source_account_id`, `value_type`, `value`, plus resolution workflow fields
* Differentiation between identity-level and person-attribute-level records is already possible via `value_type` values: identity types (`id`, `email`, `username`, `employee_id`) vs person-attribute types (`display_name`, `role`, `location`, etc.). `id` is the canonical account-binding observation per ADR-0002 (replaces the former `platform_id` for connectors where the platform identifier equals `source_account_id`).
* Operators reviewing the unmapped queue benefit from a single view across both domains

## Considered Options

* Option A: Single shared `unmapped` table (owned by identity-resolution domain, used by person domain)
* Option B: Separate `person_unmapped` table in person domain with identical schema

## Decision Outcome

Chosen option: **"Option A: Single shared `unmapped` table"**, because the data has identical structure, common origin (`identity_inputs`), and natural differentiation by `value_type` values. Adding a second table with the same schema creates maintenance burden without providing meaningful separation.

### Consequences

* Good, because operators see all unmapped observations in one place — no need to check two queues
* Good, because schema changes (e.g., adding a column) only need to happen once
* Good, because resolution workflows (e.g., linking an unmapped record to a person) use a single code path
* Bad, because the person domain writes to a table owned by the identity-resolution domain, creating a cross-domain write dependency
* Bad, because queries for person-domain-only unmapped records require filtering by `value_type`

### Confirmation

* Verify that the `unmapped` table schema in identity-resolution DESIGN.md includes `source_account_id` and documents both alias-level and person-attribute-level usage
* Verify that person domain DESIGN.md references the shared table and documents the `value_type` filter convention
* Verify that person domain dependency rules document the cross-domain write to `unmapped`

## Pros and Cons of the Options

### Option A: Single shared `unmapped` table

The identity-resolution domain owns the `unmapped` table. The person domain writes person-attribute unmapped records to the same table. Records are differentiated by `value_type` values.

* Good, because no schema duplication — one table, one set of indexes, one set of queries
* Good, because common origin from `identity_inputs` makes unified storage natural
* Good, because operator tooling has a single unmapped queue to process
* Neutral, because cross-domain write is documented and constrained to INSERT only
* Bad, because domain boundary is blurred — person domain depends on IR domain's table schema

### Option B: Separate `person_unmapped` table in person domain

The person domain owns its own `person_unmapped` table with the same columns as the IR `unmapped` table.

* Good, because clean domain boundary — each domain owns all its tables
* Good, because schema can evolve independently per domain
* Bad, because identical schema duplicated across two tables
* Bad, because operators must check two queues or build a UNION view
* Bad, because resolution logic must be duplicated or abstracted

## More Information

The `value_type` vocabulary provides a natural partition:
- **Identity types** (IR domain): `id`, `email`, `username`, `employee_id`
- **Person-attribute types** (person domain): `display_name`, `role`, `location`, and future attribute types

If the two domains' unmapped records diverge significantly in structure in the future (e.g., person domain needs fields that IR does not), this decision can be revisited. The migration path would be: create `person_unmapped`, backfill from `unmapped WHERE value_type IN (person-attribute types)`, update person domain writes.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses:

* `cpt-ir-fr-unmapped-queue` — unmapped alias observations are stored in the shared `unmapped` table
* `cpt-person-fr-conflict-detection` — person-attribute observations that cannot be resolved are routed to the same `unmapped` table, differentiated by `value_type`
* `cpt-ir-adr-shared-unmapped` — this ADR documents the shared table decision
