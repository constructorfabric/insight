from typing import Any

import pytest
from insight_datapath.dbt_graph import from_manifest
from insight_datapath.leg_selection import (
    Full,
    Partial,
    Reach,
    Verdict,
    combine,
    plan_legs,
    reach,
    verdict_for,
)
from insight_datapath.select_legs import outputs
from insight_datapath.suite_scan import Suites

CONNECTORS = "../connectors"
M365 = f"{CONNECTORS}/collaboration/m365"
GITHUB = f"{CONNECTORS}/git/github"

SOURCES = {
    "source.ingestion.bronze_m365.email": ("bronze_m365", "email", f"{M365}/dbt/schema.yml"),
    "source.ingestion.bronze_github.commits": (
        "bronze_github",
        "commits",
        f"{GITHUB}/dbt/schema.yml",
    ),
    "source.ingestion.config.roles": ("config", "roles", "../silver/task-tracking/schema.yml"),
}

M365_EMAIL = "source.ingestion.bronze_m365.email"
GITHUB_COMMITS = "source.ingestion.bronze_github.commits"
UNION = ["macro.ingestion.union_by_tag"]

MODELS: dict[str, tuple[str, list[str], list[str], list[str]]] = {
    # id: (file, parents, macros, tags)
    "model.ingestion.m365__email": (f"{M365}/dbt/m365__email.sql", [M365_EMAIL], [], []),
    "model.ingestion.class_collab_email": (
        "../silver/collaboration/class_collab_email.sql",
        ["model.ingestion.m365__email"],
        UNION,
        [],
    ),
    "model.ingestion.m365__identity": (f"{M365}/dbt/m365__identity.sql", [M365_EMAIL], [], []),
    "model.ingestion.github__identity": (
        f"{GITHUB}/dbt/github__identity.sql",
        [GITHUB_COMMITS],
        [],
        [],
    ),
    "model.ingestion.identity_inputs": (
        "identity/identity_inputs.sql",
        ["model.ingestion.m365__identity", "model.ingestion.github__identity"],
        UNION,
        [],
    ),
    "model.ingestion.github__commits": (
        f"{GITHUB}/dbt/github__commits.sql",
        [GITHUB_COMMITS],
        ["macro.ingestion.dedup"],
        [],
    ),
    "model.ingestion.git_view": (
        "../gold/git_view.sql",
        ["model.ingestion.github__commits"],
        [],
        ["gold"],
    ),
    "model.ingestion.roles_current": (
        "../silver/task-tracking/roles_current.sql",
        ["source.ingestion.config.roles"],
        [],
        [],
    ),
    "model.ingestion.account_map": ("identity/account_map.sql", [], [], []),
    "operation.ingestion.on-run-start": (
        "dbt_project.yml",
        [],
        ["macro.ingestion.create_config"],
        [],
    ),
}

MACROS = {
    "macro.ingestion.dedup": ("macros/dedup.sql", ["macro.ingestion.quote"]),
    "macro.ingestion.quote": ("macros/quote.sql", []),
    "macro.ingestion.union_by_tag": ("macros/union_by_tag.sql", []),
    "macro.ingestion.unused": ("macros/unused.sql", []),
    "macro.ingestion.create_config": ("macros/create_config.sql", []),
}


def _manifest() -> dict[str, Any]:
    nodes = {
        node_id: {
            "original_file_path": path,
            "package_name": "ingestion",
            "resource_type": node_id.split(".", 1)[0],
            "depends_on": {"nodes": parents, "macros": macros},
            "tags": tags,
        }
        for node_id, (path, parents, macros, tags) in MODELS.items()
    }
    sources = {
        source_id: {
            "schema": schema,
            "name": name,
            "original_file_path": path,
            "package_name": "ingestion",
        }
        for source_id, (schema, name, path) in SOURCES.items()
    }
    macros = {
        macro_id: {
            "original_file_path": path,
            "package_name": "ingestion",
            "depends_on": {"macros": callees},
        }
        for macro_id, (path, callees) in MACROS.items()
    }
    parent_map = {node_id: parents for node_id, (_, parents, _, _) in MODELS.items()}
    child_map: dict[str, list[str]] = {source_id: [] for source_id in sources}
    for node_id, parents in parent_map.items():
        child_map.setdefault(node_id, [])
        for parent in parents:
            child_map.setdefault(parent, []).append(node_id)
    return {
        "metadata": {"project_name": "ingestion"},
        "nodes": nodes,
        "sources": sources,
        "macros": macros,
        "docs": {
            "doc.ingestion.overview": {
                "original_file_path": "docs/overview.md",
                "package_name": "ingestion",
            }
        },
        "disabled": {},
        "parent_map": parent_map,
        "child_map": child_map,
    }


