# Worked example — metric → source drill-down (greenfield, lean)

Drill from any metric value to the source records behind it. Shape: **new capability, not yet
built** — no branch, no PR. This example shows three things the other two don't: staying inside the
author's own draft scope, explicit `n/a` handling, and how to handle a scenario whose tooling
doesn't exist.

> **Snapshot.** The denominators in this example were counted when it was
> written. They are illustrative, not current — re-take them with
> `.claude/skills/quality-vector-tests/scripts/counts.sh`. What the example teaches is the *form*, which does
> not move.

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
Every vector has something — so this is not a padding problem, it's a *falsifiability* problem.
Nothing here has a denominator, an oracle or a suite, "pagination tests" and "Coverage" are filed
under Efficiency, and the whole trust invariant the epic exists for (BR-1: if a tile says 42, the
drill-down must account for 42) has no scenario at all.

## Grounding that changed the scenarios
- `insight.ic_drill` is an **empty stub view** (`SELECT '' … FROM system.one WHERE 0`) reached via
  an OData `drill_id eq` filter — the contract exists, the evidence doesn't. So the scenarios are
  written against the issue's requirement ids, not against code.
- The issue defines its own requirement ids (`BR-n`); those serve as the reviewed criteria here.
  The AC review found three scenarios with **no criterion at all** (latency, run-cost, scanning) —
  flagged to the author rather than silently tagged.
- Countable denominators found in the repo: **26** connectors, **60** catalog metric keys across 5
  families, **17** acceptance criteria in the issue itself.
- Semgrep, Trivy and CodeQL run in this repo's CI (`.github/workflows/`), so a Security scenario is
  measurable here — and since the SPA lives in `src/frontend`, the same scanners cover it. There is
  **no load harness anywhere** (no k6, locust, gatling, jmeter). A Performance scenario naming one
  would have to be built first — which is a finding, not a formatting detail.

## After (canonical format)
```markdown
## Testing

Drill-down exists to make a number believable, so the headline scenario is the trust invariant
itself (BR-1): the evidence returned must account for exactly the value it explains. Everything
else guards the honesty of what surrounds it — volume, coverage, speed and exposure. Speed and
run-cost run on the reference-org dataset. The BR ids are the issue's own. Criteria map:
BR-1,BR-2 → 1 · BR-3 → 2 · BR-9 → 4 · BR-10,BR-11 → 3 · BR-12,BR-18 → 5; the remaining BRs
are proven by the epic's other subtasks, and scenarios 6–8 await the criteria proposed to the
author in the AC review.

- [ ] 1. **Count match** — Reliability · metric-spec · BR-1, BR-2 — for each catalog metric, run
      the metric query and the drill query with identical period, person and filters → the drilled
      record count equals the displayed value for all 60 catalog metrics, 0 discrepancy; derived
      values account for their stated inputs.
- [ ] 2. **Excluded records stay out** — Reliability · metric-spec · BR-3 — seed bot, automation,
      migration-artefact and unattributed records → 0 of them appear in any drill-down.
- [ ] 3. **Refusals** — Reliability · stand-api · BR-10, BR-11 — drive the two refusal cases →
      oversized request returns 4xx with a reason, never a partial 200; undrillable target returns
      an error distinguishable from an empty result.
- [ ] 4. **Page honesty** — Reliability · stand-api · BR-9 — page a 3,000-record fixture at page
      size 500 (6 pages) and union the pages → 0 duplicates, 0 omissions, total exact (3,000,
      not 500).
- [ ] 5. **Connector coverage** — Versatility · metric-spec · BR-18, BR-12 — per-connector
      fixtures driven by the metric catalog → all 26 connectors and 60 metrics return evidence or
      an explicit lineage gap; 0 silently undrillable.
- [ ] 6. **Drill latency** — Performance · stand-api — 200 requests on the reference-org dataset,
      deepest lineage path included → P95 < 1s. *No load harness exists in the repo today; wiring
      one is a prerequisite.*
- [ ] 7. **Memory growth** — Efficiency · manual — 30-minute soak of repeated paged requests →
      RSS growth < 5%, CPU back to baseline. *Proposed bar, no precedent in the repo.*
- [ ] 8. **Critical findings** — Security · ci-static — Trivy CRITICAL + Semgrep ERROR counts
      from the workflows already in CI → 0 (covers src/frontend too; the SPA is in this repo).
```

## What the format did
- Renamed behaviours into scenarios the author can read at a glance: "pagination tests" → **Page
  honesty**, "Cover all 25 connector data" → **Connector coverage**, "No critical issues in the ci
  pipeline" → **Critical findings**.
- Gave every countable expect a denominator counted from the repo (60 metrics, 26 connectors,
  3,000/500) — and corrected the issue's own "25 connectors" to the 26 the tree had when this was written.
- Moved "pagination tests" and "Coverage" from Efficiency to **Reliability** — paging correctness
  is a correctness claim; Efficiency kept the author's genuine run-cost item.
- Split "100% reconciliation" from "0 leaked records" into scenarios 1 and 2, because one expect
  can't carry two different oracles.
- Attributed every scenario to the suite it will land in — the two seeded-fixture scenarios to
  the metrics rig, the HTTP-contract ones to the stand suite, the scan to CI — so an unchecked
  box after implementation names exactly where the missing test belongs.
- Added exactly **one** scenario the author didn't have — the main gate — and flagged it to the
  user rather than slipping it in. The epic's own AC-1 demanded it.
- Flagged the two unbuildable bars and the one invented bar in italics, so nobody reports them
  green by default — and surfaced that scenarios 6–8 had no acceptance criterion, which is AC
  review output, not a formatting detail.

## Note on co-authored issues
This body was rewritten by the feature's engineer between two edits, dropping three requirements and
renumbering BR-15…21 → BR-12…18. Re-fetch the body immediately before editing, and re-check any
requirement ids you cite — a section built from a stale copy reverts someone else's work and leaves
dangling references.
