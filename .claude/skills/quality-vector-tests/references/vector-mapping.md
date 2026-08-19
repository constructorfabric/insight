# Vector mapping — which scenario goes under which quality vector

Every scenario belongs to **exactly one** vector. When a scenario seems to fit two, file it
under the vector whose *risk* it addresses, or split it into two scenarios. The suite a
scenario lands in is a separate dimension — that's the layer table in SKILL.md.

## The five vectors and what belongs to each

| Vector | Guiding question | Scenarios that live here | Common miss |
|---|---|---|---|
| **Efficiency** | What does it cost to *run* Insight, and how does it scale? | Reference-org compute footprint (CPU/mem); resource utilization per service/connector; soak tests for resource growth/leaks. Storage is negligible — lead with compute. | Test *coverage* is **not** Efficiency — it's Reliability. Delivery-pipeline speed is an engineering-process signal, not product Efficiency. |
| **Reliability** | Can the user trust the dashboard — right, current, up? | The **differential/parity gate** (port/migration/consolidation); reconciliation of a number against its evidence; correctness of the headline behavior; data validation; **pagination integrity** (true totals, no duplicates, no omissions); metric-definition loading/precedence/reconcile; fail-clear on invalid/oversized input; source freshness; sync success; **test coverage (unit / API / e2e)** as the leading indicator; service availability. | Don't bury the differential inside a generic "correctness" bullet — it's usually the headline, tag it `*(main gate)*`. Pagination gets filed under Efficiency because it involves volume, but "no duplicates, no omissions" is a correctness claim. |
| **Performance** | How fast at the reference org's scale? | Per-endpoint latency budgets (gate P95, track P99/P999); dashboard page-load via Lighthouse; interactive timing via Playwright — neither is wired in the repo today, so verify before naming them (SKILL.md step 3). Measure on the **same reference-org fixture** as Efficiency. | No global averages — the data endpoints *are* the product, so budget per metric/endpoint. |
| **Security** | Is the surface safe? | Static + dependency scanning (Semgrep, Trivy — no critical); authn/authz; no secret/token leak; the security face of tenant isolation; input-validation guards (e.g. SQLi on a filter param). | The *data* face of tenant isolation (does one tenant's data appear for another) can also read as Reliability — pick one and don't double-list. |
| **Versatility** | How broad is the coverage — across sources, and across the catalog and its surfaces? | Per-source / per-connector coverage (does each source participate correctly?); vendor coverage; API-version currency; connector readiness (bronze→silver→gold, tested); org-chart sync across directory providers; and catalog breadth — every metric resolving across every UI view or surface that should render it. | A connector counts only when production-complete *and tested* — coverage means the metric actually resolves, not just that the connector exists. |

## Cross-cutting note: "coverage" is not one thing
This mapping deliberately splits coverage by *what it covers*:
- **test-rigor** coverage (unit / API / metric-tests) → **Reliability**
- **breadth** coverage (connector readiness, catalog breadth) → **Versatility**
- **security-surface** coverage (dependency, secrets) → **Security**

So "code coverage with e2e tests" → Reliability; "coverage of each connector for each metric" →
Versatility; "no critical Trivy findings" → Security. Assign by *what the number measures*, not by
the word "coverage".

## The shared fixture
Efficiency and Performance are measured on **the same reference-organisation dataset** — a defined
user count, connector set and concurrency profile. Do not invent its size: take it from the perf
target the feature itself cites, name that number in the framing sentence, and reuse it across
both vectors so results compare run to run. Features that predate a shared fixture use their own
demo org (the examples here use a 3,000-person one); say which you measured on.
