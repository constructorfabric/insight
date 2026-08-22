"""Tests for the opt-in org-size knob (`SEED_ORG_HEADCOUNT`).

The knob exists so a stand can be seeded with a realistic headcount; the thing
these tests actually protect is the OTHER side of it — that a stand which did
not ask for one gets the committed 26-person roster, unchanged, because CI and
the compose stand suite assert against exactly those people.

Run against the installed package (see the README's develop section):

    uv run --extra dev pytest tests
"""

from __future__ import annotations

import contextlib
import io
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


def test_bulk_names_are_deterministic() -> None:
    """Same Faker seed, same pool: a re-seeded stand keeps its people's names."""
    regenerated = scale._generate_bulk_last_names()

    assert regenerated == scale._BULK_LAST_NAMES
    assert len(set(regenerated)) == len(regenerated)


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


def _env(headcount: str | None = None) -> dict[str, str]:
    env = {
        "DEV_USER_EMAIL": _EMAIL,
        "TENANT_DEFAULT_ID": _TENANT,
        "SEED_ANCHOR_DATE": "2026-06-30",
        "SEED_DAYS": "60",
        config.CROSS_TENANT_FIXTURE_ENV: "0",
    }
    if headcount is not None:
        env[_ENV] = headcount
    return env


def _emit(doc: manifest.Manifest) -> list[str]:
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        manifest.emit_manifest_sentinel(doc)
    return buffer.getvalue().splitlines()


@pytest.fixture(scope="module")
def default_manifest() -> manifest.Manifest:
    return manifest.build_manifest(_env())


@pytest.fixture(scope="module")
def grown_manifest() -> manifest.Manifest:
    return manifest.build_manifest(_env(str(_GROWN)))


def test_the_default_roster_still_emits_one_plain_line(
    default_manifest: manifest.Manifest,
) -> None:
    """The manifest is printed after every write, in a Job with backoffLimit 0.

    Refusing an oversized document there would report a correctly seeded stand
    as a failure, with nothing left to retry.
    """
    lines = _emit(default_manifest)

    assert len(lines) == 1
    assert lines[0].startswith(manifest.SENTINEL_PREFIX)
    assert len(lines[0].encode("utf-8")) <= manifest._SENTINEL_MAX_BYTES
    assert json.loads(lines[0][len(manifest.SENTINEL_PREFIX) :]) == default_manifest
    assert manifest.decode_manifest_sentinel(lines) == default_manifest


def test_a_grown_roster_round_trips_through_the_chunked_form(
    grown_manifest: manifest.Manifest,
) -> None:
    assert len(grown_manifest["personas"]) == _GROWN

    lines = _emit(grown_manifest)

    assert lines
    for line in lines:
        assert line.startswith(manifest.GZ_SENTINEL_PREFIX), line[:40]
        assert len(line.encode("utf-8")) <= manifest._SENTINEL_MAX_BYTES
    assert manifest.decode_manifest_sentinel(lines) == grown_manifest


def test_chunks_survive_reordering_and_surrounding_log_noise(
    grown_manifest: manifest.Manifest,
) -> None:
    lines = [
        "2026-06-30 INFO seed.silver generating rows",
        *reversed(_emit(grown_manifest)),
        "done",
    ]

    assert manifest.decode_manifest_sentinel(lines) == grown_manifest


def test_a_missing_chunk_is_an_error_not_a_partial_document() -> None:
    doc = manifest.build_manifest(_env(str(config.MAX_ORG_HEADCOUNT)))
    lines = _emit(doc)
    assert len(lines) > 1, "the largest roster should need several chunks"

    with pytest.raises(ValueError):
        manifest.decode_manifest_sentinel(lines[:-1])


#: The apiserver's cap on the summed length of a ConfigMap's `data` entries.
_CONFIGMAP_DATA_MAX_BYTES = 1024 * 1024


def test_the_largest_roster_still_fits_a_configmap() -> None:
    """`seed-stand.sh` publishes this document; raising the ceiling can break it."""
    doc = manifest.build_manifest(_env(str(config.MAX_ORG_HEADCOUNT)))
    compact = json.dumps(doc, separators=(",", ":"), ensure_ascii=False)

    assert len(compact.encode("utf-8")) < _CONFIGMAP_DATA_MAX_BYTES


def test_input_without_a_sentinel_is_an_error() -> None:
    with pytest.raises(ValueError):
        manifest.decode_manifest_sentinel(["nothing to see here"])


@pytest.mark.parametrize("payload", ["null", "[]", "42", '"a string"'])
def test_a_sentinel_carrying_a_non_object_is_refused(payload: str) -> None:
    """`json.loads` returns whatever the text was; only an object is a manifest."""
    with pytest.raises(ValueError, match="not a JSON object"):
        manifest.decode_manifest_sentinel([manifest.SENTINEL_PREFIX + payload])


def test_an_identical_line_read_twice_is_tolerated(grown_manifest: manifest.Manifest) -> None:
    """A re-read log repeats lines; the same bytes are the same chunk."""
    lines = _emit(grown_manifest)

    assert manifest.decode_manifest_sentinel(lines + lines) == grown_manifest


def test_conflicting_totals_are_an_error_not_a_splice(grown_manifest: manifest.Manifest) -> None:
    """Two emissions in one log must fail, not decode whichever came last."""
    lines = _emit(grown_manifest)
    foreign = f"{manifest.GZ_SENTINEL_PREFIX}1/{len(lines) + 1} AAAA"

    with pytest.raises(ValueError):
        manifest.decode_manifest_sentinel([foreign, *lines])


def test_a_differing_duplicate_chunk_is_an_error(grown_manifest: manifest.Manifest) -> None:
    lines = _emit(grown_manifest)
    forged = f"{manifest.GZ_SENTINEL_PREFIX}1/{len(lines)} AAAA"

    with pytest.raises(ValueError):
        manifest.decode_manifest_sentinel([*lines, forged])


@_needs_reassembler
@pytest.mark.parametrize("manifest_fixture", ["default_manifest", "grown_manifest"])
def test_the_shell_reassembler_agrees_with_the_python_one(
    manifest_fixture: str, request: pytest.FixtureRequest
) -> None:
    doc = request.getfixturevalue(manifest_fixture)
    log = "\n".join(["starting", *_emit(doc), "done"]) + "\n"

    result = subprocess.run(
        ["bash", str(_REASSEMBLER)],
        input=log,
        capture_output=True,
        text=True,
        check=True,
    )

    out = result.stdout.strip()
    assert out.startswith(manifest.SENTINEL_PREFIX), out[:60]
    assert json.loads(out[len(manifest.SENTINEL_PREFIX) :]) == doc


@_needs_reassembler
def test_the_shell_reassembler_refuses_a_corrupt_payload() -> None:
    """A failed decode must not print a bare sentinel — it matches the grep.

    Consumers select the manifest with `grep -m1 '^SEED_MANIFEST_JSON: '`, so
    an empty line under that prefix reads as a valid, empty manifest.
    """
    result = subprocess.run(
        ["bash", str(_REASSEMBLER)],
        input=f"{manifest.GZ_SENTINEL_PREFIX}1/1 !!!!notbase64!!!!\n",
        capture_output=True,
        text=True,
        check=False,
    )

    assert result.returncode != 0
    assert manifest.SENTINEL_PREFIX not in result.stdout
