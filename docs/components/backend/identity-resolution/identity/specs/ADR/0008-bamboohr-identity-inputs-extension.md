# ADR-0008: Extend BambooHR `identity_inputs` With Person Attributes

**ID**: `cpt-insightspec-adr-0008-bamboohr-identity-inputs-extension`



<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Extend the dbt model to emit eleven fields (chosen)](#extend-the-dbt-model-to-emit-eleven-fields-chosen)
  - [Emit only the originally-modelled three fields](#emit-only-the-originally-modelled-three-fields)
  - [Post-seed enrichment step](#post-seed-enrichment-step)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**Status:** Accepted

## Context and Problem Statement

`bamboohr__identity_inputs.sql` initially emits three fields:
`workEmail` (email), `employeeNumber` (employee_id), and
`displayName` (display_name). The C# service projects every BambooHR
person attribute onto the `Person` response (first/last name,
department, division, job title, status, parent email, parent id),
so the dbt model must emit them too — otherwise the response shape
stays empty for everything beyond email + display_name.

## Decision Drivers

- The Phase 1 response shape must be populated end-to-end against
  BambooHR data; partial responses are not acceptable.
- The org-tree (Phase 2) needs `parent_email` and `parent_id`
  observations available now so the reconciliation service can
  resolve them to `parent_person_id` later.
- The seed pipeline's routing constants and the dbt model must stay
  in lockstep with ADR-0007.

## Considered Options

- Emit only the three originally-modelled fields and surface empty
  attributes from the service.
- Extend the dbt model to emit eleven fields (chosen).
- Push attribute hydration out of the dbt model into a post-seed
  enrichment step.

## Decision Outcome

Extend the model to emit eleven fields:

- Profile attributes: `firstName`, `lastName`, `department`,
  `division`, `jobTitle`, `status` → `value_full_text`.
- Org-chart pointers: `supervisorEmail` → `parent_email`,
  `supervisorEId` → `parent_id` → `value_id`.

`parent_person_id` is intentionally **not** emitted by the dbt model
— it is written by the reconciliation service (separate PR) once it
resolves `parent_email` / `parent_id` to a stable Insight
`person_id`.

### Consequences

- The seed pipeline's column-routing constants must include the new
  `value_type`s (handled in
  `seed-persons-from-identity-input.py`).
- BambooHR sync time grows slightly (more rows per employee), but
  the observation log is append-only and steady-state increases are
  bounded by attribute change frequency, not by employee count.
- Backfill on existing clusters requires
  `dbt run --full-refresh --select bamboohr__identity_inputs+`
  followed by re-running the seed.
- The dbt model is now connector-coupled to the service's response
  shape; future shape changes require coordinated PRs.

### Confirmation

Confirmed by the integration test that seeds an Alice row across
multiple value_types and asserts every response field is populated.
The seed pipeline's unit tests pin the routing constants against the
ADR-0007 table.

## Pros and Cons of the Options

### Extend the dbt model to emit eleven fields (chosen)

- Good, because every BambooHR-served person gets a fully-populated
  response from day one.
- Good, because the org-chart pointers (`parent_email`,
  `parent_id`) flow through the same observation pipeline as
  primary identifiers.
- Good, because no separate enrichment process is required — one
  Bronze → Silver path, one source of truth.
- Bad, because the dbt model now mirrors the service's response
  shape — a coupling that needs documentation discipline.

### Emit only the originally-modelled three fields

- Good, simpler dbt model.
- Bad, because most response fields stay empty in production —
  defeats the purpose of the lookup.

### Post-seed enrichment step

- Good, because dbt stays minimal and the enrichment lives where
  the C# code already does.
- Bad, because adding a second writer to `persons` complicates the
  ownership story (see ADR-0006 service-owned migrations).
- Bad, because enrichment failures become a separate operational
  surface vs the seed.

## More Information

- `src/ingestion/connectors/hr-directory/bamboohr/dbt/bamboohr__identity_inputs.sql`
  is the canonical model.
- `src/backend/services/identity-resolution/seed/seed-persons-from-identity-input.py`
  carries the matching routing constants.

## Traceability

- [`cpt-insightspec-fr-identity-lookup-hydrate`](../PRD.md#hydrate-person-attributes)
- [`cpt-insightspec-fr-identity-lookup-parent`](../PRD.md#surface-parent-attributes-when-present)
- [`cpt-insightspec-actor-seed-pipeline`](../PRD.md#seed-pipeline)
