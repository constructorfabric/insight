"""Every gears deployment sets INSIGHT_SERVICE_VERSION to the image tag it runs."""

from __future__ import annotations

import re

from conftest import TENANT, UMBRELLA, UMBRELLA_BASE, render
from test_umbrella_log_level_contract import GEARS_SERVICES

VERSION_ENV = re.compile(r'name: INSIGHT_SERVICE_VERSION\s*\n\s*value: "([^"]*)"')
IMAGE_TAG = re.compile(r'image: "[^"@:]+:([^"]+)"')


def rendered_deployment_identity(stdout: str) -> dict[str, tuple[list[str], list[str]]]:
    per_service: dict[str, tuple[list[str], list[str]]] = {}
    for doc in stdout.split("\n---\n"):
        source = re.search(r"# Source: insight/charts/([^/]+)/templates/deployment\.yaml", doc)
        if not source:
            continue
        per_service[source.group(1)] = (VERSION_ENV.findall(doc), IMAGE_TAG.findall(doc))
    return per_service


def test_every_gears_deployment_stamps_its_image_tag_as_the_log_version(umbrella_deps) -> None:
    code, out, err = render(UMBRELLA, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert code == 0, err

    per_service = rendered_deployment_identity(out)
    assert GEARS_SERVICES <= set(per_service), (
        f"gears deployments rendered: {sorted(per_service)}, expected at least {sorted(GEARS_SERVICES)}"
    )
    for service in GEARS_SERVICES:
        versions, image_tags = per_service[service]
        assert len(versions) == 1, f"{service} sets INSIGHT_SERVICE_VERSION {len(versions)} times"
        assert versions[0], f"{service} sets an empty INSIGHT_SERVICE_VERSION"
        assert versions[0] in image_tags, (
            f"{service}: INSIGHT_SERVICE_VERSION {versions[0]!r} names no image the pod runs {image_tags}"
        )
