"""Contract for `authenticator.oidc.resolveBy` across the umbrella's deploy surface.

The mode decides which internal identity route a login resolves through, so the
things pinned here are the ones an operator cannot discover by reading a pod log:
what the default renders as, and which values files the chart refuses outright
because they would deploy cleanly and then deny every sign-in.
"""

from __future__ import annotations

import yaml
from conftest import TENANT, UMBRELLA_BASE, render

ROSTER = ("--set", "identityResolution.rosterSourceType=bamboohr")


def _authenticator_secret(manifests: str) -> dict:
    docs = [doc for doc in yaml.safe_load_all(manifests) if isinstance(doc, dict)]
    secrets = [
        doc
        for doc in docs
        if doc.get("kind") == "Secret" and doc["metadata"]["name"] == "insight-authenticator-config"
    ]
    assert len(secrets) == 1, f"expected one authenticator config Secret, got {len(secrets)}"

    return secrets[0]["stringData"]


def test_logins_resolve_by_external_id_unless_an_install_says_otherwise(umbrella_deps) -> None:
    """The mode is declared, and its default is the behaviour that came before it.

    An upgrade must not change how anyone's sign-in resolves just by landing.
    """
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert code == 0, err

    secret = _authenticator_secret(out)
    assert secret["APP__gears__authenticator__config__idp__resolve_by"] == "external_id"
    assert secret["APP__gears__authenticator__config__idp__source_type"] == "ms-entra"


def test_email_mode_carries_the_mode_and_stops_demanding_a_source_type(umbrella_deps) -> None:
    """The email mode reads neither external-id knob, so it must not require one.

    Demanding a `sourceType` an install's login never consults leaves a value
    sitting in its values file that reads as if it were in force.
    """
    code, out, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        *ROSTER,
        "--set",
        "authenticator.oidc.resolveBy=email",
        "--set",
        "authenticator.oidc.sourceType=",
    )
    assert code == 0, err

    secret = _authenticator_secret(out)
    assert secret["APP__gears__authenticator__config__idp__resolve_by"] == "email"
    assert secret["APP__gears__authenticator__config__idp__source_type"] == ""


def test_email_mode_without_a_roster_is_refused_at_render(umbrella_deps) -> None:
    """The email lookup is confined to the roster, so no roster means no lookup.

    Identity refuses it rather than matching an address any source happened to
    state — which would deny every sign-in. Say so while the operator is still
    editing values, not after the install is up and nobody can get in.
    """
    code, _, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "authenticator.oidc.resolveBy=email",
    )
    assert code != 0, "render must fail"
    assert "rosterSourceType" in err


def test_an_unknown_resolve_mode_is_refused_at_render(umbrella_deps) -> None:
    """A typo must not silently fall back to a mode the install did not choose."""
    code, _, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        *ROSTER,
        "--set",
        "authenticator.oidc.resolveBy=e-mail",
    )
    assert code != 0, "render must fail"
    assert "resolveBy" in err


def test_external_id_mode_still_demands_a_source_type(umbrella_deps) -> None:
    """The guard this change moved out of secrets.yaml must still fire.

    It used to be enforced twice — a `required` at the point of use and a check
    in `insight.validate`. Only the second survives, so mistyping its condition
    would leave a chart that renders happily and an authenticator that refuses
    its own config at boot.
    """
    code, _, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "authenticator.oidc.sourceType=",
    )
    assert code != 0, "render must fail"
    assert "sourceType" in err


def test_email_mode_without_the_email_scope_is_refused(umbrella_deps) -> None:
    """The mode's only input is the claim that scope carries.

    An explicit scope list that omits it installs cleanly and denies every
    login; adding it back costs nothing, so this is refused while the operator
    is still editing values.
    """
    code, _, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        *ROSTER,
        "--set",
        "authenticator.oidc.resolveBy=email",
        "--set",
        "authenticator.oidc.scopes={openid,profile}",
    )
    assert code != 0, "render must fail"
    assert "scopes" in err


def test_email_mode_refuses_provisioning_rather_than_ignoring_it(umbrella_deps) -> None:
    """No login provisions in email mode — minting needs a source-native id.

    Left to render, the flag would sit in values reading as if it were in force
    while every person outside the roster is refused with nothing pointing here.
    """
    code, _, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        *ROSTER,
        "--set",
        "authenticator.oidc.resolveBy=email",
        "--set",
        "authenticator.oidc.provisionOnLogin=true",
    )
    assert code != 0, "render must fail"
    assert "provisionOnLogin" in err
