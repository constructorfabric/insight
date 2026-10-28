"""Every ledger request carries the session a replicated warehouse needs.

The sweep reads back what the tick before it wrote, through a load balancer that
may answer from a replica still fetching. On a cluster every request therefore
carries a quorum on the write and sequential consistency on the read, and the
identity mutation waits for every replica; on a single node the request is
exactly the one the ledger has always sent. The topology comes from the same
`RECONCILE_DEST_CLICKHOUSE_*` pair the Bronze destination is configured from.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import sys
import urllib.parse
from collections.abc import Callable
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep import ledger as ledger_module
from sweep.ledger import CLUSTER_SESSION_SETTINGS, Ledger, LedgerError
from sweep.plan import Instance

INSTANCE = Instance("example-tracker", "example-tenant", "example-tracker-main")
CLUSTER = "insight_cluster"

ENV = {
    "RECONCILE_DEST_CLICKHOUSE_HOST": "ledger.invalid",
    "RECONCILE_DEST_CLICKHOUSE_PORT": "8123",
    "RECONCILE_DEST_CLICKHOUSE_USERNAME": "user",
    "RECONCILE_DEST_CLICKHOUSE_PASSWORD": "password",
}


class _Response:
    def __enter__(self) -> _Response:
        return self

    def __exit__(self, *exc: object) -> bool:
        return False

    def read(self) -> bytes:
        return b""


class _Request:
    """One request as urllib was handed it: its URL parameters and its statement."""

    def __init__(self, url: str, body: bytes) -> None:
        self.parameters = {
            name: values[0] for name, values in urllib.parse.parse_qs(urllib.parse.urlparse(url).query).items()
        }
        self.sql = body.decode()

    @property
    def settings(self) -> dict[str, str]:
        return {name: value for name, value in self.parameters.items() if name != "query"}


@pytest.fixture
def requests(monkeypatch: pytest.MonkeyPatch) -> list[_Request]:
    seen: list[_Request] = []

    def fake_urlopen(request: urllib.request.Request, timeout: int) -> _Response:
        seen.append(_Request(request.full_url, request.data))
        return _Response()

    monkeypatch.setattr(ledger_module.urllib.request, "urlopen", fake_urlopen)
    return seen


def _standalone() -> Ledger:
    return Ledger("http://ledger.invalid:8123", "user", "password")


def _clustered() -> Ledger:
    return Ledger("http://ledger.invalid:8123", "user", "password", cluster=CLUSTER)


#: Every statement the ledger issues, by the method that issues it. A method
#: added without a line here is a request the cluster settings may not reach.
STATEMENTS: list[tuple[str, Callable[[Ledger], object]]] = [
    ("watermark", lambda ledger: ledger.watermark()),
    ("closed_job_ids", lambda ledger: ledger.closed_job_ids("2026-08-27 08:00:00.000")),
    ("abandoned_jobs", lambda ledger: ledger.abandoned_jobs("2026-08-27 08:00:00.000")),
    ("unidentified_connectors", lambda ledger: ledger.unidentified_connectors()),
    ("recorded_instance", lambda ledger: ledger.recorded_instance("bronze_example_tracker")),
    ("adopt_identity", lambda ledger: ledger.adopt_identity(INSTANCE)),
    ("insert", lambda ledger: ledger.insert([{"tick_id": "tick-a", "connector": "example-tracker"}])),
]


def test_every_public_statement_is_listed() -> None:
    public = {name for name in dir(Ledger) if not name.startswith("_") and name != "from_env"}
    assert public == {name for name, _ in STATEMENTS}


class TestAClusterSessionRidesEveryRequest:
    @pytest.mark.parametrize(("name", "issue"), STATEMENTS, ids=[name for name, _ in STATEMENTS])
    def test_quorum_writes_and_sequential_reads(self, requests: list[_Request], name: str, issue: Callable) -> None:
        issue(_clustered())

        assert requests, f"{name} issued no request"
        for request in requests:
            assert request.settings.items() >= CLUSTER_SESSION_SETTINGS.items(), f"{name}: {request.parameters}"

    def test_the_mutation_waits_for_every_replica(self, requests: list[_Request]) -> None:
        _clustered().adopt_identity(INSTANCE)

        (request,) = requests
        assert request.settings["mutations_sync"] == "2"
        assert "SETTINGS" not in request.sql, "a clause in the statement would override the session's wait"

    def test_the_catalogue_is_read_on_every_replica(self, requests: list[_Request]) -> None:
        """`total_rows` is what the answering replica holds, and one still
        fetching holds nothing yet."""
        _clustered().recorded_instance("bronze_example_tracker")

        (request,) = requests
        assert f"clusterAllReplicas('{CLUSTER}', system.tables)" in request.sql
        assert "skip_unavailable_shards = 1" in request.sql

    def test_the_recorded_identity_is_read_on_every_replica(self, monkeypatch: pytest.MonkeyPatch) -> None:
        """Bronze is written without a quorum, so a replica still fetching could
        answer the identity read with half the rows and one identity where the
        relation holds two."""
        seen: list[_Request] = []
        answers = iter([b'{"name":"rows"}\n', b""])

        class _Answer(_Response):
            def read(self) -> bytes:
                return next(answers)

        def fake_urlopen(request: urllib.request.Request, timeout: int) -> _Answer:
            seen.append(_Request(request.full_url, request.data))
            return _Answer()

        monkeypatch.setattr(ledger_module.urllib.request, "urlopen", fake_urlopen)
        _clustered().recorded_instance("bronze_example_tracker")

        _catalogue, identity = seen
        assert f"clusterAllReplicas('{CLUSTER}', bronze_example_tracker.rows)" in identity.sql
        assert "GROUP BY tenant_id, source_id LIMIT 2" in identity.sql
        assert identity.settings.items() >= CLUSTER_SESSION_SETTINGS.items()

    def test_the_insert_keeps_its_statement_beside_the_session(self, requests: list[_Request]) -> None:
        _clustered().insert([{"tick_id": "tick-a"}])

        (request,) = requests
        assert request.parameters["query"] == f"INSERT INTO {ledger_module.TABLE} FORMAT JSONEachRow"


class TestASingleNodeSendsTheRequestItAlwaysHas:
    @pytest.mark.parametrize(("name", "issue"), STATEMENTS, ids=[name for name, _ in STATEMENTS])
    def test_no_cluster_setting_reaches_a_single_node(
        self, requests: list[_Request], name: str, issue: Callable
    ) -> None:
        issue(_standalone())

        for request in requests:
            assert not set(request.settings) & set(CLUSTER_SESSION_SETTINGS), f"{name}: {request.parameters}"

    def test_the_mutation_still_waits_for_the_rewrite(self, requests: list[_Request]) -> None:
        _standalone().adopt_identity(INSTANCE)

        (request,) = requests
        assert request.settings == {"mutations_sync": "1"}

    def test_the_catalogue_is_the_node_s_own(self, requests: list[_Request]) -> None:
        _standalone().recorded_instance("bronze_example_tracker")

        (request,) = requests
        assert "FROM system.tables" in request.sql
        assert "clusterAllReplicas" not in request.sql


class TestTheTopologyComesFromTheReconcileEnv:
    @pytest.mark.parametrize("flag", ["true", "True", " yes ", "1", "on"])
    def test_the_flag_turns_the_session_on(self, requests: list[_Request], flag: str) -> None:
        ledger = Ledger.from_env(
            {**ENV, "RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": flag, "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME": CLUSTER}
        )
        ledger.watermark()

        assert requests[0].settings == CLUSTER_SESSION_SETTINGS

    @pytest.mark.parametrize("flag", ["false", "", "0", "off", "clustered"])
    def test_anything_else_is_a_single_node(self, requests: list[_Request], flag: str) -> None:
        ledger = Ledger.from_env({**ENV, "RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": flag})
        ledger.watermark()

        assert requests[0].settings == {}

    def test_a_name_without_the_flag_names_no_cluster(self, requests: list[_Request]) -> None:
        """The flag is what the chart hands every creator; a leftover name must
        not half-enable the session."""
        ledger = Ledger.from_env({**ENV, "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME": CLUSTER})
        ledger.watermark()

        assert requests[0].settings == {}

    @pytest.mark.parametrize("name", ["", "   ", "insight-cluster", "insight.cluster", "a'; DROP TABLE x"])
    def test_the_flag_without_a_usable_name_is_refused(self, name: str) -> None:
        """The name is spliced into `clusterAllReplicas(...)`: an identifier or nothing."""
        with pytest.raises(LedgerError, match="RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME"):
            Ledger.from_env(
                {
                    **ENV,
                    "RECONCILE_DEST_CLICKHOUSE_CLUSTER_MODE": "true",
                    "RECONCILE_DEST_CLICKHOUSE_CLUSTER_NAME": name,
                }
            )
