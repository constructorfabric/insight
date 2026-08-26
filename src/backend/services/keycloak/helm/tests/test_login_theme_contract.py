"""Render contract for the `insight` login theme.

The theme stops one login dead end (insight#2721). Every assertion here
guards a way the change can fail SILENTLY — Keycloak starts, falls back to a
built-in theme, logs one line, and the dead end simply returns.
"""

from __future__ import annotations

from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import yaml
from theme_harness import (
    CANONICAL_REALMS,
    MOUNT_PATH,
    SUBCHART,
    SUBCHART_BASE,
    THEME_NAME,
    THEME_ROOT,
    THEME_SOURCE,
    UMBRELLA_BASE,
    assert_theme_payload,
    keycloak_pod,
    render,
    render_error,
    theme_configmap,
    theme_properties,
)


def test_theme_source_is_where_the_chart_looks_for_it() -> None:
    # `.Files.Get` is now guarded, but this names the file so a rename fails
    # with the reason rather than as a mismatched blob.
    assert THEME_SOURCE.is_file(), f"missing {THEME_SOURCE}"


def test_subchart_renders_a_usable_theme() -> None:
    assert_theme_payload(render(SUBCHART, *SUBCHART_BASE))


def test_return_url_is_configurable() -> None:
    docs = render(SUBCHART, *SUBCHART_BASE, "--set", "theme.returnUrl=https://app.test/")
    assert theme_properties(docs)["insightReturnUrl"] == "https://app.test/"


def test_shipped_default_carries_the_marker_the_spa_retries_on() -> None:
    # src/frontend/src/auth/auth-error.ts reads exactly `auth_error` and
    # auto-retries every code except access_denied. A typo here means no retry
    # is counted and the browser bounces back into the login it just failed.
    default = theme_properties(render(SUBCHART, *SUBCHART_BASE))["insightReturnUrl"]
    assert parse_qs(urlsplit(default).query)["auth_error"] == ["kc_page_expired"]


def test_return_url_cannot_smuggle_a_theme_property_expression() -> None:
    # Keycloak resolves ${...} in every theme property against the server's own
    # environment — which on this pod carries the database password — and serves
    # the result in the page. A newline would inject a second property. Both are
    # refused at render time rather than documented.
    for hostile in (
        "${env.KC_BOOTSTRAP_ADMIN_PASSWORD}",
        "/?next=${env.KC_DB_PASSWORD}",
        "/?a=b\nparent=base",
    ):
        message = render_error(
            SUBCHART, *SUBCHART_BASE, "--set-string", f"theme.returnUrl={hostile}"
        )
        assert "must be a literal URL" in message, hostile


def test_theme_mounts_where_keycloak_discovers_it() -> None:
    docs = render(SUBCHART, *SUBCHART_BASE)
    pod = keycloak_pod(docs)
    (container,) = pod["spec"]["containers"]

    mounts = {m["mountPath"]: m for m in container["volumeMounts"]}
    assert MOUNT_PATH in mounts, f"theme must mount at {MOUNT_PATH}, got {list(mounts)}"
    assert mounts[MOUNT_PATH]["readOnly"] is True

    # A mount at the theme root would hide the theme with no error anywhere.
    shadowing = [path for path in mounts if path != MOUNT_PATH and MOUNT_PATH.startswith(path)]
    assert not shadowing, f"{shadowing} would shadow {THEME_ROOT}"

    volume = next(v for v in pod["spec"]["volumes"] if v["name"] == mounts[MOUNT_PATH]["name"])
    assert volume["configMap"]["name"] == theme_configmap(docs)["metadata"]["name"]


def test_theme_change_rolls_the_pods() -> None:
    # Keycloak caches themes under `start`, so a checksum that does not track
    # the ConfigMap means an edited theme never reaches a running pod. Assert
    # the value moves, not merely that the key exists.
    def checksum(*extra: str) -> str:
        return keycloak_pod(render(SUBCHART, *SUBCHART_BASE, *extra))["metadata"][
            "annotations"
        ]["checksum/theme"]

    assert checksum() == checksum()
    assert checksum() != checksum("--set", "theme.returnUrl=https://other.test/")


def test_canonical_realms_agree_with_what_the_chart_ships() -> None:
    assert CANONICAL_REALMS, "no canonical broker realm found to check"

    bound = []
    for path in CANONICAL_REALMS:
        realm = yaml.safe_load(path.read_text())

        # error.ftl — the page the timeout path actually renders — offers a way
        # out only when the client names one. Pure realm config with no chart
        # dependency, so it holds for every realm.
        clients = [c for c in realm.get("clients", []) if c["clientId"] == "insight-authenticator"]
        assert clients, f"insight-authenticator missing from {path}"
        assert clients[0].get("baseUrl") == "/", path

        # Binding the theme is opt-in: an install whose Keycloak this chart does
        # not deploy must leave it unset. A realm that DOES bind it has to name
        # the theme that ships, or Keycloak silently falls back to the built-in.
        theme = realm.get("loginTheme")
        if theme is not None:
            assert theme == THEME_NAME, path
            bound.append(path)

    assert bound, "the chart ships a login theme no canonical realm binds"


def test_umbrella_ships_a_usable_theme(umbrella_deps: Path) -> None:
    # The umbrella is the unit of distribution, and the only render where
    # `.Files.Get` resolves against the PACKAGED subchart.
    assert_theme_payload(render(umbrella_deps, *UMBRELLA_BASE))

    docs = render(
        umbrella_deps, *UMBRELLA_BASE, "--set", "keycloak.theme.returnUrl=https://app.test/"
    )
    assert theme_properties(docs)["insightReturnUrl"] == "https://app.test/"


def test_subchart_and_umbrella_defaults_agree(umbrella_deps: Path) -> None:
    # Umbrella values win at render time, so a maintainer who fixes only the
    # subchart default ships nothing.
    subchart = theme_properties(render(SUBCHART, *SUBCHART_BASE))["insightReturnUrl"]
    umbrella = theme_properties(render(umbrella_deps, *UMBRELLA_BASE))["insightReturnUrl"]
    assert subchart == umbrella
