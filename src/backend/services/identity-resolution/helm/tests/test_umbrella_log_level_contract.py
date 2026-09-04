"""One log-level knob across the install (insight#2488, AC-2).

The umbrella publishes `global.observability.logs.level` twice: as
INSIGHT_LOG_LEVEL in the platform ConfigMap and as `logging.default.
console_level` in every gears service's rendered config. These tests hold the
two surfaces to the same value, so no service can grow a level knob of its
own without failing here.
"""

from __future__ import annotations

import re

from conftest import TENANT, UMBRELLA, UMBRELLA_BASE, render

GEARS_SERVICES = 5  # identity-resolution, analytics, authenticator, previews, git-cli-proxy


def rendered_console_levels(stdout: str) -> list[str]:
    return re.findall(r"console_level:\s*(\S+)", stdout)


def rendered_platform_level(stdout: str) -> str:
    match = re.search(r'INSIGHT_LOG_LEVEL:\s*"?(\w+)"?', stdout)
    assert match, "the platform ConfigMap should publish INSIGHT_LOG_LEVEL"
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

    levels = rendered_console_levels(out)
    assert len(levels) >= GEARS_SERVICES, (
        f"expected a console_level from each of the {GEARS_SERVICES} gears services, found {len(levels)}"
    )
    assert set(levels) == {"debug"}, f"a service kept a level of its own: {levels}"
    assert rendered_platform_level(out) == "debug"


def test_the_default_is_info_on_both_surfaces(umbrella_deps) -> None:
    code, out, err = render(UMBRELLA, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert code == 0, err

    assert set(rendered_console_levels(out)) == {"info"}
    assert rendered_platform_level(out) == "info"
