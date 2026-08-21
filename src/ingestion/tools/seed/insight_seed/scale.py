"""Grow the demo roster past its committed size, by appending only.

Imported only when `SEED_ORG_HEADCOUNT` asks for more people than the
committed roster holds, so the default seed stays byte-identical to the one CI
asserts against. The committed people keep their uuids, emails, names and
build order — `manifest._fixtures` resolves them by uuid.

Growth people take namespaces of their own for uuid, email and display name:
dbt's `resolve_person_id` resolves a duplicated email to NOBODY, silently
dropping both people from every metric. Growth splits round-robin over the
existing teams as squads of at most `SQUAD_SPAN` ICs under a squad manager
reporting to the team's existing lead. See the seeder README for the shape.
"""

from __future__ import annotations

from collections import Counter
from collections.abc import Sequence
from dataclasses import dataclass

from . import config, profiles
from .profiles import Person

#: Most growth ICs under one squad manager; also drives the manager/IC split.
SQUAD_SPAN = 8

#: Existing teams only: a team absent from `TEAM_PROFILES` seeds people who
#: generate no rows.
GROWTH_TEAMS: tuple[str, ...] = ("development", "sales", "hr", "support")

#: The EXISTING `lead` role: several tables index `Person.role` directly, and
#: an unknown role string raises `KeyError` mid-seed.
SQUAD_MANAGER_ROLE = "lead"

#: Growth uuids get their own block; `a`/`b`/`c`/`d` are taken.
_UUID_PREFIX = "eeeeeeee-0000-0000-0000-"

#: INVARIANT: this seed plus the exact Faker pin in pyproject keep growth
#: names stable across runs; changing either renames a re-seeded stand's
#: whole growth population.
_BULK_NAME_SEED = 0x5EED

#: Draws before concluding Faker's surname pool cannot fill the name grid.
_MAX_NAME_DRAWS = 100_000


def _generate_bulk_last_names() -> list[str]:
    """Surnames for the growth population, drawn from Faker deterministically.

    Each draw is filtered against `profiles._LAST_NAMES` and deduplicated, so
    the properties the module docstring claims — disjoint from the committed
    surnames, no duplicates — hold by construction. Drawing continues until
    the first-name grid can name a roster at the ceiling.
    """
    from faker import Faker

    fake = Faker("en_US")
    fake.seed_instance(_BULK_NAME_SEED)

    committed = set(profiles._LAST_NAMES)
    names: list[str] = []
    seen: set[str] = set()
    needed = config.MAX_ORG_HEADCOUNT - config.CANONICAL_ROSTER_SIZE

    for _ in range(_MAX_NAME_DRAWS):
        if len(profiles._FIRST_NAMES) * len(names) >= needed:
            return names
        surname = fake.last_name()
        if surname in committed or surname in seen:
            continue
        seen.add(surname)
        names.append(surname)

    raise RuntimeError(
        f"Faker yielded only {len(names)} usable surnames in {_MAX_NAME_DRAWS} draws, "
        f"short of what a roster at the {config.MAX_ORG_HEADCOUNT}-person ceiling needs."
    )


_BULK_LAST_NAMES = _generate_bulk_last_names()


@dataclass(frozen=True)
class TeamPlan:
    """How many growth people one team takes, and in what shape."""

    team: str
    managers: int
    ics: int

    @property
    def total(self) -> int:
        return self.managers + self.ics


def _managers_for(slots: int) -> int:
    """Squad managers needed so no squad carries more than `SQUAD_SPAN` ICs."""
    squad = SQUAD_SPAN + 1
    return (slots + squad - 1) // squad


def plan(total_target: int, teams: Sequence[str] = GROWTH_TEAMS) -> list[TeamPlan]:
    """Split the growth over the teams, and each team's share into a squad shape.

    Round-robin rather than proportional: the four teams are equal-sized in the
    committed roster, so equal shares keep them that way, and the remainder goes
    to the first teams in order rather than being dropped — the totals must add
    up to `total_target` exactly.

    Within a team, the slots divide into squads of one manager plus at most
    `SQUAD_SPAN` ICs. One slot therefore buys a manager with no reports yet,
    which is the honest shape of a team that just started growing.
    """
    if not teams:
        raise ValueError("plan needs at least one team to grow.")
    grow = total_target - config.CANONICAL_ROSTER_SIZE
    if grow < 0:
        raise ValueError(
            f"total_target={total_target} is below the committed roster's "
            f"{config.CANONICAL_ROSTER_SIZE} people; the roster only grows."
        )

    plans: list[TeamPlan] = []
    for index, team in enumerate(teams):
        slots = grow // len(teams) + (1 if index < grow % len(teams) else 0)
        managers = _managers_for(slots)
        plans.append(TeamPlan(team=team, managers=managers, ics=slots - managers))
    return plans


