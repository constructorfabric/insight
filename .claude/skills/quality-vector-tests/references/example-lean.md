# Worked example — metric → source drill-down (greenfield, lean)

Drill from any metric value to the source records behind it. Shape: **new capability, not yet
built** — no branch, no PR. This example shows three things the other two don't: staying inside the
author's own draft scope, explicit `Not applicable` vectors, and how to handle a target whose
tooling doesn't exist.

## Before (loose author draft)
```
**Reliability** 
API coverage - limits - should return error for too much data requested. 

**Efficiency**
Resource usage per service for drill-down requests - pagination tests. Coverage. 

**Versatility** 
Cover all 25 connector data -  by metric group

**Performance** 
Latency for drill down requests 

**Security** 
No critical issues in the ci pipeline for static analysis
```
Every vector has something — so this is not a padding problem, it's a *measurability* problem.
Nothing here has a denominator or a number, "pagination tests" and "Coverage" are filed under
Efficiency, and the whole trust invariant the epic exists for (BR-1: if a tile says 42, the
drill-down must account for 42) has no check at all.

## Grounding that changed the numbers
- `insight.ic_drill` is an **empty stub view** (`SELECT '' … FROM system.one WHERE 0`) reached via
  an OData `drill_id eq` filter — the contract exists, the evidence doesn't. So the checks are
  written against the issue's requirement ids, not against code.
- Countable denominators found in the repo: **26** connectors, **59** catalog metric keys across 5
  families, **17** acceptance criteria in the issue itself.
- Semgrep, Trivy and CodeQL run in this repo's CI (`.github/workflows/`), so a Security target is
  measurable here — and since the SPA lives in `src/frontend`, the same scanners cover it. There is
  **no load harness anywhere** (no k6, locust, gatling, jmeter). A Performance target naming one
  would have to be built first — which is a finding, not a formatting detail.

## After (canonical format)
```markdown
## Testing

Drill-down exists to make a number believable, so the headline check is the trust invariant itself
(BR-1): the evidence returned must account for exactly the value it explains. Everything else
guards the honesty of what surrounds it — volume, coverage, speed and exposure. Speed and run-cost
are measured on the reference-org dataset.

### Reliability
1. **Count match** *(main gate — BR-1, BR-2)*
   - Metric: drilled record count ÷ displayed value.
   - How measured: for each catalog metric, run the metric query and the drill query with identical
     period, person and filters, and compare; derived values compare against their stated inputs.
   - Target: **59/59** metrics match, **0** discrepancy.
2. **Excluded records shown** *(BR-3)*
   - Metric: records the metric excluded that appear in its drill-down.
   - How measured: fixture seeded with bot, automation, migration-artefact and unattributed records.
   - Target: **0**.
3. **API coverage** *(BR-10, BR-11)*
   - Metric: acceptance criteria covered by an automated test.
   - How measured: happy path plus the two refusal cases — oversized request, undrillable target.
   - Target: **17/17**; oversized → 4xx + reason, never a partial 200; undrillable → error
     distinguishable from an empty result.
4. **Page errors** *(BR-9)*
   - Metric: duplicate rows, missing rows, and reported-total accuracy.
   - How measured: page a 3,000-record fixture at page size 500 (7 pages) and union the pages.
   - Target: **0** duplicates, **0** omissions, total exact (3,000, not 500).

### Versatility
5. **Connector coverage** *(BR-18, BR-12)*
   - Metric: connectors and metrics that return evidence or an explicit lineage gap.
   - How measured: per-connector fixtures driven by the metric catalog.
   - Target: **26/26** connectors, **59/59** metrics; **0** silently undrillable.

### Performance
6. **Latency (P95)**
   - Metric: drill request latency at reference-org scale.
   - How measured: 200 requests on the reference-org dataset, deepest lineage path included.
   - Target: **< 1s** — *no load harness exists in the repo today; one is a prerequisite.*

### Efficiency
7. **Memory growth**
   - Metric: RSS of the services a drill touches, start vs end.
   - How measured: 30-minute soak driving repeated paged requests on the reference-org dataset.
   - Target: **< 5%** growth, CPU back to baseline — *proposed bar, no precedent in the repo.*

### Security
8. **Critical findings**
   - Metric: critical findings in the new drill-down code.
   - How measured: Trivy `--severity CRITICAL` + Semgrep `--severity ERROR` counts, from the
     workflows already in this repo's CI.
   - Target: **0** — *covers `src/frontend` too, since the SPA is in this repo.*
```

## What the format did
- Renamed behaviours into metrics the author can read at a glance: "pagination tests" → **Page
  errors**, "Cover all 25 connector data" → **Connector coverage**, "No critical issues in the ci
  pipeline" → **Critical findings**.
- Gave every target a denominator counted from the repo (59, 25, 17, 3,000/500) instead of "all"
  or "100%".
- Moved "pagination tests" and "Coverage" from Efficiency to **Reliability** — paging correctness is
  a correctness claim; Efficiency kept the author's genuine run-cost item.
- Split "100% reconciliation" from "0 leaked records" into checks 1 and 2, because one target line
  can't carry two numbers.
- Added exactly **one** check the author didn't have — the main gate — and flagged it to the user
  rather than slipping it in. The epic's own AC-1 demanded it.
- Marked the two unbuildable targets and the one invented bar in italics, so nobody reports them
  green by default.

## Note on co-authored issues
This body was rewritten by the feature's engineer between two edits, dropping three requirements and
renumbering BR-15…21 → BR-12…18. Re-fetch the body immediately before editing, and re-check any
requirement ids you cite — a section built from a stale copy reverts someone else's work and leaves
dangling references.
