"""Tests for the opt-in org-size knob (`SEED_ORG_HEADCOUNT`).

The knob exists so a stand can be seeded with a realistic headcount; the thing
these tests actually protect is the OTHER side of it — that a stand which did
not ask for one gets the committed 26-person roster, unchanged, because CI and
the compose stand suite assert against exactly those people.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

from insight_seed import config, manifest, profiles, scale

_EMAIL = "dev@company.nonpresent"
_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"

_SEED_DIR = Path(__file__).resolve().parent.parent
_REASSEMBLER = _SEED_DIR / "manifest-from-log.sh"
_needs_reassembler = pytest.mark.skipif(
    not (_REASSEMBLER.is_file() and shutil.which("bash")),
    reason="manifest-from-log.sh needs bash",
)

_ENV = config.ORG_HEADCOUNT_ENV
_COMMITTED = config.CANONICAL_ROSTER_SIZE
_GROWN = 250
_PLAN_TARGETS = (27, 30, 100, _GROWN, 999, config.MAX_ORG_HEADCOUNT)


@pytest.fixture(scope="module")
def committed_roster() -> list[profiles.Person]:
    return profiles.build_roster(_EMAIL)


@pytest.fixture(scope="module")
def grown_roster() -> list[profiles.Person]:
    return profiles.build_seeded_roster(_EMAIL, _GROWN)


def _growth(grown_roster: list[profiles.Person]) -> list[profiles.Person]:
    return grown_roster[_COMMITTED:]


@pytest.mark.parametrize("raw", ["", "   ", "0"])
def test_an_unasked_for_headcount_parses_to_no_growth(raw: str) -> None:
    assert config.parse_org_headcount({_ENV: raw}) == 0


def test_an_absent_variable_parses_to_no_growth() -> None:
    assert config.parse_org_headcount({}) == 0


@pytest.mark.parametrize(
    ("raw", "want"),
    [("26", 26), ("27", 27), ("250", 250), (" 250 ", 250), ("3000", 3000)],
)
def test_a_headcount_at_or_above_the_committed_roster_passes_through(raw: str, want: int) -> None:
    assert config.parse_org_headcount({_ENV: raw}) == want


@pytest.mark.parametrize("value", range(1, _COMMITTED))
def test_below_the_committed_roster_is_refused(value: int) -> None:
    """The roster only grows: the committed people are named fixtures."""
    with pytest.raises(config.EnvContractError) as refusal:
        config.parse_org_headcount({_ENV: str(value)})

    assert str(_COMMITTED) in str(refusal.value), f"refusal should name the floor: {value}"


def test_over_the_ceiling_is_refused_by_name() -> None:
    with pytest.raises(config.EnvContractError) as refusal:
        config.parse_org_headcount({_ENV: str(config.MAX_ORG_HEADCOUNT + 1)})

    assert str(config.MAX_ORG_HEADCOUNT) in str(refusal.value)


@pytest.mark.parametrize("raw", ["abc", "26.5", "1e3", " twenty ", "0x1a"])
def test_a_non_integer_is_refused(raw: str) -> None:
    with pytest.raises(config.EnvContractError):
        config.parse_org_headcount({_ENV: raw})


def test_a_negative_headcount_is_refused() -> None:
    """An integer, so it reaches the floor check rather than the parse one."""
    with pytest.raises(config.EnvContractError):
        config.parse_org_headcount({_ENV: "-1"})


def test_the_empty_environment_matches_build_roster_exactly(
    committed_roster: list[profiles.Person],
) -> None:
    got = profiles.build_seeded_roster(_EMAIL, config.parse_org_headcount({}))

    assert len(got) == _COMMITTED
    assert got == committed_roster
    # Names are assigned by build-order index, so order is part of the contract.
    assert [p.uuid for p in got] == [p.uuid for p in committed_roster]
    assert [p.email for p in got] == [p.email for p in committed_roster]
    assert [p.display_name for p in got] == [p.display_name for p in committed_roster]


@pytest.mark.parametrize("raw", ["0", "26", ""])
def test_an_inert_value_builds_the_committed_roster(
    raw: str, committed_roster: list[profiles.Person]
) -> None:
    """Parsed exactly as a run would parse it, then built from the result."""
    headcount = config.parse_org_headcount({_ENV: raw})

    assert profiles.build_seeded_roster(_EMAIL, headcount) == committed_roster


def test_the_default_path_never_imports_scale() -> None:
    """In a fresh interpreter, so this test's own import cannot mask it.

    `build_seeded_roster` imports `scale` inside the grown branch precisely
    so the committed path cannot be affected by anything in that module.
    """
    source = (
        "import sys\n"
        "from insight_seed import profiles\n"
        f"roster = profiles.build_seeded_roster({_EMAIL!r}, 0)\n"
        f"assert len(roster) == {_COMMITTED}, len(roster)\n"
        "assert 'insight_seed.scale' not in sys.modules, 'scale.py was imported'\n"
    )

    subprocess.run([sys.executable, "-c", source], check=True)


def test_a_grown_roster_has_exactly_the_asked_for_size(
    grown_roster: list[profiles.Person],
) -> None:
    assert len(grown_roster) == _GROWN


def test_the_committed_roster_is_the_prefix_of_a_grown_one(
    committed_roster: list[profiles.Person], grown_roster: list[profiles.Person]
) -> None:
    assert grown_roster[:_COMMITTED] == committed_roster


@pytest.mark.parametrize("field", ["uuid", "email", "display_name"])
def test_every_identifier_stays_unique_across_a_grown_roster(
    field: str, grown_roster: list[profiles.Person]
) -> None:
    values = [getattr(person, field) for person in grown_roster]

    assert len(set(values)) == len(values), f"duplicate {field}"


def test_growth_uses_its_own_identifier_namespaces(
    committed_roster: list[profiles.Person], grown_roster: list[profiles.Person]
) -> None:
    committed_emails = {p.email for p in committed_roster}
    committed_uuids = {p.uuid for p in committed_roster}

    for person in _growth(grown_roster):
        assert person.email not in committed_emails
        assert person.uuid not in committed_uuids
        assert person.uuid.startswith("eeeeeeee-"), person.uuid
        assert person.email.endswith(f"@{profiles.COMPANY_EMAIL_SUFFIX}")


def test_growth_reuses_the_existing_roles_and_teams(
    grown_roster: list[profiles.Person],
) -> None:
    """An unknown role or team KeyErrors mid-seed; there must be neither."""
    for person in _growth(grown_roster):
        assert person.role in {"lead", "ic"}
        assert person.team in profiles.TEAM_PROFILES
        assert person.role in manifest.ROLE_TO_REALM_ROLES


def test_the_org_chart_stays_connected(grown_roster: list[profiles.Person]) -> None:
    """Every growth person reports to somebody already in the roster."""
    by_uuid = {p.uuid: p for p in grown_roster}

    for person in _growth(grown_roster):
        assert person.parent_uuid is not None
        parent = by_uuid[str(person.parent_uuid)]
        assert parent.team == person.team
        assert parent.role == "lead"


def test_no_squad_exceeds_the_span(grown_roster: list[profiles.Person]) -> None:
    reports: dict[str, int] = {}
    for person in _growth(grown_roster):
        if person.role == "ic":
            reports[str(person.parent_uuid)] = reports.get(str(person.parent_uuid), 0) + 1

    assert reports
    assert max(reports.values()) <= scale.SQUAD_SPAN


def test_committed_ics_keep_their_lead(
    committed_roster: list[profiles.Person], grown_roster: list[profiles.Person]
) -> None:
    for before, after in zip(committed_roster, grown_roster, strict=False):
        assert before.parent_uuid == after.parent_uuid


def test_team_shares_stay_balanced(grown_roster: list[profiles.Person]) -> None:
    counts = dict.fromkeys(scale.GROWTH_TEAMS, 0)
    for person in _growth(grown_roster):
        counts[str(person.team)] += 1

    assert max(counts.values()) - min(counts.values()) <= 1


@pytest.mark.parametrize("target", [_COMMITTED + 1, config.MAX_ORG_HEADCOUNT])
def test_the_growth_extremes_stay_unique(target: int) -> None:
    roster = profiles.build_seeded_roster(_EMAIL, target)

    assert len(roster) == target
    assert len({p.email for p in roster}) == target
    assert len({p.display_name for p in roster}) == target


@pytest.mark.parametrize("target", _PLAN_TARGETS)
def test_planned_slots_add_up_to_the_growth(target: int) -> None:
    plans = scale.plan(target)

    assert sum(p.total for p in plans) == target - _COMMITTED


@pytest.mark.parametrize("target", _PLAN_TARGETS)
def test_every_planned_ic_fits_a_squad(target: int) -> None:
    for team_plan in scale.plan(target):
        if team_plan.ics:
            assert team_plan.managers >= 1, f"{team_plan.team} has ics and no lead"
        assert team_plan.ics <= team_plan.managers * scale.SQUAD_SPAN, team_plan.team


def test_a_base_that_is_not_the_committed_roster_is_refused() -> None:
    """`extend_roster` runs inside a Job against a real stand.

    A duplicated email resolves to nobody in gold rather than failing, so the
    writer has to refuse before it writes anything.
    """
    with pytest.raises(RuntimeError):
        scale.extend_roster(profiles.build_roster(_EMAIL)[:10], _GROWN)


def test_a_target_that_is_not_growth_is_refused() -> None:
    with pytest.raises(RuntimeError):
        scale.extend_roster(profiles.build_roster(_EMAIL), _COMMITTED)


def test_bulk_names_cannot_collide_with_the_committed_ones() -> None:
    assert not set(scale._BULK_LAST_NAMES) & set(profiles._LAST_NAMES)


#: The head of the pool the pinned Faker seed yields. Written out, not derived:
#: comparing the generator to its own output passes whatever the seed is.
_GOLDEN_SURNAMES = ("Thomas", "Austin", "Gonzalez", "Glenn", "Fisher")


def test_the_growth_surname_pool_is_pinned() -> None:
    """A changed seed or Faker version renames a re-seeded stand's whole growth."""
    assert tuple(scale._BULK_LAST_NAMES[: len(_GOLDEN_SURNAMES)]) == _GOLDEN_SURNAMES
    assert len(set(scale._BULK_LAST_NAMES)) == len(scale._BULK_LAST_NAMES)
    assert scale._generate_bulk_last_names() == scale._BULK_LAST_NAMES


