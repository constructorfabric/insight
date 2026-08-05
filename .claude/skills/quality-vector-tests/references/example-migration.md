# Worked example — a unified metric system (rolling migration)

A generic metrics runtime that the whole catalog migrates onto, wave by wave. Shape: **migration
platform** → the headline check is a reusable differential gate, and coverage must be
registry-driven (the metric list is still settling, so measure the machinery, not a fixed list).

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
no denominators, and `uptime/logging` filed as a check when it needs an observability stack that
isn't wired.

## After (canonical format)
```markdown
## Testing

One system serves every metric; each group migrates onto it in turn. So we measure the shared
machinery and the invariants — not a fixed metric list. All speed checks run on the reference-org
dataset so numbers stay comparable over time.

### Reliability
1. **Old-vs-new diff** *(main gate)*
   - Metric: metric values differing between the old per-domain path and the new engine.
   - How measured: automated diff harness over the seeded dataset, every catalog metric run on both
     paths; each metric tagged `exact`, `known-diff` (assert direction), or `merge` (merged = Σ parts).
   - Target: **0** untagged differences.
2. **Definition integrity**
   - Metric: invalid definitions served, and empty windows during a reload.
   - How measured: unit + integration on the loader/reconciler, including a concurrent-reload test.
   - Target: **0** invalid definitions served, **0** empty windows.
3. **Bad-request handling**
   - Metric: malformed requests returning a clear error rather than data or a crash.
   - How measured: API tests over the 4 rejection cases — unknown metric, bad dimension, oversized
     result set, malformed `metric_date` (the YYYY-MM-DD filter guard).
   - Target: **4/4** return a specific error; **0** wrong results, **0** crashes.

### Versatility
4. **Metric × view coverage**
   - Metric: catalog metrics served correctly in each of the 4 views (period, peers, over-time, breakdown).
   - How measured: registry-driven harness reading the catalog; asserts value + dimensions + peer group.
   - Target: **100%** of catalog metrics; a new metric is covered by config, not new test code.
5. **UI group coverage**
   - Metric: migrated metric groups rendering correctly on the dashboard.
   - How measured: Playwright against the metric-collection renderer; sequenced behind the FE build.
   - Target: every migrated group displays; **0** regressions on existing screens.

### Performance
6. **Latency (P95)**
   - Metric: per-metric endpoint latency at reference-org scale.
   - How measured: load harness on the reference-org dataset, P95 per endpoint.
   - Target: **< 1s**.
7. **Dashboard load**
   - Metric: page-load and time-to-interactive for a team.
   - How measured: Lighthouse (load) + Playwright (interactive), same dataset — *neither is wired
     today: no lighthouse in `insight-front`, and its Playwright is browser-mode unit testing, not
     an e2e suite. Wiring one is a prerequisite.*
   - Target: **< 10s**.

### Efficiency
**Not applicable** — the runtime replaces existing query paths rather than adding a service; run-cost
is unchanged and tracked at the platform level.

### Security
**Not applicable** — no new external surface; the endpoint inherits the gateway's existing authn/authz.
```

## What the format did
- De-paired the vectors; one check per vector; continuous 1–7 numbering.
- Turned `coverage?` / `Lighthouse?` into decided targets with denominators (`4/4` rejection cases,
  `100%` of catalog metrics, `< 10s`).
- Renamed behaviours into metrics: "Smoke test for metrics" → **Metric × view coverage**;
  "Dashboard loading time. Lighthouse?" → **Dashboard load**.
- Added the differential as the `main gate` (missing from the draft) — the real proof the migrated
  numbers are right.
- Dropped `uptime/logging`, an un-runnable check blocked on an observability stack that isn't wired,
  and made Efficiency/Security explicit `Not applicable` rather than silently absent.
