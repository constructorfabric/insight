# Testing Strategy

Insight is tested **shift-left**: contributors run the majority of checks locally before opening a PR. The strategy has
two axes — **levels** (what a test proves) and **environments** (where it runs). Several tests can share a level.

Entry points: `./dev-compose.sh test-stand` (raises a compose stand and runs the suites under `tests/`),
`scripts/ci/*` (coverage + spec gates), and the standard per-language tools (`cargo`, `pytest`).

---

## 1. Test pyramid

| Level | Scope | Tooling | Runs in |
|---|---|---|---|
| **Unit** | One function / module in isolation | `cargo test`, `pytest`, `vitest` | CI (every PR) |
| **Integration** | Components against real stores; the API contract | Testcontainers · dbt tests · OpenAPI-drift + metric-coverage · API & metric rig | CI (every PR) |
| **E2E** | The whole system through its real surfaces | ingestion (Airbyte → Argo) · chart install + smoke · compose-stand suite (API contract + UI journeys, Playwright) | CI (smoke, +2 non-required compose-stand checks) · Test (full) · Beta (shallow) |
| **Performance** | It stays fast under load | latency p50/95/99 · load · stress · soak | Test · Beta |

**Push tests down** — write each check at the lowest level that gives confidence; higher levels exist only for what
lower ones can't cover.

```sh
# fast local loop
cd src/backend && cargo test                        # Rust unit + integration (all services, incl. identity-resolution)
./dev-compose.sh test-stand minimal --instance=datapath          # an instance of your own, identity seeded
./dev-compose.sh test-stand test --tree=tests/datapath/metrics/ai --instance=datapath -q   # one metric class
```

---

## 2. Environments

A build is **promoted up** (CI → Test → Beta); a proven check is **gated down** (report-only in Test → blocking in CI).

| Environment | What it is | Trigger | Runs | Gates |
|---|---|---|---|---|
| **CI** | ephemeral, per-PR | every PR | Unit + Integration + smoke/BVT (5–15 min) | blocks merge |
| **Test** | long-lived, tracks `main` | merge + nightly | full regression — all levels, real orchestration | reports; files regressions |
| **Beta** | prod-parity, pre-release | release candidate | acceptance / shallow validation + soak/perf | gates the release |

● full · ◐ smoke/subset/shallow · ○ not run

| Level | CI | Test | Beta |
|---|:--:|:--:|:--:|
| Unit | ● | ● | ○ |
| Integration | ● | ● | ○ |
| E2E | ◐ | ● | ◐ |
| Performance | ○ | ● | ● |

---

## 3. Coverage

- Line-coverage threshold is **80 %** per component, plus **80 % on new code** (diff-cover).
- Enforced by `scripts/ci/coverage.py` over Cobertura reports: per-language jobs upload reports, the `coverage-gate`
  job judges them and writes a job-summary. `coverage-gate` **must** be the required status check.
- Only **changed** components are measured on a PR (`scripts/ci/changed.py`).
- Both gates only see files present in a Cobertura report. A path excluded from a component's own coverage
  config is invisible to the new-code gate too — it is not scored zero, it is omitted. For the frontend that
  exemption covers the vendored `src/components/ui/**` primitives, the thin `src/routes/**` wrappers,
  generated files, and stories (`src/frontend/vitest.config.ts`).

---

## 4. Unit

- Every new public function / behaviour **must** have at least one unit test.
- Pure logic **must not** reach for a DB or the network — that belongs in Integration.

```sh
cd src/backend && cargo test                                 # Rust (use `cargo test -p identity-resolution` to scope)
cd src/ingestion/connectors/<domain>/<name> && pytest        # Python connector
cargo fmt --check && cargo clippy --all-targets              # lint
```

**CI:** `ci.yml` — fmt + clippy + coverage, per changed component.

---

## 5. Integration

Components against a real store, and the API contract:

- **Testcontainers** — identity-resolution (Rust) against a real MariaDB.
- **Identity data path** — `tests/datapath/identity`: a connector's bronze reaching persons-seed, and an
  operator's correction surviving the seed run after it, read back as stand personas.
