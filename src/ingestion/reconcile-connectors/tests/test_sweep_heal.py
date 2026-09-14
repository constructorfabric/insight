"""Whose history is it, for rows recorded before the ledger carried identity.

Three rungs, taken in order of how directly the answer is known: the Secret the
install configures now, the identity the connector's own data was recorded
under, and — only then — the naming this install demonstrably follows.

The last rung is the only place a value is inferred rather than read, so most of
what is pinned here is when it must NOT fire: never for a connector with
recorded syncs, and never at all on an install that names even one instance
differently.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep import heal
from sweep.plan import Instance

CONNECTOR = "example-tracker"
TENANT = "tenant-under-test"


def configured_only(connector: str) -> heal.Unidentified:
    """A connector whose unidentified rows record no sync at all."""
    return heal.Unidentified(connector, has_syncs=False)


def with_syncs(connector: str) -> heal.Unidentified:
    return heal.Unidentified(connector, has_syncs=True)


def conventional(connector: str) -> Instance:
    return Instance(connector, TENANT, f"{connector}-main")


class TestTheSecretAnswersFirst:
    def test_a_connector_installed_once_claims_its_unidentified_rows(self) -> None:
        instance = Instance(CONNECTOR, TENANT, "main")

        adoption = heal.plan_adoption([with_syncs(CONNECTOR)], [instance])

        assert [claim.instance for claim in adoption.claimed] == [instance]
        assert adoption.claimed[0].basis == heal.FROM_SECRET
        assert adoption.unresolved == []

    def test_the_identity_is_the_installed_one_not_a_conventional_name(self) -> None:
        """The rows are handed what this install actually calls the instance.

        Defaulting to the usual source id would write an instance that never ran
        onto every row of the one that did — and it would read as fact
        afterwards, because nothing in the row says it was assumed.
        """
        instance = Instance(CONNECTOR, TENANT, "reports-only")

        adoption = heal.plan_adoption([with_syncs(CONNECTOR)], [instance])

        assert [claim.instance for claim in adoption.claimed] == [instance]

    def test_a_connector_installed_twice_keeps_its_rows_unidentified(self) -> None:
        """The connection told the two apart and it is not in the rows, so which
        instance ran them cannot be recovered."""
        first = Instance(CONNECTOR, TENANT, "main")
        second = Instance(CONNECTOR, TENANT, "second")

        adoption = heal.plan_adoption([with_syncs(CONNECTOR)], [first, second])

        assert adoption.claimed == []
        assert "installed 2 times" in adoption.unresolved[0].reason


class TestWhatTheConnectorRecordedAboutItself:
    def test_recorded_data_answers_where_no_secret_can(self) -> None:
        """Its rows outlive its Secret, and they carry the identity the sync ran
        under — a fact, not an inference."""
        departed = Instance("departed-tracker", TENANT, "departed-tracker-main")

        adoption = heal.plan_adoption(
            [with_syncs("departed-tracker")],
            [conventional(CONNECTOR)],
            recovered=[departed],
        )

        claimed = {claim.instance.connector: claim for claim in adoption.claimed}
        assert claimed["departed-tracker"].instance == departed
        assert claimed["departed-tracker"].basis == heal.FROM_RECORDED_DATA

    def test_a_secret_outranks_recorded_data(self) -> None:
        """What the install configures now is the more direct answer; recorded
        data is what is read when there is no Secret to ask."""
        live = Instance(CONNECTOR, TENANT, "reports-only")
        stale = Instance(CONNECTOR, TENANT, f"{CONNECTOR}-main")

        adoption = heal.plan_adoption([with_syncs(CONNECTOR)], [live], recovered=[stale])

        assert adoption.claimed[0].instance == live
        assert adoption.claimed[0].basis == heal.FROM_SECRET


class TestTheInstallsOwnNamingIsTheLastRung:
    def test_a_connector_that_never_synced_takes_the_unanimous_name(self) -> None:
        """Nothing of it survives — no Secret, no rows it wrote — but it never
        recorded a sync either, so there is no work to misattribute. Every
        instance this install CAN read is named the same way, and that is what
        licenses reading the rest the same."""
        adoption = heal.plan_adoption(
            [configured_only("departed-tracker")],
            [conventional(CONNECTOR), conventional("other-tracker")],
        )

        assert adoption.unresolved == []
        assert adoption.claimed[0].instance == Instance(
            "departed-tracker", TENANT, "departed-tracker-main"
        )
        assert adoption.claimed[0].basis == heal.FROM_INSTALL_NAMING

    def test_a_connector_with_recorded_syncs_is_never_inferred(self) -> None:
        """Its identity decides whose work those syncs were. Wrong, it is
        indistinguishable from right ever after — so the rows stay unidentified
        and say so."""
        adoption = heal.plan_adoption(
            [with_syncs("departed-tracker")],
            [conventional(CONNECTOR), conventional("other-tracker")],
        )

        assert adoption.claimed == []
        assert "recorded syncs" in adoption.unresolved[0].reason

    def test_one_instance_named_differently_turns_the_rung_off(self) -> None:
        """Fails closed for the whole install, not just for that connector: a
        single counter-example shows the install does not work the way the rung
        would assume."""
        adoption = heal.plan_adoption(
            [configured_only("departed-tracker")],
            [conventional(CONNECTOR), Instance("other-tracker", TENANT, "reports-only")],
        )

        assert adoption.claimed == []
        assert "does not name every instance the same way" in adoption.unresolved[0].reason

    def test_more_than_one_tenant_turns_the_rung_off(self) -> None:
        """The rung has to name a tenant as well as a source id, and an install
        showing two has not shown which one a vanished connector ran under."""
        adoption = heal.plan_adoption(
            [configured_only("departed-tracker")],
            [
                conventional(CONNECTOR),
                Instance("other-tracker", "another-tenant", "other-tracker-main"),
            ],
        )

        assert adoption.claimed == []

    def test_an_install_with_nothing_readable_infers_nothing(self) -> None:
        adoption = heal.plan_adoption([configured_only("departed-tracker")], [])

        assert adoption.claimed == []
        assert adoption.unresolved[0].connector == "departed-tracker"


class TestTheRungsTogether:
    def test_the_resolvable_are_claimed_beside_the_unresolvable(self) -> None:
        """One connector that cannot be resolved must not hold up the rest."""
        instance = conventional(CONNECTOR)
        recovered = Instance("left-data", TENANT, "left-data-main")

        adoption = heal.plan_adoption(
            [
                with_syncs(CONNECTOR),
                with_syncs("left-data"),
                configured_only("never-ran"),
                with_syncs("vanished"),
            ],
            [instance],
            recovered=[recovered],
        )

        assert {claim.instance.connector for claim in adoption.claimed} == {
            CONNECTOR,
            "left-data",
            "never-ran",
        }
        assert [u.connector for u in adoption.unresolved] == ["vanished"]

    def test_a_name_repeated_by_the_ledger_is_claimed_once(self) -> None:
        instance = Instance(CONNECTOR, TENANT, "main")

        adoption = heal.plan_adoption(
            [with_syncs(CONNECTOR), with_syncs(CONNECTOR)], [instance]
        )

        assert len(adoption.claimed) == 1
