# Vector mapping — which scenario goes under which quality vector

The [authoring guide](../../../../.cf-studio/config/kits/sdlc/guides/quality-vectors.md)
owns the product definitions. These are common Insight probes, not exhaustive
definitions. Each scenario has one primary vector for its verification claim;
independent claims in different vectors use assertion-focused tests sharing setup.
The test suite is a separate dimension, described in SKILL.md.

## The five vectors and what belongs to each

| Vector | Guiding question | Scenarios that live here | Common miss |
|---|---|---|---|
| **Efficiency** | What does it cost to deliver, operate and use this capability? | Compute and storage per unit of useful work; resource growth during a soak; operator effort for setup, recovery or upgrades; user effort in an agreed workflow. | Select the cost that matters to the requirement. Neither storage nor human effort is negligible by definition. Test coverage measures evidence, not operating cost. |
| **Reliability** | Can the user trust the dashboard — right, current, up and recoverable? | Differential/parity checks; reconciliation against evidence; data and pagination integrity; source freshness; sync recovery; service availability and dependency-failure behavior. | Test coverage informs confidence but does not measure product reliability. Choose assertions for the actual consistency, correctness or recovery obligation. |
| **Performance** | How fast, and at what sustained rate and scale? | Operation latency, throughput and scaling at the requirement's own load and measurement boundary; page or interactive timing when relevant tools are available. Reuse the approved reference fixture with Efficiency where comparable. | Preserve the upstream percentile, target and conditions. Do not impose a universal P95/P99 rule or infer an end-to-end percentile by adding component percentiles. |
| **Security** | Is the surface safe? | Static + dependency scanning (Semgrep, Trivy — no critical); authn/authz; no secret/token leak; the security face of tenant isolation; input-validation guards (e.g. SQLi on a filter param). | The *data* face of tenant isolation (does one tenant's data appear for another) can also read as Reliability — pick one and don't double-list. |
| **Versatility** | How broad is the coverage — across sources, and across the catalog and its surfaces? | Per-source / per-connector coverage (does each source participate correctly?); vendor coverage; API-version currency; connector readiness (bronze→silver→gold, tested); org-chart sync across directory providers; and catalog breadth — every metric resolving across every UI view or surface that should render it. | A connector counts only when production-complete *and tested* — coverage means the metric actually resolves, not just that the connector exists. |

## Cross-cutting note: "coverage" is not one thing
This mapping deliberately splits coverage by *what it covers*:
- **test-rigor** coverage (unit / API / metric-tests) → evidence confidence; it does not itself satisfy a product quality vector
- **breadth** coverage (connector readiness, catalog breadth) → **Versatility**
- **security-surface** coverage (dependency, secrets) → **Security**

Thus connector participation across the supported metric catalog can verify
Versatility, while findings against an agreed security policy can verify Security.
Code coverage alone establishes neither outcome.

## The shared fixture
Efficiency and Performance are measured on **the same reference-organisation dataset** — a defined
user count, connector set and concurrency profile. Do not invent its size: take it from the perf
target the feature itself cites, name that number in the framing sentence, and reuse it across
both vectors so results compare run to run. Features that predate a shared fixture use their own
demo org; say which you measured on.
