# Worked example — identity resolution (C#→Rust port)

Port of the C# identity service to Rust, grown into a full identity epic. Shape: **port** → the
core gate is parity (same data in, same people out). This example shows the vector spread across
**all five** vectors, and the correction that grounding forced.

## The correction grounding forced
The author's first draft was almost all non-functional (soak, monitoring, load, scanning) — it
barely measured the *core* behaviour. For identity, a **wrong merge corrupts every downstream
metric**, so the headline must be resolution correctness + the no-false-merge safety + the C#→Rust
differential. Grounding also confirmed the framing: the person is **email-keyed** by the seed
sources; every other source maps its own key on, and the reviewer namespace (GitHub login /
Bitbucket display-name) ≠ author email (git→HR ≈ 68% today), so cross-namespace resolution is a
deferred, kept-red check.

## After (canonical format — full five-vector spread)
```markdown
## Testing

Port of the C# identity service to Rust. The person is **email-keyed** by the seed sources
(HR/Entra/comms/AI); every other source must map its own key onto that person. Speed runs on the
3,000-person demo org.

### Reliability
1. **People diff** *(main gate)*
   - Metric: people whose resolution differs between the C# and Rust services.
   - How measured: both services run over the same seeded dataset; compare the resolved person set.
   - Target: **0** differences.
2. **False merges**
   - Metric: distinct humans auto-merged into one person.
   - How measured: fixture with known-distinct near-duplicate identities (shared names, shared
     display-names, recycled logins).
   - Target: **0**.
3. **API + e2e coverage**
   - Metric: identity endpoints and the resolve path covered by automated tests.
   - How measured: e2e across MariaDB + ClickHouse + identity; contract/Swagger check.
   - Target: **3/3** endpoints; **0** regressions on existing screens.

### Versatility
4. **Org-chart sync**
   - Metric: directory providers whose reporting lines and team model resolve correctly.
   - How measured: per-provider fixtures — MS Entra, BambooHR (Workday / AD as they land).
   - Target: **2/2** live providers; manager + team at correct depth; root and cycles handled.
5. **Cross-namespace resolution**
   - Metric: share of reviewer-namespace identities (GitHub login / Bitbucket display-name)
     resolved to a person.
   - How measured: fixtures across all 26 connectors.
   - Target: resolved wherever evidence allows; **git→HR ≈ 68% today** — kept as a check that stays
     red until cross-namespace resolution lands, rather than a silently deferred gap.

### Performance
6. **Latency (P95)**
   - Metric: resolution / lookup latency under load.
   - How measured: load test with a baseline first, on the 3,000-person demo org.
   - Target: **< 100ms** — *proposed; no existing latency budget in the repo sets this bar.*

### Efficiency
7. **Memory growth**
   - Metric: RSS and CPU of the Rust service under sustained load.
   - How measured: soak on the demo dataset, sampled at start vs end.
   - Target: **< 5%** growth; CPU returns to baseline — *proposed; no soak precedent in the repo.*

### Security
8. **Critical findings**
   - Metric: critical findings in the new Rust service.
   - How measured: Trivy `--severity CRITICAL` + Semgrep `--severity ERROR` counts in CI.
   - Target: **0**.
```

## Notes
- The author's final call was to keep a leaner set without checks 1 and 2. Those two are what
  actually *prove* a port, so if a port's Testing section has no differential and no
  no-false-merge check, flag the gap against the issue's own Acceptance Criteria before pushing —
  then defer to the author's scope if they still want it out.
- "code coverage with e2e tests", which the author had under *Efficiency*, moved to **Reliability**
  — test rigor is a reliability signal, not a run-cost one.
- Before filing check 8, confirm the scanners exist: `grep -rniE "semgrep|trivy" .github/`. In this
  repo they do (semgrep, trivy and codeql workflows), so the target is real; `insight-front` has
  none, so a frontend-scoped security target there is still "not measurable until one is wired".
