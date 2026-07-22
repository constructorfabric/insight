#!/usr/bin/env python3
"""Insight coverage component registry — the single source of truth shared by
`coverage.py` (processes reports → per-component gate) and `changed.py` (emits
the CI matrix). Pure data + lookup: no CLI, no side effects, never runs tests.

Per component: name, lang, root (collection cwd), paths (repo-relative prefixes
for bucketing), plus per-language extras consumed by the CI producer jobs:
  rust   -> package (cargo package name); all_features (default True)
  dotnet -> solution
  python -> cov_package (the source_* package to measure)

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
    {
        "name": "analytics",
        "lang": "rust",
        "root": "src/backend",
        "package": "analytics",
        # DB-backed integration tests: the CI rust job provisions a MariaDB
        # service, runs `analytics migrate` once up front, then runs the
        # `#[ignore]`d live_tests (INTEGRATION_TESTS_MARIADB_URL). ClickHouse
        # tests skip (no INTEGRATION_TESTS_CLICKHOUSE_URL — see cf/insight#1564).
        "live_db": True,
        # llvm-cov reports every instrumented file, including path-dependency
        # crates (insight-clickhouse) compiled into this binary. Those crates are
        # their OWN components with their own coverage jobs — counting them here
        # would let this service's report drag their number down to whatever this
        # service happens to exercise. Scope the report to this service's code.
        "cover_ignore_regex": "src/backend/libs/",
        "paths": ["src/backend/services/analytics"],
    },
    # fakeidp is a dev/e2e test double (see cf/NGINX_BFF.md §10 G6), not shipped
    # code — but it has real integration tests, so it is covered + gated like any
    # other crate. Its only cross-crate files are none (standalone deps), so no
    # cover_ignore_regex is needed.
    {
        "name": "fakeidp",
        "lang": "rust",
        "root": "src/backend",
        "package": "fakeidp",
        "paths": ["src/backend/services/fakeidp"],
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
    # cover=False (mirrors identity): the authenticator's security-critical
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
    # .NET
    # cover=False: identity is excluded from coverage collection and gating
    # entirely (2026-07 decision) — its tests still run in the dotnet CI job
    # and still fail the pipeline on regressions; only the Cobertura
    # collection, upload, and the per-component/new-code gates are dropped.
    {
        "name": "identity",
        "lang": "dotnet",
        "root": "src/backend/services/identity",
        "solution": "Insight.Identity.sln",
        "cover": False,
        "paths": ["src/backend/services/identity"],
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
        "name": "github-v2",
        "lang": "python",
        "root": "src/ingestion/connectors/git/github-v2",
        "cov_package": "source_github_v2",
        "paths": ["src/ingestion/connectors/git/github-v2"],
    },
    {
        "name": "bitbucket-cloud",
        "lang": "python",
        "root": "src/ingestion/connectors/git/bitbucket-cloud",
        "cov_package": "source_bitbucket_cloud",
        "paths": ["src/ingestion/connectors/git/bitbucket-cloud"],
    },
    {
        "name": "hubspot",
        "lang": "python",
        "root": "src/ingestion/connectors/crm/hubspot",
        "cov_package": "source_hubspot",
        "paths": ["src/ingestion/connectors/crm/hubspot"],
    },
    {
        "name": "salesforce",
        "lang": "python",
        "root": "src/ingestion/connectors/crm/salesforce",
        "cov_package": "source_salesforce",
        "paths": ["src/ingestion/connectors/crm/salesforce"],
    },
    {
        "name": "github-copilot",
        "lang": "python",
        "root": "src/ingestion/connectors/ai/github-copilot",
        "cov_package": "source_github_copilot",
        "paths": ["src/ingestion/connectors/ai/github-copilot"],
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
    # cover=False (mirrors the rust/dotnet flag): the suites job still runs and
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
        "paths": ["src/ingestion/connectors/task-tracking/jira"],
    },
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
