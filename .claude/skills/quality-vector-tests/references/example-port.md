# Worked example — identity resolution (C#→Rust port)

> **Snapshot.** Denominators here were counted when the example was written.
> Re-take them with `.claude/skills/quality-vector-tests/scripts/counts.sh`; the *form* is what this teaches.

Port of the C# identity service to Rust, grown into a full identity epic. Shape: **port** → the
core gate is parity (same data in, same people out). This example shows the vector spread across
**all five** vectors, and the correction that grounding forced.

## The correction grounding forced
The author's first draft was almost all non-functional (soak, monitoring, load, scanning) — it
barely measured the *core* behaviour. For identity, a **wrong merge corrupts every downstream
metric**, so the headline must be resolution correctness + the no-false-merge safety + the C#→Rust
differential. Grounding also confirmed the framing: the person is **email-keyed** by the seed
sources; every other source maps its own key on, and the reviewer namespace (GitHub login /
Bitbucket display-name) ≠ author email, and nothing joins the two namespaces yet — so
cross-namespace resolution is a deferred, kept-red scenario.

## After (canonical format — full five-vector spread)
```markdown
## Testing

Port of the C# identity service to Rust. The person is **email-keyed** by the seed sources
(HR/Entra/comms/AI); every other source must map its own key onto that person. Speed runs on the
3,000-person demo org. All 4/4 acceptance criteria covered: AC-1 → 1,2 · AC-2 → 3 ·
AC-3 → 4 · AC-4 → 5; scenarios 6–8 await the criteria proposed in the AC review (the issue
defined none for speed, run-cost or scanning).

- [ ] 1. **People diff** *(main gate)* — Reliability · identity-e2e · AC-1 — run the C# and Rust
      services over the same seeded dataset and compare the resolved person set → 0 differences.
- [ ] 2. **False merges** — Reliability · identity-e2e · AC-1 — build a fixture of known-distinct
      near-duplicate identities (shared names, shared display-names, recycled logins), resolve
      each → 0 distinct humans merged into one person.
- [ ] 3. **Endpoint coverage** — Reliability · stand-api · AC-2 — e2e across MariaDB + ClickHouse +
      identity with a contract check → 3/3 endpoints covered.
- [ ] 4. **Org-chart sync** — Versatility · identity-e2e · AC-3 — per-provider fixtures (MS Entra,
      BambooHR; Workday / AD as they land) → 2/2 live providers resolve reporting lines: manager
      and team at correct depth, root and cycles handled.
- [ ] 5. **Cross-namespace resolution** — Versatility · identity-e2e · AC-4 — fixtures across all 26
      connectors → reviewer-namespace identities (GitHub login / Bitbucket display-name)
      resolved wherever the seeded fixtures carry linking evidence; the unlinkable remainder is
      an xfail assertion that flips to a hard pass when cross-namespace resolution lands —
      tracked in the suite, not silently deferred.
- [ ] 6. **Resolution latency** — Performance · manual — load test with a baseline first, on the
      3,000-person demo org → P95 < 100ms. *Proposed; no existing latency budget in the repo sets
      this bar.*
- [ ] 7. **Memory growth** — Efficiency · manual — soak on the demo dataset, sampled at start vs
      end → RSS growth < 5%, CPU back to baseline. *Proposed; no soak precedent in the repo.*
- [ ] 8. **Critical findings** — Security · ci-static — Trivy CRITICAL + Semgrep ERROR counts in
      CI → 0.
```

## Notes
- The author's final call was to keep a leaner set without scenarios 1 and 2. Those two are what
  actually *prove* a port, so if a port's Testing section has no differential and no
  no-false-merge scenario, flag the gap against the issue's own Acceptance Criteria before pushing —
  then defer to the author's scope if they still want it out.
- "code coverage with e2e tests", which the author had under *Efficiency*, moved to **Reliability**
  — test rigor is a reliability signal, not a run-cost one.
- Before filing scenario 8, confirm the scanners exist: `grep -rniE "semgrep|trivy" .github/`. In
  this repo they do (semgrep, trivy and codeql workflows), so the expect is real — and it covers
  `src/frontend` as well, since the SPA lives in this repo.
