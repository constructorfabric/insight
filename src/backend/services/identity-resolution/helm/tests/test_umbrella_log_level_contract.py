"""One log-level knob and one line format across the install (insight#2488,
AC-1 + AC-2).

The umbrella publishes `global.observability.logs.{level,format}` twice: as
INSIGHT_LOG_LEVEL / INSIGHT_LOG_FORMAT in the platform ConfigMap and as
`logging.default.{console_level,console_format}` in every gears service's
rendered config. These tests hold the two surfaces to the same value per
service, so no service can grow a knob or a line shape of its own — or drop
out of the contract — without failing here.
"""

from __future__ import annotations

import re

from conftest import TENANT, UMBRELLA, UMBRELLA_BASE, render

GEARS_SERVICES = {
    "analytics",
    "authenticator",
    "gitCliProxy",
    "identityResolution",
    "insightV3Core",
    "previews",
}

# Minimum count of platform-ConfigMap-sourced INSIGHT_LOG_LEVEL env entries per
# ingestion template; airbyte-sync carries one in each of its two step scripts.
INGESTION_TEMPLATES = {
    "insight/templates/ingestion/reconcile-cron.yaml": 1,
    "insight/templates/ingestion/dbt-run.yaml": 1,
    "insight/templates/ingestion/data-quality-test.yaml": 1,
    "insight/templates/ingestion/airbyte-sync.yaml": 2,
}

INGESTION_ENABLED = [
    "--set",
    "ingestion.templates.enabled=true",
    "--set",
    "ingestion.toolboxImage=ghcr.io/example/insight-toolbox:0.0.0-test",
]

# `render` templates the release as "contract-test", so the platform ConfigMap
# the env entries must reference is contract-test-platform.
PLATFORM_KEY_REF = re.compile(
    r"- name: INSIGHT_LOG_LEVEL\s*\n"
    r"\s*valueFrom:\s*\n"
    r"\s*configMapKeyRef:\s*\n"
    r"\s*name: contract-test-platform\s*\n"
    r"\s*key: INSIGHT_LOG_LEVEL"
)


def rendered_per_service(stdout: str, key: str) -> dict[str, list[str]]:
    """Map each gears subchart to the values of `key` its configmap renders."""
    per_service: dict[str, list[str]] = {}
    for doc in stdout.split("\n---\n"):
        source = re.search(r"# Source: insight/charts/([^/]+)/templates/configmap\.yaml", doc)
        values = re.findall(rf"{key}:\s*(\S+)", doc)
        if source and values:
            per_service.setdefault(source.group(1), []).extend(values)
    return per_service


def assert_every_service_renders_exactly(stdout: str, key: str, expected: str) -> None:
    per_service = rendered_per_service(stdout, key)
    assert set(per_service) == GEARS_SERVICES, (
        f"services rendering {key}: {sorted(per_service)}, expected {sorted(GEARS_SERVICES)}"
    )
    for service, values in per_service.items():
        assert values == [expected], f"{service} rendered {key}: {values}, expected [{expected!r}]"


def rendered_platform_level(stdout: str) -> str:
    match = re.search(r'INSIGHT_LOG_LEVEL:\s*"?(\w+)"?', stdout)
    assert match, "the platform ConfigMap should publish INSIGHT_LOG_LEVEL"
    return match.group(1)


def rendered_platform_format(stdout: str) -> str:
    match = re.search(r'INSIGHT_LOG_FORMAT:\s*"?(\w+)"?', stdout)
    assert match, "the platform ConfigMap should publish INSIGHT_LOG_FORMAT"
    return match.group(1)


def test_the_one_knob_reaches_every_service_config(umbrella_deps) -> None:
    code, out, err = render(
        UMBRELLA,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "global.observability.logs.level=debug",
    )
    assert code == 0, err

    assert_every_service_renders_exactly(out, "console_level", "debug")
    assert rendered_platform_level(out) == "debug"


def test_every_ingestion_workflow_step_reads_the_platform_knob(umbrella_deps) -> None:
    """The ingestion Python jobs log through the same knob as the gears
    services: each workflow template must source INSIGHT_LOG_LEVEL from the
    platform ConfigMap, not carry a level of its own (insight#3191)."""
    code, out, err = render(
        UMBRELLA,
        *UMBRELLA_BASE,
        *INGESTION_ENABLED,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "global.observability.logs.level=debug",
    )
    assert code == 0, err

    refs_per_template: dict[str, int] = dict.fromkeys(INGESTION_TEMPLATES, 0)
    for doc in out.split("\n---\n"):
        source = re.search(r"# Source: (insight/templates/ingestion/\S+)", doc)
        if source and source.group(1) in refs_per_template:
            refs_per_template[source.group(1)] += len(PLATFORM_KEY_REF.findall(doc))
    for template, minimum in INGESTION_TEMPLATES.items():
        assert refs_per_template[template] >= minimum, (
            f"{template} sources INSIGHT_LOG_LEVEL from the platform ConfigMap "
            f"{refs_per_template[template]} time(s), expected at least {minimum}"
        )


def test_the_default_is_info_on_both_surfaces(umbrella_deps) -> None:
    code, out, err = render(UMBRELLA, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert code == 0, err

    assert_every_service_renders_exactly(out, "console_level", "info")
    assert rendered_platform_level(out) == "info"


def test_the_one_format_reaches_every_service_config(umbrella_deps) -> None:
    code, out, err = render(
        UMBRELLA,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "global.observability.logs.format=text",
    )
    assert code == 0, err

    assert_every_service_renders_exactly(out, "console_format", "text")
    assert rendered_platform_format(out) == "text"


def test_the_default_format_is_json_on_both_surfaces(umbrella_deps) -> None:
    code, out, err = render(UMBRELLA, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert code == 0, err

    assert_every_service_renders_exactly(out, "console_format", "json")
    assert rendered_platform_format(out) == "json"


def test_a_legacy_top_level_format_override_cannot_split_the_surfaces(umbrella_deps) -> None:
    """The gears subcharts never see the top-level observability block, so the
    platform ConfigMap must not honour it for the format either."""
    code, out, err = render(
        UMBRELLA, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}", "--set", "observability.logs.format=text"
    )
    assert code == 0, err

    assert_every_service_renders_exactly(out, "console_format", "json")
    assert rendered_platform_format(out) == "json"
