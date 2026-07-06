# Metrics Domain

The unified metrics system: metrics are defined once in a typed registry,
computed by one generic runtime over normalized source measure observations,
and served self-describing through `POST /v1/metric-results`. All new metrics
are authored through this system.

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
