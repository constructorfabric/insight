"""What the umbrella must compose for previews: the gateway-JWT verification
chain (same wiring as identity-resolution) and the /api/previews edge route."""

from __future__ import annotations

import yaml
from conftest import UMBRELLA_BASE, render


def _docs(manifests: str) -> list[dict]:
    return [d for d in yaml.safe_load_all(manifests) if isinstance(d, dict)]


def _previews_host(manifests: str) -> dict:
    configs = [
        d
        for d in _docs(manifests)
        if d.get("kind") == "ConfigMap" and d["metadata"]["name"].endswith("-previews-gears-config")
    ]
    assert len(configs) == 1, f"expected one previews gears ConfigMap, got {len(configs)}"
    return yaml.safe_load(configs[0]["data"]["insight.yaml"])


def test_the_gateway_jwt_chain_matches_identity_resolutions(umbrella_deps) -> None:
    """Issuer resolved off the release's authenticator over https, the fixed
    audience, and the authn-tls CA mounted for the JWKS fetch."""
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE)
    assert code == 0, err

    plugin = _previews_host(out)["gears"]["oidc-authn-plugin"]["config"]
    assert plugin["jwt"]["trusted_issuers"] == [
        {"issuer": "https://contract-test-authenticator.default.svc.cluster.local:8443"}
    ]
    assert plugin["jwt"]["expected_audience"] == ["internal-services"]
    assert plugin["http_client"]["custom_ca_certificate_paths"] == ["/etc/insight/authn-ca/ca.crt"]

    deployments = [
        d for d in _docs(out) if d.get("kind") == "Deployment" and d["metadata"]["name"] == "contract-test-previews"
    ]
    assert len(deployments) == 1
    volumes = deployments[0]["spec"]["template"]["spec"]["volumes"]
    (ca_volume,) = [v for v in volumes if v["name"] == "authn-ca"]
    assert ca_volume["secret"]["secretName"] == "insight-authenticator-authn-tls-cert"


def test_the_edge_routes_api_previews_to_the_service(umbrella_deps) -> None:
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE)
    assert code == 0, err

    gateway_configs = [d for d in _docs(out) if d.get("kind") == "ConfigMap" and "routes.yaml" in (d.get("data") or {})]
    assert len(gateway_configs) == 1, "expected exactly one gateway route-table ConfigMap"

    routes = yaml.safe_load(gateway_configs[0]["data"]["routes.yaml"])["routes"]
    (row,) = [r for r in routes if r["prefix"] == "/api/previews"]
    assert row["upstream"] == "http://contract-test-previews:8085"
    assert row["strip_prefix"] is True, "the service serves /v1/* — the prefix must strip"


def test_previews_can_be_left_out(umbrella_deps) -> None:
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE, "--set", "previews.deploy=false")
    assert code == 0, err

    assert not [d for d in _docs(out) if (d.get("metadata") or {}).get("name", "").endswith("-previews-gears-config")]


def test_the_experiments_namespace_stays_single_sourced_through_the_umbrella(umbrella_deps) -> None:
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE, "--set", "previews.experiments.namespace=stand-previews")
    assert code == 0, err

    docs = _docs(out)
    config = _previews_host(out)["gears"]["previews"]["config"]
    assert config["namespace"] == "stand-previews"

    (role,) = [d for d in docs if d.get("kind") == "Role" and d["metadata"]["name"].endswith("-previews-experiments")]
    assert role["metadata"]["namespace"] == "stand-previews"