SUITES = Suites(
    metric_classes=("ai", "collab", "git"),
    seeds={
        "ai": frozenset({("config", "roles")}),
        "collab": frozenset({("bronze_m365", "email")}),
        "git": frozenset({("bronze_github", "commits")}),
        "identity": frozenset({("bronze_github", "commits")}),
    },
    template_users={"tests/datapath/metrics/templates/m365.yaml": frozenset({"collab"})},
)


@pytest.fixture(scope="module")
def world() -> Reach:
    return reach(from_manifest(_manifest()), SUITES)


def _suites(*names: str) -> Partial:
    return Partial(frozenset(names))


@pytest.mark.parametrize(
    ("path", "expected"),
    [
        ("src/ingestion/connectors/collaboration/m365/dbt/m365__email.sql", _suites("collab")),
        ("src/ingestion/silver/collaboration/class_collab_email.sql", _suites("collab")),
        ("src/ingestion/connectors/git/github/dbt/github__commits.sql", _suites("git", "identity")),
        ("src/ingestion/gold/git_view.sql", _suites("git")),
        ("src/ingestion/silver/task-tracking/roles_current.sql", _suites("ai")),
        ("src/ingestion/dbt/identity/account_map.sql", _suites("ai", "collab", "git", "identity")),
        ("src/ingestion/dbt/identity/identity_inputs.sql", _suites("collab", "git", "identity")),
        (
            "src/ingestion/connectors/collaboration/m365/dbt/m365__identity.sql",
            _suites("collab", "git", "identity"),
        ),
        (
            "src/ingestion/connectors/collaboration/m365/dbt/schema.yml",
            _suites("collab", "git", "identity"),
        ),
        ("src/ingestion/dbt/macros/quote.sql", _suites("git", "identity")),
        ("src/ingestion/dbt/macros/union_by_tag.sql", _suites("collab", "git", "identity")),
        ("src/ingestion/dbt/docs/overview.md", _suites()),
        ("src/ingestion/silver/shared/README.md", _suites()),
        (
            "src/ingestion/connectors/collaboration/m365/connector.yaml",
            _suites("collab", "git", "identity"),
        ),
        (
            "src/ingestion/connectors/git/github/Dockerfile",
            _suites("collab", "git", "identity"),
        ),
        ("src/ingestion/connectors/quality/allure/connector.yaml", _suites()),
        ("src/ingestion/scripts/apply-ch-migrations.sh", _suites()),
        ("src/backend/services/insight-v3-core/src/api/alerts.rs", _suites()),
        ("src/backend/services/insight-v3-core/helm/Chart.yaml", _suites()),
        ("src/ingestion/tools/seed/pyproject.toml", _suites("identity")),
        ("tests/datapath/metrics/git/test_commits.py", _suites("git")),
        ("tests/datapath/metrics/schemas/bronze_github.commits.yaml", _suites("git", "identity")),
        ("tests/datapath/metrics/schemas/bronze_zoom.meetings.yaml", _suites()),
        ("tests/datapath/metrics/templates/m365.yaml", _suites("collab")),
        ("tests/datapath/identity/test_binding.py", _suites("identity")),
        ("tests/datapath/meta/test_topology.py", _suites()),
    ],
)
def test_a_change_reaches_the_suites_whose_seeded_data_crosses_it_or_its_union(
    world: Reach, path: str, expected: Verdict
) -> None:
    assert verdict_for(path, world) == expected, f"should map {path} to {expected}"


