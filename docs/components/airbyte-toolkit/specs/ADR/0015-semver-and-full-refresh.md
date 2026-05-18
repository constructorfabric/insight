---
status: accepted
date: 2026-05-13
decision-makers: platform-engineering
---

# ADR-0015: Strict Semver and Major-Bump Full-Refresh

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Bump kinds and effects](#bump-kinds-and-effects)
  - [No cross-connector cascade](#no-cross-connector-cascade)
  - [Legacy non-semver values](#legacy-non-semver-values)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Strict semver MAJOR.MINOR.PATCH](#option-a--strict-semver-majorminorpatch)
  - [Option B — Free-form version string (status quo)](#option-b--free-form-version-string-status-quo)
  - [Option C — Date-based versions](#option-c--date-based-versions)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-semver-and-full-refresh`

## Context and Problem Statement

ADR-0001 established that `descriptor.yaml.version` is the single anchor for "should I republish this definition?" but left the version format unconstrained — historically it has been a date-like string (`2026.05.04`), a one-component number (`1.0`), or a semver-ish triplet (`1.0.0`). With no parseable structure, the engine cannot answer two questions that the operator now expects it to:

1. **Did the connector change in a way that requires re-discovering the catalog (new streams, new fields)?** Today reconcile re-discovers only on first-time connection creation. If a connector adds a new stream in a subsequent version bump, the existing connection's `sync_catalog` is stale and the new stream silently never syncs.
2. **Did the connector change in a way that breaks bronze→silver semantics (renamed cursor, changed PK)?** A change of this magnitude requires the downstream dbt models to be re-materialized from scratch (`--full-refresh`), not incrementally appended. With a free-form version, the engine has no way to know it should signal that.

The operator needs to be able to *opt into* a full-refresh by bumping the major component, without resorting to manual `dbt run --full-refresh` invocations that bypass the ingestion pipeline.

## Decision Drivers

- **Parseable diff**: reconcile must classify a version change as `major`, `minor`, or `patch` deterministically.
- **No state loss surprises**: a routine `MINOR`/`PATCH` bump must not trigger a full-refresh; full-refresh is a destructive-by-design operation paid for only on `MAJOR`.
- **Catalog freshness**: on *every* version bump, the connection's `sync_catalog` must be re-discovered so newly-added streams and fields are auto-enabled — both for nocode and cdk connectors.
- **Predictable scope**: full-refresh on connector A must affect only A's downstream dbt scope (its `dbt_select`). No cross-connector cascade.
- **Migration**: existing descriptors carry non-semver values (`2026.05.04`, `1.0`). The transition must not force a synthetic full-refresh on every connector the day this lands.

## Considered Options

- **Option A** — Strict semver `MAJOR.MINOR.PATCH` (regex `^\d+\.\d+\.\d+$`, no pre-release).
- **Option B** — Keep version as a free-form string (status quo).
- **Option C** — Date-based versions (`YYYY.MM.DD`), as some connectors use today.

## Decision Outcome

Chosen option: **Option A — strict semver `MAJOR.MINOR.PATCH`**.

### Bump kinds and effects

Reconcile parses `descriptor.yaml.version` (target) and the Airbyte-side mirror (current) and classifies the diff:

| Bump kind | Trigger | Effects |
|---|---|---|
| `none` | `target == current` | No-op. |
| `patch` | `target.major == current.major AND target.minor == current.minor AND target.patch > current.patch` | Republish definition + re-discover catalog + update connection's `sync_catalog` (all streams & fields `selected: true`). No data action beyond the next scheduled/auto-triggered sync. |
| `minor` | `target.major == current.major AND target.minor > current.minor` | Same as `patch`. |
| `major` | `target.major > current.major` | Same as `patch` **PLUS** the auto-triggered sync's downstream dbt step runs once with `--full-refresh`. |
| `migration` | `current` is not parseable as semver but `target` is | Same as `patch`. Treated as a one-time format migration — does not trigger full-refresh even though the major number "increased". |

Re-discover applies to **both** connector types:

- **nocode**: after `connector_builder_projects/update_active_manifest`, call `sources/discover_schema` and `PATCH /connections/{id}` with the renormalized catalog.
- **cdk**: after `source_definitions/update` (image tag bump), same flow.

The dbt full-refresh signal is conveyed as a **one-shot trigger parameter** (`dbt_full_refresh=true`) on the auto-triggered sync workflow; it is not persisted anywhere. The next scheduled sync runs incrementally as usual.

**Bump-kind storage scope (this iteration)**: NoCode connectors already mirror `descriptor.version` onto `definition.declarativeManifest.description`, which is the natural place to read "current" from. CDK source_definitions in Airbyte have no equivalent semver-shaped field; their `dockerImageTag` carries a build identifier whose grammar is intentionally unconstrained (ADR-0011). Until a dedicated CDK semver-storage convention exists, **bump-kind classification — and therefore major-bump full-refresh dispatch — applies to NoCode connectors only**. CDK image bumps still trigger republish + catalog re-discover (new streams/fields auto-enabled), but emit `bump_kind == "patch"` regardless of how `descriptor.version` changed; operators who need to full-refresh a CDK connector run a separate explicit invocation. A follow-up will add an Airbyte-side semver tag for CDK (e.g. on the connection) to close the gap.

### No cross-connector cascade

Full-refresh dispatched from a major bump on connector A re-materializes only the dbt scope declared in A's `descriptor.yaml.dbt_select` (e.g. `tag:jira+`, `tag:silver,tag:jira+`). Silver/gold models that union connector A with connector B are *included* in A's scope and re-materialize fully — they read all of B's bronze data (append-only) as part of that re-materialization. Connector B itself is not resynced and its descriptor is not touched. This invariant is intentional: bronze is append-only and dedup happens in silver via `unique_key`, so an incidental cross-source dependency does not justify re-syncing the unrelated source.

### Legacy non-semver values

`descriptor.yaml` files in this repo today may carry `version: "2026.05.04"`, `version: "1.0"`, or other non-semver strings. The migration policy is:

- **Existing values are NOT changed by this ADR.** No mass rewrite.
- The next time an operator edits a connector and bumps its version, the new value MUST be strict semver. Reconcile rejects a non-semver `target` with a clear error.
- When reconcile reads back a non-semver `current` from Airbyte, it classifies the diff as `migration` and proceeds as if it were `patch` — no full-refresh.

This avoids a "big bang" where every connector simultaneously appears to bump from `2026.05.04` to `1.0.0` and triggers a fleet-wide full-refresh.

### Consequences

- **Good**, because `MAJOR` bumps now carry well-defined ingestion-side consequences operators can reason about; full-refresh stops being a per-developer ritual run by hand.
- **Good**, because new streams and fields added to a connector are auto-included in the next sync without operator intervention.
- **Good**, because the no-cascade invariant keeps the blast radius of a connector bump bounded.
- **Bad**, because operators must internalize semver discipline. Mitigation: reconcile rejects a non-semver target with a clear message.
- **Bad**, because re-discover adds an `sources/discover_schema` call on every republish — for nocode connectors with large schemas this is a measurable extra cost. Mitigation: re-discover is only called when a bump is detected, not on every reconcile tick.

### Confirmation

- Unit test: `classify_bump.py` accepts only `MAJOR.MINOR.PATCH` as target; emits the right `bump_kind` for every case in the table above.
- Integration test: bumping a connector's `MINOR` and reconciling causes `sync_catalog` on the existing connection to be PATCHed with the freshly-discovered streams, all `selected: true`.
- Integration test: bumping `MAJOR` results in the next auto-trigger carrying `dbt_full_refresh=true`; the subsequent scheduled run does not.
- Regression: with a legacy `current` (`2026.05.04`) and a semver `target` (`1.0.0`), no full-refresh is dispatched.

## Pros and Cons of the Options

### Option A — Strict semver MAJOR.MINOR.PATCH

A regex-validated triplet. Operator-meaningful: major = "breaking", minor = "additive", patch = "fix".

- Good, because parseable and well-understood; aligns with semver.org.
- Good, because three components are enough to carry the major-bump signal without the complexity of pre-release labels.
- Bad, because requires migration of legacy values — handled by the "migration" bump kind above.

### Option B — Free-form version string (status quo)

Whatever the operator types.

- Good, because no migration cost.
- Bad, because reconcile cannot distinguish a major change from a typo; the new full-refresh semantics simply cannot be expressed.

### Option C — Date-based versions

`YYYY.MM.DD[.HH.MM]` — what jira/cursor descriptors carry today.

- Good, because already in use for some connectors.
- Good, because monotonically increasing without operator decision.
- Bad, because "major change" has no representation — the operator cannot communicate breaking-vs-additive through the version string.
- Bad, because the same chronological day cannot host two independent changes.

## More Information

- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — establishes the version-as-anchor mechanism this ADR refines.
  - `cpt-insightspec-adr-auto-trigger-sync-on-data-change` (ADR-0008) — the carrier for the `dbt_full_refresh` parameter on the one-shot sync.
  - `cpt-insightspec-adr-nocode-via-builder-projects` (ADR-0010) — the API path for nocode catalog refresh.
  - `cpt-insightspec-adr-cdk-prebuilt-images` (ADR-0011) — the API path for cdk catalog refresh.
  - Bronze-as-append-only + silver-dedup-by-`unique_key` convention (see `docs/domain/ingestion-data-flow/specs/`) — the architectural foundation that makes no-cross-connector-cascade safe.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE**: [FEATURE.md](../feature-reconcile/FEATURE.md)

This decision addresses:

- `cpt-insightspec-fr-semver-version-format` — version field follows strict semver.
- `cpt-insightspec-fr-catalog-refresh-on-bump` — every version bump re-discovers the catalog so new streams and fields are auto-enabled.
- `cpt-insightspec-fr-full-refresh-on-major-bump` — major bumps dispatch a one-shot dbt full-refresh for the connector's `dbt_select` scope.
- `cpt-insightspec-fr-no-cross-connector-cascade` — full-refresh of connector A does not trigger full-refresh of connector B even when downstream silver models join both.
