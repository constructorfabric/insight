"""Expected task-metric totals, re-derived from the generator's own plan.

The seed is deterministic, so the values the analytics API must answer for a
seeded stand are computable up front. This module re-runs the exact issue
planner the row generator uses (`generators.task.plan_issues` — same rng, same
draw order, same dimension tables) and counts what gold will count:

* `tasks_closed`   — issues whose close event lands inside the window,
* `bugs_fixed`     — the closed ones whose type reconciles to `issue_kind='bug'`
                     via `_ISSUE_TYPE_DIM`,
* `closed_non_bug` — the closed ones reconciling to `issue_kind='other'`.

Grain is per assignee email — the strongest oracle the plan supports, since the
generator assigns every issue to the person whose rng drew it and gold
attributes a closed issue to its (resolvable) assignee.

Why these three and not more: they are pure counts over the close event, keyed
by the close DATE, with every close falling strictly inside the seeded window
(a close happens at `created_day + [3..28]` and only when that is before the
anchor). Duration, estimation and staleness measures depend on wall clock,
FINAL dedup interplay or interval joins and are not reproducible from the plan
alone — see the criteria below before adding one.

Adding a metric family here requires that its expected value is (a) computable
from `plan_issues` output alone and (b) provably equal to what the gold SQL
returns for the seeded rows — nothing relative to `now()`, nothing needing a
join the plan does not model.

Pure: no environment reads, no I/O. `build_manifest` passes the roster, window
length and anchor it already resolved, which is what keeps the manifest a pure
function of its inputs.
"""

from __future__ import annotations

import datetime as _dt
from collections.abc import Sequence
from dataclasses import dataclass
from typing import TypedDict

from .generators.base import days_window
from .generators.task import plan_issues, task_persons, task_weight
from .profiles import Person

#: Bump when the golden_metrics document changes shape. The stand-suite reader
#: (tests/lib/insight_stand/manifest.py) skips, rather than misreads, a version
#: it does not understand.
GOLDEN_METRICS_VERSION = 1


class GoldenTaskTotals(TypedDict):
    tasks_closed: int
    bugs_fixed: int
    closed_non_bug: int


class GoldenTasks(TypedDict):
    #: `from..to`, close-date grain: every counted close lands inside it.
    window: str
    per_person: dict[str, GoldenTaskTotals]


class GoldenMetricsDoc(TypedDict):
    version: int
    tasks: GoldenTasks


@dataclass(frozen=True)
class TaskTotals:
    tasks_closed: int
    bugs_fixed: int
    closed_non_bug: int


def task_totals(
    roster: Sequence[Person],
    days: int,
    anchor: _dt.date,
) -> dict[str, TaskTotals]:
    """Per-assignee-email closed-issue totals over the seeded window.

    Counts by close date, which is the `metric_date` gold stamps on the
    `tasks_closed` / `bugs_fixed` / `closed_non_bug` evidence rows.
    """
    window = days_window(days, end=anchor + _dt.timedelta(days=1))

    totals: dict[str, TaskTotals] = {}
    for person in task_persons(roster):
        weight = task_weight(person.team or "")
        closed = bugs = non_bug = 0
        for created_day in window:
            for plan in plan_issues(person.uuid, weight, created_day, anchor):
                if plan.close_at is None:
                    continue
                closed += 1
                if plan.issue_kind == "bug":
                    bugs += 1
                elif plan.issue_kind == "other":
                    non_bug += 1
        totals[person.email] = TaskTotals(
            tasks_closed=closed, bugs_fixed=bugs, closed_non_bug=non_bug
        )
    return totals


def build_golden_metrics(
    roster: Sequence[Person],
    days: int,
    anchor: _dt.date,
) -> GoldenMetricsDoc:
    """The manifest's `golden_metrics` document."""
    window_start = anchor - _dt.timedelta(days=days - 1)

    per_person: dict[str, GoldenTaskTotals] = {
        email: {
            "tasks_closed": t.tasks_closed,
            "bugs_fixed": t.bugs_fixed,
            "closed_non_bug": t.closed_non_bug,
        }
        for email, t in task_totals(roster, days, anchor).items()
    }
    return {
        "version": GOLDEN_METRICS_VERSION,
        "tasks": {
            "window": f"{window_start.isoformat()}..{anchor.isoformat()}",
            "per_person": per_person,
        },
    }
