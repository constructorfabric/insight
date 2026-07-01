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
    # just its own. NB: api-gateway's cargo package is insight-api-gateway.
    {"name": "insight-clickhouse", "lang": "rust", "root": "src/backend",
     "package": "insight-clickhouse",
     "paths": ["src/backend/libs/insight-clickhouse"]},
    {"name": "oidc-authn-plugin", "lang": "rust", "root": "src/backend",
     "package": "oidc-authn-plugin",
     "paths": ["src/backend/plugins/oidc-authn-plugin"]},
    {"name": "analytics-api", "lang": "rust", "root": "src/backend",
     "package": "analytics-api",
     "paths": ["src/backend/services/analytics-api"]},
    {"name": "api-gateway", "lang": "rust", "root": "src/backend",
     "package": "insight-api-gateway",
     "paths": ["src/backend/services/api-gateway"]},
    # jira-enrich is a standalone workspace; its `io` feature needs a live
    # ClickHouse, so cover with default features only (core tests are io-free).
    # clippy: False — jira-enrich's strict [lints.clippy] (pedantic/unwrap_used/…)
    # was never CI-enforced and the code violates it extensively. Clippy is
    # silenced here until the debt is cleared; re-enable per #1512. fmt + coverage
    # still run.
    {"name": "jira-enrich", "lang": "rust",
     "root": "src/ingestion/connectors/task-tracking/jira/enrich",
     "package": "jira-enrich", "all_features": False, "clippy": False,
     "paths": ["src/ingestion/connectors/task-tracking/jira/enrich"]},

    # .NET
    {"name": "identity", "lang": "dotnet", "root": "src/backend/services/identity",
     "solution": "Insight.Identity.sln",
     "paths": ["src/backend/services/identity"]},

    # Python CDK connectors
    {"name": "gitlab", "lang": "python", "root": "src/ingestion/connectors/git/gitlab",
     "cov_package": "source_gitlab",
     "paths": ["src/ingestion/connectors/git/gitlab"]},
    {"name": "github", "lang": "python", "root": "src/ingestion/connectors/git/github",
     "cov_package": "source_github",
     "paths": ["src/ingestion/connectors/git/github"]},
    {"name": "github-v2", "lang": "python", "root": "src/ingestion/connectors/git/github-v2",
     "cov_package": "source_github_v2",
     "paths": ["src/ingestion/connectors/git/github-v2"]},
    {"name": "bitbucket-cloud", "lang": "python", "root": "src/ingestion/connectors/git/bitbucket-cloud",
     "cov_package": "source_bitbucket_cloud",
     "paths": ["src/ingestion/connectors/git/bitbucket-cloud"]},
    {"name": "hubspot", "lang": "python", "root": "src/ingestion/connectors/crm/hubspot",
     "cov_package": "source_hubspot",
     "paths": ["src/ingestion/connectors/crm/hubspot"]},
    {"name": "salesforce", "lang": "python", "root": "src/ingestion/connectors/crm/salesforce",
     "cov_package": "source_salesforce",
     "paths": ["src/ingestion/connectors/crm/salesforce"]},
    {"name": "github-copilot", "lang": "python", "root": "src/ingestion/connectors/ai/github-copilot",
     "cov_package": "source_github_copilot",
     "paths": ["src/ingestion/connectors/ai/github-copilot"]},
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
