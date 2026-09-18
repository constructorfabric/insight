"""Helm render-contract for the insight-v3-core chart.

The facts this chart gets wrong are all invisible in a diff. Its credentials
arrive from a pre-created Secret, so a render that forgets it, or marks it
optional, yields a pod that starts and rejects every request. The ingest token
arrives from a SECOND, operator-owned Secret by `secretKeyRef` — a chart that
carries the token in its own rendered output mints a new one on every `helm
template`, where `lookup` is blind, locking out every ingest client holding
the previous value with no diff to show for it. The MCP surface is two
listeners deep — a Service port, a container port and four env leaves — and
any one of them missing leaves the other three looking correct. And
`mcp.public_url` is what the server verifies audiences against, so an empty
one must stop the render rather than reach a pod.

No cluster involved; runs anywhere helm + PyYAML exist. Renders the subchart
only, so it needs no vendored umbrella dependencies.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]
RELEASE = "contract-test"

SECRET = {"existingSecret": "cfg"}
MCP_ON = {"global__mcp__enabled": "true", "global__mcp__publicUrl": "https://insight.test"}


def helm_args(overrides: dict[str, str]) -> list[str]:
    args = ["helm", "template", RELEASE, str(CHART)]
    for key, value in overrides.items():
        args += ["--set", f"{key.replace('__', '.')}={value}"]
    return args


def render(**overrides: str) -> list[dict]:
    args = helm_args({**SECRET, **overrides})
    out = subprocess.run(args, capture_output=True, text=True, timeout=120, check=True).stdout
    return [doc for doc in yaml.safe_load_all(out) if doc]


def render_fails(**overrides: str) -> str:
    """The stderr of a render that must be refused."""
    done = subprocess.run(helm_args(overrides), capture_output=True, text=True, timeout=120, check=False)
    assert done.returncode != 0, f"render should have failed, got:\n{done.stdout}"
    return done.stderr


def of_kind(docs: list[dict], kind: str) -> list[dict]:
    return [doc for doc in docs if doc.get("kind") == kind]


def one(docs: list[dict], kind: str) -> dict:
    found = of_kind(docs, kind)
    assert len(found) == 1, f"expected exactly one {kind}, got {len(found)}"
    return found[0]


def container(docs: list[dict]) -> dict:
    return one(docs, "Deployment")["spec"]["template"]["spec"]["containers"][0]


def env_of(docs: list[dict]) -> dict[str, str]:
    """The container's literal env values; `valueFrom` entries have none."""
    return {entry["name"]: entry["value"] for entry in container(docs).get("env", []) if "value" in entry}


def env_names(docs: list[dict]) -> list[str]:
    return [entry["name"] for entry in container(docs).get("env", [])]


def host_config(docs: list[dict]) -> dict:
    return yaml.safe_load(one(docs, "ConfigMap")["data"]["insight.yaml"])


def test_the_deployment_sources_its_credentials_from_the_named_secret():
    """The ClickHouse URL, database, user and password all arrive here.

    A missing or optional reference starts a pod that answers /health and
    500s every real request, which no smoke notices.
    """
    docs = render(existingSecret="v3-core-config")
    sources = container(docs)["envFrom"]

    secrets = [ref["secretRef"] for ref in sources if "secretRef" in ref]
    assert secrets == [{"name": "v3-core-config"}], f"one required config Secret, got {sources}"
    assert not secrets[0].get("optional"), "the pod must not start without its credentials"


def test_the_migrate_hook_reads_the_same_secret():
    job = one(render(existingSecret="v3-core-config"), "Job")
    refs = job["spec"]["template"]["spec"]["containers"][0]["envFrom"]

    assert [ref["secretRef"]["name"] for ref in refs] == ["v3-core-config"]


INGEST_TOKEN = "APP__gears__insight_v3_core__config__ingest_token"


def test_the_ingest_token_arrives_by_reference_to_an_operator_owned_secret():
    """`helm template` sees nothing from `lookup`, so a token this chart
    rendered as a value would differ per render and lock out every client."""
    docs = render(ingest__tokenSecret="v3-token", ingest__tokenKey="ingest")
    entry = next(e for e in container(docs)["env"] if e["name"] == INGEST_TOKEN)

    assert entry["valueFrom"]["secretKeyRef"] == {"name": "v3-token", "key": "ingest"}
    assert "value" not in entry


def test_no_rendered_object_carries_the_token_as_a_value():
    for doc in render():
        for field in ("data", "stringData"):
            assert INGEST_TOKEN not in (doc.get(field) or {}), f"{doc['kind']} carries the token"


def test_the_migrate_hook_takes_no_ingest_token():
    """`migrate` validates its two stores only — config::validate_stores."""
    job = one(render(), "Job")

    assert INGEST_TOKEN not in [e["name"] for e in job["spec"]["template"]["spec"]["containers"][0].get("env", [])]