def test_the_realm_generator_sizes_itself_to_the_headcount() -> None:
    """`build_realm` is pure, so the roster it embeds is checkable directly."""
    from insight_seed import keycloak_realm

    def users(headcount: int) -> int:
        realm = keycloak_realm.build_realm(_EMAIL, _TENANT, [], "secret", headcount)
        return len(realm["users"])

    assert users(_COMMITTED + 1) == users(0) + 1


@pytest.mark.parametrize("module", ["identity", "silver", "keycloak_realm"])
def test_every_writer_reaches_the_roster_through_a_parsed_headcount(module: str) -> None:
    """A literal at the call site seeds the wrong org, and no unit test sees it.

    Static because the alternative is stubbing three subsystems; it still fails
    on the mutation that matters — the argument replaced by a constant.
    """
    source = (Path(__file__).parent.parent / "insight_seed" / f"{module}.py").read_text()
    calls = [ln.strip() for ln in source.splitlines() if "build_seeded_roster(" in ln and "=" in ln]

    assert "parse_org_headcount" in source, f"{module} never parses the headcount"
    assert calls, f"{module} has no build_seeded_roster call"
    for call in calls:
        argument = call.rsplit(",", 1)[-1].rstrip(")").strip()
        assert not argument.isdigit(), f"{module} passes the literal {argument} as the headcount"


#: The apiserver's cap on the summed length of a ConfigMap's `data` entries.
_CONFIGMAP_DATA_MAX_BYTES = 1024 * 1024


def test_the_largest_roster_still_fits_a_configmap() -> None:
    """`seed-stand.sh` publishes this document; raising the ceiling can break it."""
    doc = manifest.build_manifest(
        {
            "DEV_USER_EMAIL": _EMAIL,
            "TENANT_DEFAULT_ID": _TENANT,
            "SEED_ANCHOR_DATE": "2026-06-30",
            "SEED_DAYS": "60",
            config.CROSS_TENANT_FIXTURE_ENV: "0",
            _ENV: str(config.MAX_ORG_HEADCOUNT),
        }
    )
    compact = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)

    assert len(compact.encode("utf-8")) < _CONFIGMAP_DATA_MAX_BYTES
