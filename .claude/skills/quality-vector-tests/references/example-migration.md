# Worked example — a unified metric system (rolling migration)

A generic metrics runtime that the whole catalog migrates onto, wave by wave. Shape: **migration
platform** → the headline scenario is a reusable differential gate, and coverage must be
registry-driven (the metric list is still settling, so measure the machinery, not a fixed list).

> **Snapshot.** Denominators and bars here were current when the example was written;
> re-take any with `.claude/skills/quality-vector-tests/scripts/counts.sh`. The *form* is what
> this teaches.

## Before (loose author draft)
```
** Testing ** 
Single API endpoint:
- Efficiency + Versatility:
   1. Smoke test for metrics. Metrics - coverage of the each metric + coverage of each connector...
   2. UI e2e tests - coverage? 
- Performance + Efficiency:
   2. For dataset for organisation [link] - each metric returns <1s Latency...
   3. Dashboard loading time. Lighthouse? Playwright?  for same dataset.  <10s for team?
- Reliability:
   4. Monitoring for system health. Metric - uptime. + Logging - amount of error messages...
```
Problems: paired vectors, broken numbering (1,2 → 2,3 → 4), open questions, no differential gate,
no suites, and `uptime/logging` filed as a scenario when it needs an observability stack that
isn't wired. The issue also carried no usable acceptance-criteria list — the AC review proposed
the AC-1..7 set the scenarios below cite, filed back into the issue on the author's confirmation.

## After (canonical format)
```markdown
## Testing

One system serves every metric; each group migrates onto it in turn. So we measure the shared
machinery and the invariants — not a fixed metric list. All speed checks run on the reference-org
dataset so numbers stay comparable over time. All 7/7 acceptance criteria covered:
AC-1 → 1 · AC-2 → 2 · AC-3 → 3 · AC-4 → 4 · AC-5 → 5 · AC-6 → 6 · AC-7 → 7.

- [ ] 1. **Old-vs-new diff** *(main gate)* — Reliability · metric-spec · AC-1 — automated diff harness
      over the seeded dataset, every catalog metric run on both paths, each tagged `exact` /
      `known-diff(direction)` / `merge` (merged = Σ parts) → 0 untagged differences.
- [ ] 2. **Definition integrity** — Reliability · rust-unit · AC-2 — loader/reconciler tests including a
      concurrent reload → 0 invalid definitions served, 0 empty windows during a reload.
- [ ] 3. **Bad-request handling** — Reliability · stand-api · AC-3 — drive the 4 rejection cases
      (unknown metric, bad dimension, oversized result set, malformed `metric_date`) → 4/4 return
      a specific error; 0 wrong results, 0 crashes.
- [ ] 4. **Metric × view coverage** — Versatility · metric-spec · AC-4 — registry-driven harness reading
      the catalog, each metric requested in all 4 views (period, peers, over-time, breakdown) →
      value, dimensions and peer group right for 100% of catalog metrics; a new metric is covered
      by config, not new test code.
- [ ] 5. **UI group coverage** — Versatility · stand-ui · AC-5 — open each migrated metric group on the
      dashboard → every group displays, 0 regressions on existing screens. *Sequenced behind the
      FE build; the browser is the only prover of rendering.*
- [ ] 6. **Endpoint latency** — Performance · stand-api · AC-6 — load harness on the reference-org
      dataset → P95 < 1s per endpoint. *No harness wired today; wiring one is a prerequisite.*
- [ ] 7. **Dashboard load** — Performance · stand-ui · AC-7 — Lighthouse (load) + Playwright
      (interactive), same dataset → < 10s for a team. *Neither is wired today: no lighthouse in
      `src/frontend`, and its Playwright is browser-mode unit testing, not an e2e suite.*

Efficiency — n/a: the runtime replaces existing query paths rather than adding a service;
run-cost is unchanged and tracked at the platform level.
Security — n/a: no new external surface; the endpoint inherits the gateway's existing authn/authz.
```

## What the format did
- De-paired the vectors; one scenario, one vector, one suite; continuous 1–7 numbering.
- Turned `coverage?` / `Lighthouse?` into decided expects with denominators (`4/4` rejection
  cases, `100%` of catalog metrics, `< 10s`).
- Renamed behaviours into scenarios: "Smoke test for metrics" → **Metric × view coverage**;
  "Dashboard loading time. Lighthouse?" → **Dashboard load**.
- Proposed the AC-1..7 criteria the issue lacked and filed the numbering back, so every tag
  resolves.
- Added the differential as the `main gate` (missing from the draft) — the real proof the migrated
  numbers are right.
- Attributed each scenario to the suite it lands in, so the checkbox audit after each migration
  wave names exactly which suite is missing its test.
- Dropped `uptime/logging`, an un-runnable scenario blocked on an observability stack that isn't
  wired, and made Efficiency/Security explicit `n/a` rather than silently absent.