- **dbt data tests** — bronze → silver → gold model assertions.
- **Contract** — OpenAPI-drift + metric-coverage gates (every served `metric_key` is value-asserted or skip-listed).
- **Metric specs** — `tests/datapath/metrics`: seed bronze → dbt → ClickHouse gold → the analytics HTTP answer,
  read as a seeded persona of a compose stand the suite raised for itself (`test-stand minimal`).

> The data-path suite is **Integration, not E2E** — it seeds bronze directly (no orchestrators) and asserts at the
> API (no UI). The workflow file `e2e-bronze-to-api.yml` keeps its name because a required check is named after it.

```sh
./dev-compose.sh test-stand minimal --instance=datapath
./dev-compose.sh test-stand test --tree=tests/datapath/metrics/tasks --instance=datapath -q
./dev-compose.sh test-stand down --instance=datapath   # full reset: volumes go too
```

**CI:** `e2e-bronze-to-api.yml` — one shard per metric class plus one for identity, each on a minimal stand of
its own, and a blocking metric-coverage gate (`tests/lib/insight_datapath/metric_coverage.py`: every builtin
metric the catalogue serves is asserted by some spec). OpenAPI drift is a separate workflow (`openapi-specs.yml`);
the HTTP contract lanes live on the deployed stand (`e2e-stand.yml`) with the endpoint coverage gate.

---

## 6. E2E

- Real ingestion (Airbyte → Argo Workflows → bronze → API), the umbrella-chart deployment, and UI flows (Playwright,
  role + accessible-name locators — no accessibility or contrast checking).
- The **deployment smoke** (chart installs + rollout) runs post-merge on `main`. Full ingestion + UI run in
  **Test**; a **shallow acceptance validation** runs in **Beta**.
- Every user-facing surface **should** have at least one smoke assertion.
- A separate **compose-stand suite** (`tests/stand`, documented in `tests/stand/README.md`) drives a real Keycloak
  login and a set of browser journeys against the SPA, plus an API-contract suite — all against a local
  `docker-compose` stand seeded deterministically for tests (`src/ingestion/tools/seed`). Run it with
  `./dev-compose.sh test-stand up|test|down`. It asserts no metric VALUE against a declared expectation: the seed
  publishes none, and the suite that does assert values is the data-path rig in §5. It does reconcile every
  metric's drilldown evidence against that metric's own served value, which needs no declared expectation.

**CI:** `functional-k3s.yml` — ephemeral k3d install, post-merge on pushes to `main` that touch the deploy surface.
Today it deploys published images and asserts the edge answers; a real smoke must also assert `/health` + a few
golden metrics.

**CI:** `e2e-stand.yml` — two **non-required** checks against the compose-stand suite: `api-smoke` (the HTTP
contract tests, no browser) and `ui-journeys` (the browser journeys, run host-side from the checkout).
Neither blocks merge — both stand up a full stack against a live IdP and their flake rate is still unmeasured.

---

## 7. Performance

- Query latency (p50/p95/p99), load, stress, soak/endurance.
- **Not** on PR — runs in **Test** (baselines / nightly) and **Beta** (prod-load + soak). Requires the metrics stack.
- **Fixture sizes** — which organisation sizes these run against, and how much data each holds per
  metric class: [`REFERENCE-ORGS.md`](REFERENCE-ORGS.md).

---

## 8. Before you open a PR

- [ ] `cargo test` / `pytest` green for touched components; `cd src/frontend && pnpm test:coverage:ci` for the frontend
- [ ] `cargo fmt --check` + `cargo clippy --all-targets` clean
- [ ] the affected `tests/datapath` shard green if you touched a metric, a gold view, identity resolution or the API
- [ ] new / changed code stays **≥ 80 %** covered
- [ ] a new `metric_key` is value-tested or skip-listed (metric-coverage gate)
- [ ] committed OpenAPI regenerated if the router changed (`python3 scripts/ci/openapi_spec.py update`)

---

## 9. Related

- `tests/datapath/` — the data-path suite (metric specs, identity lane) and `tests/lib/insight_datapath/`
- `tests/stand/README.md` — the compose-stand suite (API contract, UI journeys, metrics)
