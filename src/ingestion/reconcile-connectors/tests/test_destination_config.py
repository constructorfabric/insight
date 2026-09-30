"""What the Bronze destination is handed as its connectionConfiguration.

Two contracts meet here. The 2.x spec is exact — `port` is a string, `protocol`
is required, and a key the spec does not declare is a 422 rather than an
ignored extra — so a standalone install must send exactly what it sent before
the topology fields existed. And a clustered install must send them, or the
only creator of bronze tables builds unreplicated ones.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

COMPOSER = Path(__file__).resolve().parents[1] / "python" / "compose_destination_config.py"

STANDALONE_ENV = {
    "RECONCILE_DEST_CLICKHOUSE_HOST": "clickhouse.example.test",
    "RECONCILE_DEST_CLICKHOUSE_PORT": "8123",
    "RECONCILE_DEST_CLICKHOUSE_PROTOCOL": "http",
    "RECONCILE_DEST_CLICKHOUSE_DATABASE": "insight",
    "RECONCILE_DEST_CLICKHOUSE_USERNAME": "insight",
    "RECONCILE_DEST_CLICKHOUSE_PASSWORD": "example-password",
}

#: What the inline builder this script replaced emitted, character for
#: character. A destination created with a different payload is recreated,
#: which drops the connection's stream cursors.
STANDALONE_JSON = (
    '{"host": "clickhouse.example.test", "port": "8123", "protocol": "http", '
    '"database": "insight", "username": "insight", "password": "example-password", '
    '"enable_json": false}'
)


def _compose(env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(COMPOSER)], env=env, capture_output=True, text=True, encoding="utf-8", check=False
    )


def test_a_standalone_install_sends_exactly_what_it_sent_before_the_topology_fields() -> None:
    done = _compose(STANDALONE_ENV)

    assert done.returncode == 0, done.stderr
    assert done.stdout.rstrip("\n") == STANDALONE_JSON


@pytest.mark.parametrize("flag", ["", "false", "0", "no", "off"])
def test_an_unset_or_disabled_flag_leaves_both_topology_keys_out(flag: str) -> None:
    """A destination older than 2.1.29 rejects a key its spec does not
    declare, so "standalone" must mean absent, not `false`."""
    done = _compose(
        STANDALONE_ENV
        | {"RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": flag, "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME": "insight_cluster"}
    )

    assert done.returncode == 0, done.stderr
    assert done.stdout.rstrip("\n") == STANDALONE_JSON, f"should stay standalone: {flag!r}"


def test_the_flag_alone_asks_for_replicated_engines_and_names_no_cluster() -> None:
    """The epic's chosen mechanism: a database on the `Replicated` engine
    distributes DDL itself, so `cluster_name` is left out."""
    done = _compose(STANDALONE_ENV | {"RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": "true"})

    config = json.loads(done.stdout)
    assert config["use_replicated_engines"] is True
    assert "cluster_name" not in config


def test_a_named_cluster_travels_with_the_replication_flag() -> None:
    done = _compose(
        STANDALONE_ENV
        | {
            "RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": "true",
            "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME": "insight_cluster",
        }
    )

    config = json.loads(done.stdout)
    assert config["use_replicated_engines"] is True
    assert config["cluster_name"] == "insight_cluster"


def test_the_credentials_stay_where_the_topology_keys_were_appended() -> None:
    """Key order decides the payload bytes, and the destination is recreated
    when the payload changes — the new keys go last, never in the middle."""
    done = _compose(STANDALONE_ENV | {"RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": "1"})

    keys = list(json.loads(done.stdout))
    assert keys == [
        "host",
        "port",
        "protocol",
        "database",
        "username",
        "password",
        "enable_json",
        "use_replicated_engines",
    ]


@pytest.mark.parametrize("missing", sorted(STANDALONE_ENV.keys() - {"RECONCILE_DEST_CLICKHOUSE_PROTOCOL"}))
def test_a_missing_credential_refuses_rather_than_publishing_a_half_built_destination(missing: str) -> None:
    done = _compose({k: v for k, v in STANDALONE_ENV.items() if k != missing})

    assert done.returncode == 1, f"should refuse without {missing}"
    assert missing in done.stderr
    assert done.stdout == ""


def test_the_protocol_falls_back_to_plain_http() -> None:
    """The bundled ClickHouse answers plain HTTP on 8123, and the chart's
    url helper assumes the same."""
    done = _compose({k: v for k, v in STANDALONE_ENV.items() if k != "RECONCILE_DEST_CLICKHOUSE_PROTOCOL"})

    assert json.loads(done.stdout)["protocol"] == "http"
