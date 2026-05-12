---
status: accepted
date: 2026-04-23
---

# Project-scoped custom-field ingestion via per-project substream


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option 1 — Per-project substream](#option-1--per-project-substream)
  - [Option 2 — Single global registry call](#option-2--single-global-registry-call)
  - [Option 3 — Hybrid](#option-3--hybrid)
  - [Option 4 — Defer to Silver](#option-4--defer-to-silver)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-youtrack-project-scoped-fields`
## Context and Problem Statement

YouTrack's custom-field data model is project-scoped — each project defines its own set of custom fields, with project-specific IDs and bundle values. The same logical field (e.g. "Severity") may have different REST IDs, different value bundles, and different cardinality flags in different projects. There is no single global registry that returns all custom fields with their bundle values resolved.

Jira, by contrast, exposes a global `/rest/api/3/field` endpoint that returns one flat list across the entire instance. The Jira connector consumes that endpoint and produces a `jira_fields` table; downstream Silver staging joins on global field IDs.

For YouTrack, we have to decide how to discover the per-project field registry needed to populate `class_task_field_metadata` (future scope §2.5) — and the answer drives the Bronze stream shape for this PR.

## Decision Drivers

- Correctness — IDs and bundle values must be project-scoped, not collapsed across projects.
- Sync cost — additional API requests per project add proportional latency to the directory phase.
- Symmetry with Jira where possible (downstream Silver layer prefers source-agnostic shapes).
- Builder-UI compatibility — substream patterns are well-supported by the declarative manifest.
- No-whitelist scope (Connector ADR-003) — the connector ingests every project the token reaches.

## Considered Options

1. **Per-project substream** — `youtrack_project_custom_fields` is a substream of `youtrack_projects`, fanning out one `/api/admin/projects/{id}/customFields` request per project.
2. **Single global registry call** — `/api/customFieldSettings/customFields` once per sync, returning all instance-wide custom-field definitions.
3. **Hybrid** — fetch global definitions for shape, then per-project for ID-to-bundle mapping.
4. **Defer to Silver** — emit only the embedded `custom_fields_json` from `youtrack_issue.customFields[]` and reconstruct the registry from observed values.

## Decision Outcome

Chosen option: **per-project substream** (Option 1), because it is the only option that preserves project-scoped IDs and bundle values without post-hoc disambiguation. The Bronze table `youtrack_project_custom_fields` is keyed on `(project_id, youtrack_id)` where `youtrack_id` is the project-scoped field ID, and carries the inlined `bundle.values[]` array for direct consumption by future Silver staging.

### Consequences

**Positive**:

- Correct project-scoped IDs and bundle values, ready for direct Silver consumption without disambiguation logic.
- `incremental_dependency` does not apply (projects are full-refresh), but project count is bounded — fan-out cost is O(projects), typically ≤ 200.
- Pattern matches the YouTrack data model rather than fighting it.

**Negative**:

- Extra `(num_projects)` API requests per sync. Mitigated by `Retry-After` honouring (`cpt-insightspec-fr-youtrack-bronze-retry-policy`) and the daily-batch cadence.
- A 403 on one project's custom-fields endpoint must be soft-fail (drop partition, log warning). The error-handler (`cpt-insightspec-fr-youtrack-bronze-retry-policy`) accommodates this.

**Rejected — Option 2 (global registry)**: `/api/customFieldSettings/customFields` returns instance-wide field definitions but **not** the project-specific IDs that issue rows carry in `customFields[].id`. Joining issues to a global registry would lose project scoping.

**Rejected — Option 3 (hybrid)**: Doubles the API surface while still requiring per-project disambiguation — strictly worse than Option 1.

**Rejected — Option 4 (defer to Silver)**: Pushes the discovery problem downstream but does not solve it — Silver still needs the registry. Worse, reconstructing from observed values means archived-but-still-referenced fields would be invisible until an issue mentions them.

### Confirmation

Decision is confirmed when:

- `youtrack_project_custom_fields` substream is declared in `connector.yaml` with parent `youtrack_projects` and the documented `fields` query parameter (including `bundle(values(...))`).
- `source.sh read task-tracking/youtrack youtrack_project_custom_fields <tenant>` succeeds against a live tenant and the emitted rows carry `project_id` plus bundle values.
- The PR description's verification table shows non-zero record counts for `youtrack_project_custom_fields` (verified — 3 330 rows in PR #227).

## Pros and Cons of the Options

### Option 1 — Per-project substream

- **Pros**: Correct IDs and bundle values, no post-hoc disambiguation, matches YouTrack's native data model, supports `incremental_dependency` if directories ever move to incremental.
- **Cons**: `O(num_projects)` API requests per sync. Mitigated by 24-hour cadence and `Retry-After` honouring.

### Option 2 — Single global registry call

- **Pros**: One API request per sync.
- **Cons**: Loses project-scoped IDs; issue rows reference field IDs that the global registry cannot disambiguate. Effectively wrong shape.

### Option 3 — Hybrid

- **Pros**: None unique.
- **Cons**: All of Option 2's costs plus Option 1's fan-out — strictly worse.

### Option 4 — Defer to Silver

- **Pros**: Connector simpler.
- **Cons**: Pushes the problem to Silver without solving it; archived-but-referenced fields invisible.

## More Information

- YouTrack REST API reference for `/api/admin/projects/{id}/customFields`: <https://www.jetbrains.com/help/youtrack/devportal/api-howto-get-projects-with-custom-fields.html>.
- Phase 1 research note (Insight platform engineering log) — verified bundle-value inlining for the default field types (Enum, OwnedField, Version).

## Traceability

- Implements PRD `cpt-insightspec-fr-youtrack-stream-project-custom-fields`, `cpt-insightspec-fr-youtrack-custom-field-bundles`.
- Implements DESIGN `cpt-insightspec-constraint-youtrack-project-scoped-fields`, `cpt-insightspec-principle-youtrack-project-scoped-registry`.
- Pairs with Connector ADR-003 (no-whitelist scope) — every project the token sees gets a custom-field fan-out.
