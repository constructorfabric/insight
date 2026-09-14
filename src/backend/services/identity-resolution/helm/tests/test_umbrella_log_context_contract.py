"""Every gears configmap pins service.version to the image tag the pod runs."""

from __future__ import annotations

import re

from conftest import TENANT, UMBRELLA, UMBRELLA_BASE, render
from test_umbrella_log_level_contract import GEARS_SERVICES

VERSION_ATTR = re.compile(r'service\.version:\s*"([^"]*)"')
IMAGE_TAG = re.compile(r'image: "[^"]+:([^":/@]+)"')


def rendered_per_service(stdout: str, template: str, pattern: re.Pattern) -> dict[str, list[str]]:
    per_service: dict[str, list[str]] = {}
    for doc in stdout.split("\n---\n"):
        source = re.search(rf"# Source: insight/charts/([^/]+)/templates/{template}", doc)
        if source:
            per_service.setdefault(source.group(1), []).extend(pattern.findall(doc))
    return per_service


def test_every_gears_configmap_pins_the_image_tag_as_service_version(umbrella_deps) -> None:
    code, out, err = render(UMBRELLA, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert code == 0, err

    versions = rendered_per_service(out, r"configmap\.yaml", VERSION_ATTR)
    tags = rendered_per_service(out, r"deployment\.yaml", IMAGE_TAG)
    assert GEARS_SERVICES <= set(versions), (
        f"configmaps with service.version: {sorted(versions)}, expected at least {sorted(GEARS_SERVICES)}"
    )
    for service in GEARS_SERVICES:
        assert len(versions[service]) == 1, f"{service} renders service.version {len(versions[service])} times"
        version = versions[service][0]
        assert version, f"{service} renders an empty service.version"
        assert version in tags.get(service, []), (
            f"{service}: service.version {version!r} names no image the pod runs {tags.get(service)}"
        )