@pytest.mark.parametrize(
    "path",
    [
        "src/ingestion/dbt/macros/unused.sql",
        "src/ingestion/dbt/macros/create_config.sql",
        "src/ingestion/dbt/dbt_project.yml",
        "src/ingestion/connectors/git/github/dbt/github__deleted.sql",
        "src/ingestion/gold/deleted.sql",
        "src/ingestion/scripts/migrations/20990101000000_x.sql",
        "src/backend/services/analytics/src/main.rs",
        "tests/lib/insight_datapath/spec_runner.py",
        "tests/datapath/conftest.py",
        "tests/datapath/metrics/conftest.py",
        "tests/datapath/metrics/schemas/README.md",
        "tests/datapath/metrics/retired/x.test.yaml",
        "dev-compose.sh",
    ],
)
def test_a_change_no_rule_can_place_runs_every_leg(world: Reach, path: str) -> None:
    assert isinstance(verdict_for(path, world), Full), f"should run every leg for {path}"


def test_a_deleted_file_is_placed_by_what_it_defined_at_the_base(world: Reach) -> None:
    manifest = _manifest()
    del manifest["nodes"]["model.ingestion.m365__email"]
    head = reach(from_manifest(manifest), SUITES)
    path = "src/ingestion/connectors/collaboration/m365/dbt/m365__email.sql"

    assert isinstance(verdict_for(path, head), Full), "should run every leg without the base graph"
    assert verdict_for(path, head, previous=world) == _suites("collab")


def test_one_unplaceable_path_makes_the_whole_change_full(world: Reach) -> None:
    verdicts = [
        verdict_for(path, world) for path in ("tests/datapath/metrics/git/a.py", "dev-compose.sh")
    ]
    assert isinstance(combine(verdicts), Full)
    assert combine([_suites("git"), _suites("collab")]) == _suites("collab", "git")


@pytest.mark.parametrize(
    ("verdict", "legs"),
    [
        (
            Full("everything"),
            [
                ("ai", ("tests/datapath/metrics/ai",)),
                ("git", ("tests/datapath/metrics/git",)),
                ("tasks", ("tests/datapath/metrics/tasks",)),
                (
                    "rest",
                    (
                        "tests/datapath/metrics/ci",
                        "tests/datapath/metrics/wiki",
                        "tests/datapath/meta",
                    ),
                ),
                ("identity", ("tests/datapath/identity",)),
            ],
        ),
        (_suites("wiki"), [("rest", ("tests/datapath/metrics/wiki", "tests/datapath/meta"))]),
        (_suites(), [("rest", ("tests/datapath/meta",))]),
        (
            _suites("git", "identity"),
            [
                ("git", ("tests/datapath/metrics/git",)),
                ("rest", ("tests/datapath/meta",)),
                ("identity", ("tests/datapath/identity",)),
            ],
        ),
    ],
)
def test_a_plan_keeps_the_meta_tree_and_runs_only_the_classes_it_names(
    verdict: Verdict, legs: list[tuple[str, tuple[str, ...]]]
) -> None:
    planned = plan_legs(verdict, ("ai", "ci", "git", "tasks", "wiki"))
    assert [(leg.shard, leg.trees) for leg in planned] == legs, f"should plan {legs} for {verdict}"


@pytest.mark.parametrize(
    ("verdict", "scope", "classes"),
    [
        (Full("everything"), "full", "ai,ci,git"),
        (_suites("ci", "identity"), "partial", "ci"),
        (_suites("identity"), "none", ""),
    ],
)
def test_the_gate_owes_exactly_the_metric_classes_that_ran(
    verdict: Verdict, scope: str, classes: str
) -> None:
    known = ("ai", "ci", "git")
    planned = outputs(plan_legs(verdict, known), verdict, known)
    assert (planned["scope"], planned["classes"], planned["known"]) == (scope, classes, "ai,ci,git")
