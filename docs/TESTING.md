# Testing Strategy

Insight is tested **shift-left**: contributors run the majority of checks locally before opening a PR. The strategy has
two axes — **levels** (what a test proves) and **environments** (where it runs). Several tests can share a level.

Entry points: `src/ingestion/tests/e2e/e2e.sh` (data-path suite), `scripts/ci/*` (coverage + spec gates), and the
standard per-language tools (`cargo`, `dotnet`, `pytest`).

---

## 1. Test pyramid

| Level | Scope | Tooling | Runs in |
|---|---|---|---|
| **Unit** | One function / module in isolation | `cargo test`, `dotnet test`, `pytest`, `vitest` | CI (every PR) |
| **Integration** | Components against real stores; the API contract | Testcontainers · dbt tests · OpenAPI-drift + metric-coverage · API & metric rig | CI (every PR) |
| **E2E** | The whole system through its real surfaces | ingestion (Airbyte → Argo) · chart install + smoke · UI (Playwright + axe) | CI (smoke) · Test (full) · Beta (shallow) |
| **Performance** | It stays fast under load | latency p50/95/99 · load · stress · soak | Test · Beta |

**Push tests down** — write each check at the lowest level that gives confidence; higher levels exist only for what
lower ones can't cover.

```sh
# fast local loop
cd src/backend && cargo test                        # Rust unit + integration
cd src/backend/services/identity && dotnet test     # .NET
cd src/ingestion/tests/e2e && ./e2e.sh build && ./e2e.sh test   # API & metric suite
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

---

## 4. Unit

- Every new public function / behaviour **must** have at least one unit test.
- Pure logic **must not** reach for a DB or the network — that belongs in Integration.

```sh
cd src/backend && cargo test                                 # Rust
cd src/backend/services/identity && dotnet test              # .NET
cd src/ingestion/connectors/<domain>/<name> && pytest        # Python connector
cargo fmt --check && cargo clippy --all-targets              # lint
```

**CI:** `ci.yml` — fmt + clippy + coverage, per changed component.

---

## 5. Integration

Components against a real store, and the API contract:

- **Testcontainers** — .NET Identity against a real MariaDB.
- **dbt data tests** — bronze → silver → gold model assertions.
- **Contract** — OpenAPI-drift + metric-coverage gates (every served `metric_key` is value-asserted or skip-listed).
- **API & metric tests** — the `bronze-to-api` rig: seed bronze → dbt → CH gold-view → analytics-api HTTP == expected value.

> The `bronze-to-api` rig is **Integration, not E2E** — it seeds bronze directly (no orchestrators) and asserts at the
> API (no UI). The workflow file `e2e-bronze-to-api.yml` is a legacy misnomer.

```sh
cd src/ingestion/tests/e2e
./e2e.sh test      # runs the rig + dbt
./e2e.sh gates     # metric-coverage + openapi-drift
```

**CI:** `e2e-bronze-to-api.yml` — blocking metric-coverage + openapi-drift gates.

---

## 6. E2E

- Real ingestion (Airbyte → Argo/Kestra → bronze → API), the umbrella-chart deployment, and UI flows (Playwright + axe).
- On PR, only the **deployment smoke** runs (chart installs + rollout). Full ingestion + UI run in **Test**; a
  **shallow acceptance validation** runs in **Beta**.
- Every user-facing surface **should** have at least one smoke assertion.

**CI:** `functional-k3s.yml` — ephemeral k3d install. Today it only *installs*; a real smoke must build + import the
PR's images and assert `/health` + a few golden metrics.

---

## 7. Performance

- Query latency (p50/p95/p99), load, stress, soak/endurance.
- **Not** on PR — runs in **Test** (baselines / nightly) and **Beta** (prod-load + soak). Requires the metrics stack.

---

## 8. Before you open a PR

- [ ] `cargo test` / `dotnet test` / `pytest` green for touched components
- [ ] `cargo fmt --check` + `cargo clippy --all-targets` clean
- [ ] `./e2e.sh test` green if you touched a metric, gold-view, or the API
- [ ] new / changed code stays **≥ 80 %** covered
- [ ] a new `metric_key` is value-tested or skip-listed (metric-coverage gate)
- [ ] committed OpenAPI regenerated if the router changed (`python3 scripts/ci/openapi_spec.py update`)

---

## 9. Related

- `src/ingestion/tests/e2e/README.md` — the API & metric rig
- `docs/domain/bronze-to-api-e2e/specs/` — PRD / DESIGN for the bronze-to-api rig
