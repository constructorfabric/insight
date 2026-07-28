# Metrics Domain

The unified metrics system: metrics are defined once in a typed registry,
computed by one generic runtime over normalized source measure observations,
and served self-describing through `POST /v1/metric-results`. All new metrics
are authored through this system.

## Concepts

Three layers, one handshake in the middle.

An **observation** is a recorded fact: "alice, June 3rd, Claude Code,
`accepted_edit_actions` = 17". It says what happened, to whom, when, and how
much — and deliberately nothing about what it means. No name a user would
see, no formula, no chart. The data side (dbt gold models over silver
classes) produces millions of these in one fixed row shape, and its only job
is to record them honestly.

A **definition** is a catalog card holding meaning: "there is a metric
`ai.tool_acceptance_rate`; compute it as `accepted_edit_actions` divided by
`tool_use_offered`, times 100; show as percent; higher is better; may be
split by tool; compare within org unit". No data lives here — only meaning
and instructions, stored in the registry and authored as one Rust struct per
metric.

A **metric result** is the computed answer the user sees. It is not stored
anywhere: at request time the runtime applies a definition to the matching
observations — "alice, January: 312 ÷ 405 = 77%" — and returns it labeled and
ready to render.

Metric **evidence** is the source-level population behind that answer. Each
managed source exposes a normalized serving table.
Definitions inherit drilldown support when all compatible inputs are backed by
the same validated evidence relation, so adding a metric over an existing
measure requires no drilldown-specific configuration.
The backend applies the same entity, period, dimension, input-role, and
computation semantics used by metric results.

The split exists because one fact serves many meanings and one meaning serves
many questions. The same `accepted_edit_actions` observation is the whole
value of `ai.accepted_edit_actions` and the numerator of
`ai.tool_acceptance_rate` — recorded once, interpreted twice. The same
definition answers any period, person, team, dimension split, or peer
comparison without new code. Each side changes without touching the other:
renaming a metric edits a card; a vendor API change fixes fact recording
while every card keeps working.

The handshake is the source measure observation contract (see
[`specs/DESIGN.md`](specs/DESIGN.md)): the data side promises to emit facts
in that shape, definitions reference facts only by measure key, and the
runtime can therefore connect any definition to any matching facts without
either side knowing the other exists.

| | Observation | Evidence | Definition | Metric result |
|---|---|---|---|---|
| What | an aggregate-ready fact | the participating source population | the meaning of facts | the computed answer |
| Lives | ClickHouse views or tables over silver | ClickHouse serving tables | registry | nowhere |
| Knows | what happened numerically | which records participated | how to compute and display | all three, combined |
| Authored by | connector + gold model | connector + gold model | one struct per metric | nobody |

## Documents

| Document | Description |
|---|---|
| [`specs/DESIGN.md`](specs/DESIGN.md) | System contract: observation contract, registry model, computations, result API, validation, authoring guide ("Adding a Metric") |

## Implementation

| Layer | Location |
|---|---|
| Metric registry (builtin seeds) | [`src/backend/services/analytics/src/domain/metric_definitions/builtin.rs`](../../../src/backend/services/analytics/src/domain/metric_definitions/builtin.rs) |
| Definition loading, reconciler, schema validator | [`src/backend/services/analytics/src/domain/metric_definitions/`](../../../src/backend/services/analytics/src/domain/metric_definitions/) |
| Result runtime (validation, query compiler, response builder) | [`src/backend/services/analytics/src/domain/metric_results/`](../../../src/backend/services/analytics/src/domain/metric_results/) |
| Result endpoint | [`src/backend/services/analytics/src/api/metric_results.rs`](../../../src/backend/services/analytics/src/api/metric_results.rs) |
| Drilldown runtime | [`src/backend/services/analytics/src/domain/metric_drilldown/`](../../../src/backend/services/analytics/src/domain/metric_drilldown/) |
| Drilldown endpoints | [`src/backend/services/analytics/src/api/metric_drilldown.rs`](../../../src/backend/services/analytics/src/api/metric_drilldown.rs) |
| Registry schema migration | [`src/backend/services/analytics/src/migration/m20260625_000001_metric_definitions.rs`](../../../src/backend/services/analytics/src/migration/m20260625_000001_metric_definitions.rs) |
| Managed observation sources (dbt gold models) | [`src/ingestion/gold/`](../../../src/ingestion/gold/) |
| Class-contract data-quality tests | [`src/ingestion/dbt/tests/ai/`](../../../src/ingestion/dbt/tests/ai/) |

## Boundaries

- Current deployments isolate one tenant per instance. Drilldown entity IDs
  remain source-derived identifiers, commonly normalized email addresses.
  Multi-tenant warehouse predicates, canonical person IDs, cross-source alias
  resolution, and subordinate authorization belong to the identity-resolution
  epic and are required before a multi-tenant instance enables drilldown.
- The AI class contracts feeding the observation models are documented in
  [`src/ingestion/silver/ai/schema.yml`](../../../src/ingestion/silver/ai/schema.yml)
  (activity invariant, label and conversation-count semantics).
- The legacy metric path ([`metric-catalog/`](../metric-catalog/) +
  ad-hoc `insight.*` gold views) is frozen for new metrics and remains only
  until its consumers migrate.
