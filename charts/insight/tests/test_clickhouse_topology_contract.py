"""One decision, one spelling per layer.

Ten tasks of the cluster-mode epic reach for "the cluster flag". This asserts
there is exactly one, that it defaults to a standalone single node in every
consumer, and that flipping the two chart values reaches each of them — a
consumer that silently keeps the standalone spelling builds unreplicated
relations on a cluster, which is the failure the epic exists to prevent.

Run: pytest charts/insight/tests
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]

#: (template, the manifest keys in it that must carry the topology). A template
#: appears once per consumer it renders -- `secrets.yaml` holds one Secret per
#: service, and each service reads the pair under its own gear name.
CONSUMERS = [
    ("templates/platform-config.yaml", ("CLICKHOUSE_CLUSTER_MODE", "CLICKHOUSE_CLUSTER_NAME")),
    ("templates/clickhouse-databases-job.yaml", ("CLICKHOUSE_CLUSTER_MODE", "CLICKHOUSE_CLUSTER_NAME")),
    ("templates/clickhouse-migrate-job.yaml", ("CLICKHOUSE_CLUSTER_MODE", "CLICKHOUSE_CLUSTER_NAME")),
    (
        "templates/ingestion/reconcile-cron.yaml",
        ("RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE", "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME"),
    ),
    (
        "templates/secrets.yaml",
        (
            "APP__gears__insight_v3_core__config__clickhouse_cluster_mode",
            "APP__gears__insight_v3_core__config__clickhouse_cluster_name",
        ),
    ),
    (
        "templates/secrets.yaml",
        (
            "APP__gears__identity_resolution__config__clickhouse_cluster_mode",
            "APP__gears__identity_resolution__config__clickhouse_cluster_name",
        ),
    ),
]

PLATFORM_CONFIG_KEYS = ("CLICKHOUSE_CLUSTER_MODE", "CLICKHOUSE_CLUSTER_NAME")


def _consumer_id(value: str | tuple[str, ...]) -> str:
    """A case is named by its template and the first key it looks for: one template
    renders several consumers, so the template alone would name two cases alike."""
    return value if isinstance(value, str) else value[0]


#: The v3-core Secret only renders for an install that deploys the service.
BASE = ["--set", "global.insightV3Core.deploy=true"]
CLUSTERED = [*BASE, "--set", "clickhouse.clusterMode=true", "--set", "clickhouse.clusterName=insight_cluster"]


@pytest.fixture(scope="session")
def chart_dependencies() -> None:
    result = subprocess.run(
        ["helm", "dependency", "update", str(CHART)], capture_output=True, text=True, timeout=300, check=False
    )
    assert result.returncode == 0, result.stderr


def _template(template: str, overrides: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "helm",
            "template",
            "contract-test",
            str(CHART),
            "--values",
            str(CHART / "tests" / "values.yaml"),
            *overrides,
            "--show-only",
            template,
        ],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )


def _render(template: str, overrides: list[str]) -> str:
    result = _template(template, overrides)
    assert result.returncode == 0, result.stderr
    return result.stdout


def _values(rendered: str, names: tuple[str, ...]) -> dict[str, str]:
    """The value each name carries, whether it is a ConfigMap/Secret entry or a
    container env var — the two shapes these templates use."""
    found: dict[str, str] = {}
    for document in yaml.safe_load_all(rendered):
        if not document:
            continue
        for key, value in (document.get("data") or document.get("stringData") or {}).items():
            if key in names:
                found[key] = value
        found.update(_env_of(document, names))

    return found


def _env_of(node: object, names: tuple[str, ...]) -> dict[str, str]:
    """Env entries anywhere in the manifest — a Job nests its pod one level
    deeper than a Deployment, and a CronJob one deeper again."""
    if isinstance(node, list):
        return {name: value for item in node for name, value in _env_of(item, names).items()}
    if not isinstance(node, dict):
        return {}

    found = {
        entry["name"]: entry.get("value", "")
        for entry in node.get("env", [])
        if isinstance(entry, dict) and entry.get("name") in names
    }
    for value in node.values():
        found.update(_env_of(value, names))

    return found


@pytest.mark.parametrize(("template", "names"), CONSUMERS, ids=_consumer_id)
def test_an_install_that_says_nothing_is_a_standalone_single_node(
    chart_dependencies: None, template: str, names: tuple[str, ...]
) -> None:
    found = _values(_render(template, BASE), names)

    mode, name = names
    assert found == {mode: "false", name: ""}, f"{template} must default to standalone"


@pytest.mark.parametrize(("template", "names"), CONSUMERS, ids=_consumer_id)
def test_every_consumer_hears_the_same_cluster(chart_dependencies: None, template: str, names: tuple[str, ...]) -> None:
    found = _values(_render(template, CLUSTERED), names)

    mode, name = names
    assert found == {mode: "true", name: "insight_cluster"}, f"{template} missed the topology"


@pytest.mark.parametrize(
    "name", ["", "   ", "has space", "1leading", "insight-cluster", "insight.cluster", 'a"; DROP TABLE x']
)
def test_a_cluster_mode_install_needs_a_name_shaped_like_an_identifier(chart_dependencies: None, name: str) -> None:
    """Without one, every creator gets replicated engines and no clause to qualify
    them, so their DDL reaches one node and leaves the replicas bare. The shape is
    checked with it: the name is interpolated into DDL unquoted, so a `-` or a `.`
    in it is a syntax error in every statement it reaches, not a wrong cluster."""
    result = _template(
        "templates/platform-config.yaml",
        [*BASE, "--set", "clickhouse.clusterMode=true", "--set", f"clickhouse.clusterName={name}"],
    )

    assert result.returncode != 0
    assert "must name the cluster" in result.stderr


@pytest.mark.parametrize("name", ["insight_cluster", "_c1", "Cluster2"])
def test_a_cluster_may_be_named_anything_clickhouse_reads_as_an_identifier(chart_dependencies: None, name: str) -> None:
    """The rule above must not reject a name an operator can actually configure."""
    found = _values(
        _render(
            "templates/platform-config.yaml",
            [*BASE, "--set", "clickhouse.clusterMode=true", "--set", f"clickhouse.clusterName={name}"],
        ),
        PLATFORM_CONFIG_KEYS,
    )

    assert found == {"CLICKHOUSE_CLUSTER_MODE": "true", "CLICKHOUSE_CLUSTER_NAME": name}


@pytest.mark.parametrize(
    "template",
    [
        "templates/ingestion/dbt-run.yaml",
        "templates/ingestion/connector-checks.yaml",
        "templates/ingestion/data-quality-test.yaml",
    ],
)
def test_every_dbt_step_reads_the_topology_from_the_platform_config_map(
    chart_dependencies: None, template: str
) -> None:
    """A dbt step writes its own profile and reads the project's cluster vars
    from the environment, so it has to be handed the same pair the rest of
    the release got — by reference, not by a second copy of the decision."""
    rendered = _render(template, CLUSTERED)

    for name in ("CLICKHOUSE_CLUSTER_MODE", "CLICKHOUSE_CLUSTER_NAME"):
        assert f"key: {name}" in rendered, f"{template} does not read {name}"


#: MariaDB has its own init Job, its own creator and no cluster fork.
MARIADB_TEMPLATE = "mariadb-init-svcdbs-job.yaml"

HELM_COMMENT = re.compile(r"\{\{-?\s*/\*.*?\*/\s*-?\}\}", re.DOTALL)


def test_no_chart_template_creates_a_clickhouse_database() -> None:
    """`create-databases.sh` is the one site that creates one (#3556), and the
    cluster mechanism needs it to stay that way: a `Replicated` database engine
    belongs on that statement, and a template that made a plain database first
    would win on a fresh cluster — silently, since everything is IF NOT EXISTS.

    Read with the Helm comment blocks stripped: these templates explain the
    rule in prose, and a guard matching its own documentation fires on every
    mention."""
    offenders = {
        path.name: [line.strip() for line in body.splitlines() if "CREATE DATABASE" in line]
        for path in sorted((CHART / "templates").glob("*.yaml"))
        if path.name != MARIADB_TEMPLATE
        and "CREATE DATABASE" in (body := HELM_COMMENT.sub("", path.read_text(encoding="utf-8")))
    }

    assert not offenders, f"a chart template creates a ClickHouse database: {offenders}"
