# Migration: current serving surface to the query engine

Companion to [DESIGN.md](DESIGN.md). The design reads timeless; this file
carries the mapping from what serves today to what the engine becomes, and is
deleted when the migration completes.

## What generalizes (machinery kept, renamed under the contract)

| Today | Becomes | Notes |
|---|---|---|
| Values total / timeseries / rollup grains | one `query` with `group_by` + optional `{time: true}` axis | grain enum dissolves into group-axis composition |
| Distributions endpoint (histogram, quantiles) | `bin` group axis + `quantile` aggregates | no separate surface |
| Compare windows (previous period / month / quarter / year) | `compare.aligned` | the equal-day-count rule is already implemented and tested |
| Group-cap "Other" folding | `top` with `remainder` | the ranking CTE, deterministic ties, and remainder fold survive as-is |
| Rows evidence pages (keyset + server sort) | unchanged | already generic |
| Ratio / derived metric computations | `expressions` over aggregate names | AST-rendered instead of two computation kinds |
| Percentile / stddev metrics | `quantile` / `stddev` aggregates | per-event row discipline unchanged |
| Seat-month/seat-day style period folds | `per_period` semi-additive aggregates | the hand-built shape becomes a contract feature |
| Measure cache (deferred PR) | refresh engine behind saved queries | its four review findings get fixed in that role |
| Metric definitions + seeds | saved queries + the OTB library | definitions store and versioning survive; the metric key stops being the API's spine |
| Dry-run validation endpoint | saved-query validation | same violation-list loop |
| Discovery catalog | dataset + saved-query catalog | advertises declarations, not metric capabilities |

## What dies

- **Peer comparison views and endpoint** — cohort spreads, target-vs-peers,
  the minimum-peer floor. A dashboard wanting a spread composes queries.
- **Capped-timeseries as a bespoke view kind** — subsumed by `top`.
- **Person-entity machinery as the API spine** — identity resolution and
  visibility become the declared entity policy of person-shaped datasets;
  datasets without the declaration carry none of it.
- **Metric-centric query shape** — requests name datasets (or saved queries), not
  metric keys; direction/format/labels move wholly into rendering metadata.
- **Grain/fold advertisement per metric** — the catalog describes datasets
  and saved queries; admissibility falls out of the contract, not per-key
  capability lists.

## What stays out of scope for this rebuild

- Ingestion layout and the field-catalog snapshot gate stay exactly as they
  are; the engine consumes the same catalog the current compiler does.
- Dataset authoring remains repository-owned (declarations in the seeds and
  roles files); no runtime authoring surface is part of this migration.

## Sequencing

1. Land the current stack (store, compiler, query surface, discovery catalog,
   families) — it is the kernel: the compiler's tenancy, null, identity, and
   read-discipline rules transfer verbatim.
2. Introduce `/v1/query` beside the existing surface, implementing the
   contract capability by capability; the existing endpoints keep serving
   until their consumers move.
3. Re-express the OTB metrics as saved queries; the transcription corpus (105
   metrics with exact legacy reconciliation) becomes the regression suite for
   the new contract.
4. Retire the metric-centric endpoints and the dead view kinds; the deferred
   cache PR lands re-aimed at saved-query refresh.
