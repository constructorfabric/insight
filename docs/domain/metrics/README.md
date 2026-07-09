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

| | Observation | Definition | Metric result |
|---|---|---|---|
| What | a fact | the meaning of facts | the computed answer |
| Lives | ClickHouse views over silver (computed on read, nothing stored) | registry (MariaDB, seeded from Rust) | nowhere — made per request |
| Knows | what happened | what it is called, how to compute, how to show | both, combined |
| Authored by | connector + gold model | one struct per metric | nobody — the runtime derives it |

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
| Registry schema migration | [`src/backend/services/analytics/src/migration/m20260625_000001_metric_definitions.rs`](../../../src/backend/services/analytics/src/migration/m20260625_000001_metric_definitions.rs) |
| Managed observation sources (dbt gold models) | [`src/ingestion/gold/`](../../../src/ingestion/gold/) |
| Class-contract data-quality tests | [`src/ingestion/dbt/tests/ai/`](../../../src/ingestion/dbt/tests/ai/) |

## Boundaries

- The AI class contracts feeding the observation models are documented in
  [`src/ingestion/silver/ai/schema.yml`](../../../src/ingestion/silver/ai/schema.yml)
  (activity invariant, label and conversation-count semantics).
- The legacy metric path ([`metric-catalog/`](../metric-catalog/) +
  ad-hoc `insight.*` gold views) is frozen for new metrics and remains only
  until its consumers migrate.
