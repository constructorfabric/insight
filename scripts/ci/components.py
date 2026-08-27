#!/usr/bin/env python3
"""Insight coverage component registry — the single source of truth shared by
`coverage.py` (processes reports → per-component gate) and `changed.py` (emits
the CI matrix). Pure data + lookup: no CLI, no side effects, never runs tests.

Per component: name, lang, root (collection cwd), paths (repo-relative prefixes
for bucketing), plus per-language extras consumed by the CI producer jobs:
  rust   -> package (cargo package name); all_features (default True)
  python -> cov_package (the source_* package to measure)
  js     -> none (the package.json scripts under `root` carry the collection)

Nocode (declarative-YAML) connectors are excluded — no first-party code to
line-cover.
"""

from __future__ import annotations

from pathlib import Path

# Repo root: this file is <repo>/scripts/ci/components.py, so root is three up.
ROOT = Path(__file__).parent.parent.parent.absolute()

# Base branch for the diff-cover patch gate and the changed-component matrix.
COMPARE_BRANCH = "origin/main"

COMPONENTS = [
    # Rust: `cargo llvm-cov --package <package>` run in <root>. Each --package
    # report includes cross-crate files and the gate merges all reports (max
    # hits/line), so a lib's coverage reflects tests in other crates too, not
    # just its own.
    {
        "name": "insight-clickhouse",
        "lang": "rust",
        "root": "src/backend",
        "package": "insight-clickhouse",
        "paths": ["src/backend/libs/insight-clickhouse"],
    },
    # cover=False (mirrors identity-resolution): the crate is an I/O shell over
    # a `MariaDB` session — connect, `GET_LOCK`, the ledger read, the schema
    # probes — exercised by the env-gated live tests in the two services that
    # depend on it, which skip cleanly in CI, so only a handful of pure-logic
    # lines would count and the crate would gate far below the 80% line. fmt +
    # clippy + tests still run and gate the pipeline. Re-enable coverage when
    # the crate carries its own MariaDB-backed suite.
    {
        "name": "insight-migration",
        "lang": "rust",
        "root": "src/backend",
        "package": "insight-migration",
        "cover": False,
        "paths": ["src/backend/libs/insight-migration"],
    },
    {
        "name": "analytics",
        "lang": "rust",
        "root": "src/backend",
        "package": "analytics",
        # DB-backed integration tests: the CI rust job provisions a MariaDB
        # service, runs `analytics migrate` once up front, then runs the
        # `#[ignore]`d live_tests (INTEGRATION_TESTS_MARIADB_URL). live_ch
        # additionally provisions a ClickHouse for the CH-gated live tests
        # (INTEGRATION_TESTS_CLICKHOUSE_URL — see cf/insight#1564).
        "live_db": True,
        "live_ch": True,
        # llvm-cov reports every instrumented file, including path-dependency
        # crates (insight-clickhouse) compiled into this binary. Those crates are
        # their OWN components with their own coverage jobs — counting them here
        # would let this service's report drag their number down to whatever this
        # service happens to exercise. Scope the report to this service's code.
        "cover_ignore_regex": "src/backend/libs/",
        "paths": ["src/backend/services/analytics"],
        "triggered_by": ["insight-migration"],
    },
    # cover=False (mirrors authenticator): the crate's business logic
    # is exercised by env-gated live tests (IDENTITY_TEST_* against a dev
    # MariaDB/ClickHouse) that skip cleanly in CI, so only the pure-logic unit
    # tests would count — gating the crate far below the 80% line. fmt + clippy
    # + tests still run and gate the pipeline. Re-enable coverage when the
    # HTTP+MariaDB integration suite lands (#1753).
    {
        "name": "identity-resolution",
        "lang": "rust",
        "root": "src/backend",
        "package": "identity-resolution",
        "cover": False,
        # DB-backed migration/bootstrap tests: the CI rust job provisions a
        # MariaDB (database `identity` — this service owns that schema), runs
        # `identity-resolution migrate` up front, then `cargo test` with
        # INTEGRATION_TESTS_MARIADB_URL set (the live tests re-run the
        # migrator to prove idempotency and skip cleanly when unset).
        "live_db": True,
        "live_db_name": "identity",
        "cover_ignore_regex": "src/backend/libs/",
        "paths": ["src/backend/services/identity-resolution"],
        # insight-clickhouse is compiled in as a path dependency: a lib change
        # must re-run this crate's tests too. A shared path in `paths` would
        # NOT do that (component_for() picks a single owner — always the lib's
        # own component); `triggered_by` is the registry's co-trigger for this.
        "triggered_by": ["insight-clickhouse", "insight-migration"],
    },
    # git-cli-proxy shells out to the git CLI; its integration tests build
    # fixture repos with `git init` + file:// origins in tempdirs (hermetic —
    # the CI runner's git suffices, no service container).
    {
        "name": "git-cli-proxy",
        "lang": "rust",
        "root": "src/backend",
        "package": "git-cli-proxy",
        "paths": ["src/backend/services/git-cli-proxy"],
    },
    # routegen is the build-time gateway config compiler (gateway DESIGN
    # DD-GW-02); fmt + clippy + coverage run here. Golden + rejection tests cover
    # the emitter/validator; tests/cli.rs drives the built binary end to end
    # (output, --check, failure paths) so main.rs is covered too. The gateway.yml
    # workflow additionally runs nginx -t on the emitted config.
    {
        "name": "routegen",
        "lang": "rust",
        "root": "src/backend",
        "package": "routegen",
        "paths": ["src/backend/tools/routegen"],
    },
    # cover=False: the authenticator's security-critical
    # flow (OIDC login, sessions, cookie->JWT exchange) is proven by the e2e
    # login-loop, which drives the server as a SEPARATE process — so it can't
    # feed `cargo llvm-cov` (that instruments the test binary, not a spawned
    # server). Only the pure-logic unit tests (cookie/jwt/cache-control/config)
    # would count, gating the crate far below the 80% line. Tests + lint still
    # run and gate the pipeline. Re-enable coverage when in-process integration
    # tests (axum router + a testcontainer Redis) land.
    {
        "name": "authenticator",
        "lang": "rust",
        "root": "src/backend",
        "package": "authenticator",
        "cover": False,
        # Linked dependency crates (authenticator-sdk, workspace libs/plugins)
        # self-report in their own jobs; scope this component to its own code.
        "cover_ignore_regex": "src/backend/(libs|plugins)/",
        "paths": ["src/backend/services/authenticator"],
    },
    # authenticator-sdk is the inter-gear contract crate (a trait + models, no
    # runtime logic to exercise); lint + build only.
    {
        "name": "authenticator-sdk",
        "lang": "rust",
        "root": "src/backend",
        "package": "authenticator-sdk",
        "cover": False,
        "paths": ["src/backend/libs/authenticator-sdk"],
    },
    # jira-enrich is a standalone workspace; its `io` feature needs a live
    # ClickHouse, so cover with default features only (core tests are io-free).
    # clippy: False — jira-enrich's strict [lints.clippy] (pedantic/unwrap_used/…)
    # was never CI-enforced and the code violates it extensively. Clippy is
    # silenced here until the debt is cleared; re-enable per #1512. fmt + coverage
    # still run.
    {
        "name": "jira-enrich",
        "lang": "rust",
        "root": "src/ingestion/connectors/task-tracking/jira/enrich",
        "package": "jira-enrich",
        "all_features": False,
        "clippy": False,
        "paths": ["src/ingestion/connectors/task-tracking/jira/enrich"],
    },
    # Python CDK connectors
    {
        "name": "gitlab",
        "lang": "python",
        "root": "src/ingestion/connectors/git/gitlab",
        "cov_package": "source_gitlab",
        "paths": ["src/ingestion/connectors/git/gitlab"],
    },
    {
        "name": "hubspot",
        "lang": "python",
        "root": "src/ingestion/connectors/crm/hubspot",
        "cov_package": "source_hubspot",
        "paths": ["src/ingestion/connectors/crm/hubspot"],
    },
    {
        "name": "bamboohr",
        "lang": "python",
        "root": "src/ingestion/connectors/hr-directory/bamboohr",
        "cov_package": "source_bamboohr",
        "paths": ["src/ingestion/connectors/hr-directory/bamboohr"],
    },
    {
        "name": "claude-team-invoices",
        "lang": "python",
        "root": "src/ingestion/connectors/ai/claude-team-invoices",
        "cov_package": "source_claude_team_invoices",
        "paths": ["src/ingestion/connectors/ai/claude-team-invoices"],
    },
    # Deploy-time ClickHouse schema tooling (the migration Job's Python half:
    # reconcile_bronze_schema, which heals warm-cluster bronze drift — #1991).
    # Owning the whole scripts/ tree means a connectors-ddl snapshot regen also
    # re-runs these tests, which is the point: the reconciler's contract is with
    # that snapshot. Shell scripts in the same tree have no measured lines.
    {
        "name": "ingestion-scripts",
        "lang": "python",
        "root": "src/ingestion/scripts",
        "cov_package": "reconcile_bronze_schema",
        "paths": ["src/ingestion/scripts"],
    },
    # Mock-server test rig for NOCODE connectors (feature-connector-mock-tests),
    # split into two CI jobs for clean results (review ask): the harness's own
    # unit tests (meta/) and the per-connector mock suites. Both measure the
    # connector_tests package; the harness component owns its paths, so at the
    # gate both jobs' Cobertura reports merge into connector-tests-harness
    # (max hits per line) while connector-mock-tests gates on test results only.
    # `triggered_by` keeps them mutually in the matrix: a harness change must
    # re-run the suites that consume it, and a suite change alone must not
    # leave the merged coverage judged without the meta tests' share.
    # Line coverage measures the harness — declarative YAML manifests have no
    # first-party lines; a connector's behavioral coverage is the spec's stream
    # matrix. Longest-prefix match keeps nested components (jira-enrich) apart.
    {
        "name": "connector-tests-harness",
        "lang": "python",
        "root": "src/ingestion/tests/connectors",
        "cov_package": "connector_tests",
        "pytest_args": "--meta-only",
        "triggered_by": ["connector-mock-tests"],
        "paths": ["src/ingestion/tests/connectors"],
    },
    # cover=False (mirrors the rust flag): the suites job still runs and
    # uploads its Cobertura — those lines merge into connector-tests-harness at
    # the gate — but every file it measures lives under the harness paths, so
    # this component itself never has measured lines and must not be in the
    # gate's --require set (it would always look like a missing report).
    {
        "name": "connector-mock-tests",
        "lang": "python",
        "root": "src/ingestion/tests/connectors",
        "cov_package": "connector_tests",
        "pytest_args": "--suites-only",
        "cover": False,
        "triggered_by": ["connector-tests-harness"],
        "paths": [
            "src/ingestion/connectors/task-tracking/jira",
            "src/ingestion/connectors/git/github",
            "src/ingestion/connectors/git/bitbucket-cloud",
        ],
    },
    # `src/frontend/helm` falls under this path but has no measured lines, so it
    # never moves the number.
    {"name": "frontend", "lang": "js", "root": "src/frontend", "paths": ["src/frontend"]},
]


def component_for(rel_path: str, components: list[dict] = COMPONENTS) -> str | None:
    """Return the name of the component owning rel_path (longest-prefix match),
    so a nested path attaches to the most specific component, or None."""
    best, best_len = None, -1
    for comp in components:
        for p in comp["paths"]:
            p = p.rstrip("/")
            if (rel_path == p or rel_path.startswith(p + "/")) and len(p) > best_len:
                best, best_len = comp["name"], len(p)
    return best