def _growth_name(index: int) -> tuple[str, str]:
    """Name for the `index`-th growth person, 0-based across every team.

    First names cycle fastest so consecutive people in one squad differ by first
    name; the surname advances once per full cycle. The pair is unique for any
    index a roster at the ceiling can reach — `_generate_bulk_last_names` sizes
    the grid to exactly that.
    """
    firsts = profiles._FIRST_NAMES
    return (
        firsts[index % len(firsts)],
        _BULK_LAST_NAMES[(index // len(firsts)) % len(_BULK_LAST_NAMES)],
    )


def extend_roster(base: list[Person], total_target: int) -> list[Person]:
    """`base` plus enough growth people to reach exactly `total_target`.

    The result starts with `base`, element for element. Every invariant the
    callers depend on is checked HERE rather than only in the tests: this
    function runs inside a Job that writes to a real stand, and a duplicated
    email resolves to nobody in gold instead of failing loudly.
    """
    if len(base) != config.CANONICAL_ROSTER_SIZE:
        raise RuntimeError(
            f"extend_roster expects the committed {config.CANONICAL_ROSTER_SIZE}-person "
            f"roster, got {len(base)}. Growth is appended to it, so a different base "
            "means the plan's arithmetic no longer describes the result."
        )
    if total_target <= len(base):
        raise RuntimeError(
            f"extend_roster was asked for {total_target} people, which is not more than "
            f"the {len(base)} it starts from; the inert path belongs in "
            "`profiles.build_seeded_roster`."
        )

    leads = {p.team: p for p in base if p.role == "lead" and p.team is not None}
    grown = list(base)
    index = 0  # 0-based across every team: drives names; +1 is the uuid sequence

    for team_plan in plan(total_target):
        lead = leads.get(team_plan.team)
        if lead is None:
            raise RuntimeError(
                f"team {team_plan.team!r} has no lead in the committed roster; growth "
                "reports to the existing leads and cannot invent one."
            )

        managers: list[Person] = []
        for n in range(1, team_plan.managers + 1):
            first, last = _growth_name(index)
            index += 1
            managers.append(
                Person(
                    uuid=f"{_UUID_PREFIX}{index:012d}",
                    email=profiles.build_email(f"email_{team_plan.team}_mgr_{n:03d}"),
                    team=team_plan.team,
                    role=SQUAD_MANAGER_ROLE,
                    parent_uuid=lead.uuid,
                    first_name=first,
                    last_name=last,
                )
            )

        ics: list[Person] = []
        for n in range(1, team_plan.ics + 1):
            first, last = _growth_name(index)
            index += 1
            # INVARIANT: plan() keeps ics <= managers * SQUAD_SPAN, so this index
            # stays in range.
            ics.append(
                Person(
                    uuid=f"{_UUID_PREFIX}{index:012d}",
                    email=profiles.build_email(f"email_{team_plan.team}_b{n:04d}"),
                    team=team_plan.team,
                    role="ic",
                    parent_uuid=managers[(n - 1) // SQUAD_SPAN].uuid,
                    first_name=first,
                    last_name=last,
                )
            )

        grown += managers + ics

    _assert_grown(base, grown, total_target)
    return grown


def _assert_grown(base: list[Person], grown: list[Person], total_target: int) -> None:
    """Every property the rest of the seeder is allowed to assume."""
    if len(grown) != total_target:
        raise RuntimeError(f"grown roster holds {len(grown)} people, expected {total_target}.")
    if grown[: len(base)] != base:
        raise RuntimeError(
            "grown roster does not start with the committed roster; the named fixtures "
            "would move and every test resolving one by uuid would break."
        )
    for label, values in (
        ("uuid", [p.uuid for p in grown]),
        ("email", [p.email for p in grown]),
        ("display_name", [p.display_name for p in grown]),
    ):
        if len(set(values)) != len(grown):
            duplicates = sorted(v for v, n in Counter(values).items() if n > 1)
            raise RuntimeError(
                f"grown roster repeats {label}: {duplicates[:5]}. Two people sharing an "
                "address resolve to nobody in gold rather than failing loudly."
            )