def test_an_empty_ingest_token_secret_refuses_to_render():
    assert "ingest.tokenSecret is required" in render_fails(**SECRET, ingest__tokenSecret="")


def test_an_empty_existing_secret_refuses_to_render():
    assert "existingSecret is required" in render_fails()


def test_the_platform_configmap_is_wired_in_optionally():
    """The OTEL_* emission contract lives in the umbrella's platform CM; a
    standalone subchart install has no such object, so the reference is
    optional while the Secret is not."""
    sources = container(render())["envFrom"]

    (platform,) = [ref["configMapRef"] for ref in sources if "configMapRef" in ref]
    assert platform["name"] == f"{RELEASE}-platform"
    assert platform["optional"] is True


def test_the_rest_port_is_the_only_one_until_mcp_is_enabled():
    docs = render()

    (port,) = one(docs, "Service")["spec"]["ports"]
    assert (port["name"], port["port"], port["targetPort"]) == ("http", 8086, "http")
    assert [p["name"] for p in container(docs)["ports"]] == ["http"]


def test_mcp_adds_a_second_port_on_the_service_and_the_container():
    """A Service port whose targetPort names no container port resolves to
    nothing and every MCP client times out against a healthy-looking pod."""
    docs = render(**MCP_ON)

    ports = {port["name"]: port for port in one(docs, "Service")["spec"]["ports"]}
    assert ports["http"]["port"] == 8086
    assert ports["mcp"]["port"] == 8087
    assert ports["mcp"]["targetPort"] == "mcp"

    declared = {port["name"]: port["containerPort"] for port in container(docs)["ports"]}
    assert declared == {"http": 8086, "mcp": 8087}


def test_no_mcp_env_leaf_is_rendered_while_mcp_is_off():
    assert [name for name in env_names(render()) if "__mcp__" in name] == []


def test_enabling_mcp_wires_the_listener_and_the_published_origin():
    env = env_of(render(**MCP_ON))
    prefix = "APP__gears__insight_v3_core__config__mcp__"

    assert env[f"{prefix}enabled"] == "true"
    assert env[f"{prefix}bind_addr"] == "0.0.0.0:8087"
    assert env[f"{prefix}public_url"] == "https://insight.test"


def test_the_mcp_bind_addr_follows_the_configured_port():
    docs = render(**MCP_ON, mcp__port="9099")

    assert env_of(docs)["APP__gears__insight_v3_core__config__mcp__bind_addr"] == "0.0.0.0:9099"
    assert {port["name"]: port["port"] for port in one(docs, "Service")["spec"]["ports"]}["mcp"] == 9099


def test_enabling_mcp_without_a_public_url_refuses_to_render():
    """The server verifies token audiences against this origin; empty, it
    would reject every MCP call after a green install."""
    stderr = render_fails(**SECRET, global__mcp__enabled="true")

    assert "global.mcp.publicUrl is required" in stderr


@pytest.mark.parametrize(
    ("overrides", "expected"),
    [
        ({}, "false"),
        ({"global__mcp__allowInsecurePrivateNetwork": "true"}, "true"),
        ({"mcp__allowInsecurePrivateNetwork": "true"}, "true"),
        ({"global__mcp__allowInsecurePrivateNetwork": "false", "mcp__allowInsecurePrivateNetwork": "true"}, "true"),
    ],
)
def test_allow_insecure_private_network_follows_the_global_switch(overrides: dict[str, str], expected: str):
    """The umbrella knob turns it on for every MCP verifier at once; unset, the
    subchart's own value stands. The two are OR-ed, so a global `false` cannot
    take back a subchart `true` — the last row is that asymmetry, not a typo."""
    env = env_of(render(**MCP_ON, **overrides))

    assert env["APP__gears__insight_v3_core__config__mcp__allow_insecure_private_network"] == expected


def test_the_migrate_hook_runs_the_image_the_configmap_pins():
    """The umbrella contract already ties every gears service's
    service.version to the image its Deployment runs
    (identity-resolution/helm/tests/test_umbrella_log_context_contract.py).
    It reads deployment.yaml only, so the pre-upgrade migrate Job — a second
    container running the same binary — is unchecked there: a Job left on a
    stale tag would apply the previous release's migrations under a version
    the config claims is current.
    """
    tag = "2026.09.10-abc123"
    docs = render(image__tag=tag)

    attributes = host_config(docs)["opentelemetry"]["resource"]["attributes"]
    assert attributes["service.version"] == tag

    images = {container(docs)["image"], one(docs, "Job")["spec"]["template"]["spec"]["containers"][0]["image"]}
    assert images == {f"ghcr.io/constructorfabric/insight-v3-core:{tag}"}
