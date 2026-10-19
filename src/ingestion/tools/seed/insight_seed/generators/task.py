"""
task-tracking silver generator: worklogs + users + field-history events.

`class_task_field_history` is the substrate the `task_issue_state` gold
model groups into per-issue records — driving bugs_fixed / tasks_closed /
on_time_count / etc. Every issue gets one row per relevant field (status,
assignee, issuetype, priority, duedate, timeoriginalestimate, timespent)
tagged event_kind='synthetic_initial'; closed issues add a follow-up
'changelog' row flipping status to 'Closed'.

Everyone except sales (light) tracks tasks. Support team gets extra
volume + a `data_source='zendesk-placeholder'` marker, since there's
no real Zendesk connector in the repo yet — the marker exists so the
distinction is visible in the silver data even though no production
Zendesk feed exists.
"""

from __future__ import annotations

import datetime as _dt
import json
from collections.abc import Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING

from ..profiles import TEAM_PROFILES, Person
from .base import (
    UTC,
    anchor_date,
    anchor_datetime,
    days_window,
    deterministic_int,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    weekday_multiplier,
)
from .insert import bulk_insert, truncate

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


def task_persons(roster: Sequence[Person]) -> list[Person]:
    return [
        p
        for p in roster
        if p.team
        and (
            TEAM_PROFILES[p.team].weights.get("jira", 0) > 0
            or TEAM_PROFILES[p.team].weights.get("zendesk-placeholder", 0) > 0
        )
    ]


def task_weight(team: str) -> float:
    """The dominant task-tracking weight for a team — jira or the zendesk
    placeholder, whichever the team actually lives in."""
    weights = TEAM_PROFILES[team].weights
    return max(weights.get("jira", 0), weights.get("zendesk-placeholder", 0))


_ISSUE_TYPES = ("Bug", "Task", "Story", "Improvement")

_ISSUE_TYPE_DIM = {
    # issue_type_name: (issue_type_id, issue_kind)
    "Bug": ("10004", "bug"),
    "Task": ("10001", "task"),
    "Story": ("10002", "task"),
    "Improvement": ("10003", "task"),
}
_PRIORITIES = ("Highest", "High", "Medium", "Medium", "Low")
_CLOSE_STATUSES = ("Closed", "Resolved", "Verified")

# Resolution decisions for config.field_value_map. There is no raw resolution
# catalogue: classification is id-keyed and nothing displays the Jira names.
# Seeded issues carry no resolution field-history events (adding a draw would
# re-deal the rng wire format), so gold classifies them 'unknown'; the rows
# exist so stands exercise the resolution config path end to end.
_RESOLUTION_DIM = {
    # display_name: (resolution_id, resolution_kind)
    "Fixed": ("1", "fixed"),
    "Won't Fix": ("2", "wontfix"),
    "Duplicate": ("3", "duplicate"),
}

# Status dimension. The task_issue_state gold model resolves a status to a
# lifecycle category by joining class_task_statuses on
# (insight_source_id, status_id), and gold detects a closed task via
# status_category = 'done' (never a localized name — see issue #1541). So
# status field-history events MUST carry a status_id (value_ids[1]) that
# matches a class_task_statuses row, and that dimension must be seeded.
# status_category values are the reconciled set: new / in_progress / done.
_STATUS_DIM = {
    # status_name: (status_id, status_category)
    "To Do": ("1", "new"),
    "Closed": ("6", "done"),
    "Resolved": ("5", "done"),
    "Verified": ("10001", "done"),
}
_STATUS_CATEGORY_ID = {"new": 2, "in_progress": 4, "done": 3, "undefined": 1}

# Per-team data_source for task-tracking rows. Support uses the
# `zendesk-placeholder` marker — there's no real Zendesk connector in
# the repo so this keeps the per-team distinction visible without
# pretending to be Jira. Everyone else lives in Jira.
_TASK_DATA_SOURCE = {"support": "zendesk-placeholder"}
_DEFAULT_TASK_DATA_SOURCE = "jira"


def _task_data_source(team: str | None) -> str:
    return _TASK_DATA_SOURCE.get(team or "", _DEFAULT_TASK_DATA_SOURCE)


def _value_id_type(field_id: str) -> str:
    """`assignee` rows carry an account-id; everything else is a literal
    (status names, issue types, due-date strings, time-in-seconds). The
    downstream task-current-state MV reads the typing to decide how to
    resolve the value against class_task_users."""
    return "account_id" if field_id == "assignee" else "string_literal"


_EVENT_BANDS = {"synthetic_initial": 0, "changelog": 1}
_OBSERVED_BAND = 2


def _event_order(event_at: _dt.datetime, event_kind: str, rank: int) -> int:
    """The class's ordering key, as the `task_event_order` dbt macro composes it."""
    epoch_ms = int(event_at.timestamp() * 1000)
    band = _EVENT_BANDS.get(event_kind, _OBSERVED_BAND)
    return epoch_ms * 1_000_000 + band * 100_000 + rank


def _fh_row(
    *,
    tenant_uuid: str,
    src_id: str,
    data_source: str,
    issue_id: str,
    event_at: _dt.datetime,
    event_kind: str,
    field_id: str,
    field_name: str,
    value_id: str | None,
    value_display: str | None,
    author_id: str,
    seq: int,
) -> tuple[object, ...]:
    """Build one class_task_field_history row in column order."""
    value_ids = [value_id] if value_id is not None else []
    value_displays = [value_display] if value_display is not None else []
    event_id = deterministic_uuid("task.fh", issue_id, field_id, str(seq))
    return (
        deterministic_uuid("task.fh.uk", issue_id, field_id, str(seq)),
        src_id,
        data_source,
        issue_id,
        f"INSIGHT-{issue_id[-4:]}",
        event_id,
        event_at,
        event_kind,
        seq,
        _event_order(event_at, event_kind, seq),
        author_id,
        field_id,
        field_name,
        "single",
        "set",
        value_ids,
        value_displays,
        _value_id_type(field_id),
        event_at,
        1,
    )


@dataclass(frozen=True)
class IssuePlan:
    """One issue's whole deterministic lifecycle, as the day's rng draws it.

    The single source both the row emitter (`seed_task_field_history`) and the
    expected-value derivation (`insight_seed.golden_metrics`) read, so the
    numbers the manifest promises cannot drift from the rows the seed writes.
    """

    issue_id: str
    issue_type: str
    priority: str
    created_at: _dt.datetime
    est_seconds: float
    spent_seconds: float
    due_date: str
    close_status: str | None
    close_at: _dt.datetime | None

    @property
    def issue_kind(self) -> str:
        """The reconciled kind gold classifies this issue as ('bug' / 'task')."""
        return _ISSUE_TYPE_DIM[self.issue_type][1]


def plan_issues(
    person_uuid: str,
    weight: float,
    created_day: _dt.date,
    anchor: _dt.date,
) -> list[IssuePlan]:
    """Every issue one person opens on one day, with its optional close.

    INVARIANT: the draw order against the per-(person, day) rng is the wire
    format of the seeded data — reordering or skipping a draw silently
    re-deals every value after it and desynchronises the golden metrics from
    any stand seeded before the change.
    """
    persona = persona_multiplier(person_uuid)
    rng = seeded_rng(person_uuid, created_day, "task.fh")
    # ~0.5 new issue/business-day for medium-load persons.
    mean = 0.6 * persona * weight * weekday_multiplier(created_day)
    n_new = poisson(rng, mean)

    plans: list[IssuePlan] = []
    for i in range(n_new):
        issue_id = deterministic_uuid("task.issue", person_uuid, created_day.isoformat(), str(i))
        issue_type = _ISSUE_TYPES[rng.randint(0, len(_ISSUE_TYPES) - 1)]
        priority = _PRIORITIES[rng.randint(0, len(_PRIORITIES) - 1)]
        created_at = _dt.datetime.combine(
            created_day,
            _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59)),
        )
        est_seconds = float(rng.randint(2, 16) * 3600)
        spent_seconds = float(est_seconds * rng.uniform(0.5, 1.5))
        due_date = (created_day + _dt.timedelta(days=rng.randint(7, 30))).isoformat()

        # ~55% of issues get closed before the anchor.
        close_status: str | None = None
        close_at: _dt.datetime | None = None
        if rng.random() < 0.55:
            days_to_close = rng.randint(3, 28)
            close_day = created_day + _dt.timedelta(days=days_to_close)
            if close_day < anchor:
                close_status = _CLOSE_STATUSES[rng.randint(0, len(_CLOSE_STATUSES) - 1)]
                close_at = _dt.datetime.combine(
                    close_day,
                    _dt.time(rng.randint(10, 17), rng.randint(0, 59)),
                )

        plans.append(
            IssuePlan(
                issue_id=issue_id,
                issue_type=issue_type,
                priority=priority,
                created_at=created_at,
                est_seconds=est_seconds,
                spent_seconds=spent_seconds,
                due_date=due_date,
                close_status=close_status,
                close_at=close_at,
            )
        )
    return plans


AIRBYTE_COLS = [
    "_airbyte_raw_id",
    "_airbyte_extracted_at",
    "_airbyte_meta",
    "_airbyte_generation_id",
]

#: One Jira instance serves the install, as a real one does. The previous
#: generator gave every person their own `insight_source_id`; nothing read it
#: and no connector works that way.
JIRA_SOURCE_ID = deterministic_uuid("task.source", "jira")

#: Jira's own priority ids are numeric, and the stream types them so.
_PRIORITY_ID = {"Highest": "1", "High": "2", "Medium": "3", "Low": "4"}

PROJECT_KEY = "INSIGHT"
PROJECT_ID = "10000"

#: field_id -> (display name, Jira schema type). The schema type is what
#: `jira_field_kind` classifies on, and an unmapped one makes
#: `jira__issue_field_snapshot` raise rather than silently drop the field.
_FIELD_SCHEMA = {
    "status": ("Status", "status"),
    "assignee": ("Assignee", "user"),
    "issuetype": ("Issue Type", "issuetype"),
    "priority": ("Priority", "priority"),
    "duedate": ("Due Date", "date"),
    "timeoriginalestimate": ("Original Estimate", "number"),
    "timespent": ("Time Spent", "number"),
}


def _envelope(tenant_uuid: str) -> dict[str, object]:
    read_at = anchor_datetime()
    return {
        "_airbyte_extracted_at": read_at,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
        "tenant_id": tenant_uuid,
        "source_id": JIRA_SOURCE_ID,
        "collected_at": read_at.isoformat(),
    }


def _positioned(cols: list[str], fields: dict[str, object]) -> tuple[object, ...]:
    return tuple(fields.get(col) for col in cols)


def _account_id(person: Person) -> str:
    return deterministic_uuid("task.account", person.uuid)


#: Jira numbers issues sequentially from a per-instance counter, and both
#: `jira_id` and the readable key are numeric: `jira__issue_key_ids` keeps only
#: rows matching `^[^-]+-[0-9]+$` and `^[0-9]+$`. A hash of the plan's uuid
#: satisfies neither and collides — 4 hex characters over a few hundred issues
#: is a near-certain birthday collision, and `GROUP BY issue_key` then keeps one
#: issue per colliding key.
FIRST_ISSUE_NUMBER = 10000


def _issue_key(number: int) -> str:
    return f"{PROJECT_KEY}-{number}"


@dataclass(frozen=True)
class PlannedIssue:
    """A planned issue, the person who opened it, and its Jira number.

    The number is positional rather than derived from the plan's id: Jira's own
    are sequential, and only a sequence guarantees the uniqueness the readable
    key needs. `plan_issues` is untouched — the oracle reads it.
    """

    person: Person
    plan: IssuePlan
    number: int

    @property
    def issue_id(self) -> str:
        """What bronze carries as `jira_id`: numeric, as Jira reports it."""
        return str(self.number)

    @property
    def id_readable(self) -> str:
        return _issue_key(self.number)


def planned_issues(roster: Sequence[Person], days: int) -> list[PlannedIssue]:
    """Every issue this run writes, from the one planner the oracle reads."""
    issues: list[PlannedIssue] = []
    for person in task_persons(roster):
        weight = task_weight(person.team or "")
        if weight <= 0:
            continue
        for created_day in days_window(days):
            for plan in plan_issues(person.uuid, weight, created_day, anchor_date()):
                issues.append(PlannedIssue(person, plan, FIRST_ISSUE_NUMBER + len(issues)))
    return issues


def seed_jira_fields(client: clickhouse_connect.driver.client.Client, tenant_uuid: str) -> int:
    """The field catalogue the journal classifies every value through."""
    truncate(client, "bronze_jira", "jira_fields")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "field_id",
        "name",
        "schema_type",
        "schema_items",
        "schema_custom",
        "custom",
    ]
    rows = [
        _positioned(
            cols,
            {
                **_envelope(tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("task.field.raw", field_id),
                "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{field_id}",
                "field_id": field_id,
                "name": name,
                "schema_type": schema_type,
                "schema_items": "",
                "schema_custom": "",
                "custom": False,
            },
        )
        for field_id, (name, schema_type) in _FIELD_SCHEMA.items()
    ]
    return bulk_insert(client, "bronze_jira", "jira_fields", cols, rows)


def seed_jira_statuses(client: clickhouse_connect.driver.client.Client, tenant_uuid: str) -> int:
    truncate(client, "bronze_jira", "jira_statuses")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "status_id",
        "name",
        "category_id",
        "category_key",
        "category_name",
    ]
    rows = [
        _positioned(
            cols,
            {
                **_envelope(tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("task.status.raw", status_id),
                "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{status_id}",
                "status_id": status_id,
                "name": name,
                "category_id": _STATUS_CATEGORY_ID[category],
                "category_key": category,
                "category_name": category.replace("_", " ").title(),
            },
        )
        for name, (status_id, category) in _STATUS_DIM.items()
    ]
    return bulk_insert(client, "bronze_jira", "jira_statuses", cols, rows)


def seed_jira_issuetypes(client: clickhouse_connect.driver.client.Client, tenant_uuid: str) -> int:
    truncate(client, "bronze_jira", "jira_issuetypes")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "issuetype_id",
        "name",
        "description",
        "subtask",
        "hierarchy_level",
    ]
    rows = [
        _positioned(
            cols,
            {
                **_envelope(tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("task.type.raw", type_id),
                "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{type_id}",
                "issuetype_id": type_id,
                "name": name,
                "description": f"{name} issues",
                "subtask": False,
                "hierarchy_level": 0,
            },
        )
        for name, (type_id, _kind) in _ISSUE_TYPE_DIM.items()
    ]
    return bulk_insert(client, "bronze_jira", "jira_issuetypes", cols, rows)


def seed_jira_user(
    client: clickhouse_connect.driver.client.Client, roster: Sequence[Person], tenant_uuid: str
) -> int:
    """The directory the assignee account id resolves to a person through."""
    truncate(client, "bronze_jira", "jira_user")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "account_id",
        "email",
        "display_name",
        "account_type",
        "active",
    ]
    rows = [
        _positioned(
            cols,
            {
                **_envelope(tenant_uuid),
                "_airbyte_raw_id": deterministic_uuid("task.user.raw", person.uuid),
                "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{_account_id(person)}",
                "account_id": _account_id(person),
                "email": person.email,
                "display_name": person.display_name,
                "account_type": "atlassian",
                "active": True,
            },
        )
        for person in task_persons(roster)
    ]
    return bulk_insert(client, "bronze_jira", "jira_user", cols, rows)


def seed_jira_project(
    client: clickhouse_connect.driver.client.Client, tenant_uuid: str
) -> dict[str, int]:
    truncate(client, "bronze_jira", "jira_projects")
    truncate(client, "bronze_jira", "jira_project_visibility")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "project_id",
        "project_key",
        "name",
        "is_private",
    ]
    row = {
        **_envelope(tenant_uuid),
        "_airbyte_raw_id": deterministic_uuid("task.project.raw", PROJECT_KEY),
        "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{PROJECT_ID}",
        "project_id": PROJECT_ID,
        "project_key": PROJECT_KEY,
        "name": "Insight",
        "is_private": False,
    }
    return {
        "bronze_jira.jira_projects": bulk_insert(
            client, "bronze_jira", "jira_projects", cols, [_positioned(cols, row)]
        ),
        "bronze_jira.jira_project_visibility": bulk_insert(
            client, "bronze_jira", "jira_project_visibility", cols, [_positioned(cols, row)]
        ),
    }


def _custom_fields_json(person: Person, plan: IssuePlan) -> str:
    """The issue's field values, as Jira nests them under `fields`.

    `jira__issue_field_snapshot` unpivots exactly this object, so a field
    absent here produces no synthetic_initial row — and one not declared in
    the field catalogue makes that model raise.
    """
    # INVARIANT: the CURRENT status, not the opening one. The journal derives
    # an issue's initial status by walking the changelog backwards from this
    # snapshot, so a snapshot pinned to "To Do" contradicts the close entry and
    # the issue never reaches a done category.
    current_status = plan.close_status or "To Do"
    status_id, _category = _STATUS_DIM[current_status]
    issuetype_id, _kind = _ISSUE_TYPE_DIM[plan.issue_type]
    return json.dumps(
        {
            "status": {"id": status_id, "name": current_status},
            "assignee": {"accountId": _account_id(person), "emailAddress": person.email},
            "issuetype": {"id": issuetype_id, "name": plan.issue_type},
            "priority": {"id": _PRIORITY_ID[plan.priority], "name": plan.priority},
            "duedate": plan.due_date,
            "timeoriginalestimate": int(plan.est_seconds),
            "timespent": int(plan.spent_seconds),
        }
    )


def seed_jira_issues(
    client: clickhouse_connect.driver.client.Client,
    issues: Sequence[PlannedIssue],
    tenant_uuid: str,
) -> dict[str, int]:
    """The issues, their keys and the census of what the project holds."""
    truncate(client, "bronze_jira", "jira_issue")
    truncate(client, "bronze_jira", "jira_issue_keys")
    truncate(client, "bronze_jira", "jira_issue_census")
    issue_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "jira_id",
        "id_readable",
        "project_key",
        "status_id",
        "priority_id",
        "issuetype_id",
        "assignee_id",
        "reporter_id",
        "due_date",
        "created",
        "updated",
        "custom_fields_json",
    ]
    key_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "jira_id",
        "id_readable",
        "updated",
    ]
    census_cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "jira_id",
        "project_key",
    ]
    issue_rows: list[tuple[object, ...]] = []
    key_rows: list[tuple[object, ...]] = []
    census_rows: list[tuple[object, ...]] = []
    for planned in issues:
        plan = planned.plan
        issue_id = planned.issue_id
        readable = planned.id_readable
        updated = (plan.close_at or plan.created_at).isoformat()
        shared = {
            **_envelope(tenant_uuid),
            "jira_id": issue_id,
            "id_readable": readable,
        }
        issue_rows.append(
            _positioned(
                issue_cols,
                {
                    **shared,
                    "_airbyte_raw_id": deterministic_uuid("task.issue.raw", plan.issue_id),
                    "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{issue_id}",
                    "project_key": PROJECT_KEY,
                    # The dedicated columns carry the CURRENT state; the journal
                    # is derived from the JSON and the changelog, not from these.
                    "status_id": _STATUS_DIM[plan.close_status or "To Do"][0],
                    "priority_id": _PRIORITY_ID[plan.priority],
                    "issuetype_id": _ISSUE_TYPE_DIM[plan.issue_type][0],
                    "assignee_id": _account_id(planned.person),
                    "reporter_id": _account_id(planned.person),
                    "due_date": plan.due_date,
                    "created": plan.created_at.isoformat(),
                    "updated": updated,
                    "custom_fields_json": _custom_fields_json(planned.person, plan),
                },
            )
        )
        key_rows.append(
            _positioned(
                key_cols,
                {
                    **shared,
                    "_airbyte_raw_id": deterministic_uuid("task.issuekey.raw", plan.issue_id),
                    "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:key:{issue_id}",
                    "updated": updated,
                },
            )
        )
        census_rows.append(
            _positioned(
                census_cols,
                {
                    **shared,
                    "_airbyte_raw_id": deterministic_uuid("task.census.raw", plan.issue_id),
                    "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:census:{issue_id}",
                    "project_key": PROJECT_KEY,
                },
            )
        )
    return {
        "bronze_jira.jira_issue": bulk_insert(
            client, "bronze_jira", "jira_issue", issue_cols, issue_rows
        ),
        "bronze_jira.jira_issue_keys": bulk_insert(
            client, "bronze_jira", "jira_issue_keys", key_cols, key_rows
        ),
        "bronze_jira.jira_issue_census": bulk_insert(
            client, "bronze_jira", "jira_issue_census", census_cols, census_rows
        ),
    }


def seed_jira_issue_history(
    client: clickhouse_connect.driver.client.Client,
    issues: Sequence[PlannedIssue],
    tenant_uuid: str,
) -> int:
    """The changelog entry that closes an issue.

    One entry per closed issue, flipping `status` from To Do to its close
    status. Gold reads the close from the status category the id resolves to,
    never the name, so the `to` id is what matters.
    """
    truncate(client, "bronze_jira", "jira_issue_history")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "jira_id",
        "id_readable",
        "changelog_id",
        "author_account_id",
        "created_at",
        "items",
    ]
    rows: list[tuple[object, ...]] = []
    for planned in issues:
        plan = planned.plan
        if plan.close_at is None or plan.close_status is None:
            continue
        changelog_id = deterministic_int("task.changelog", plan.issue_id)
        rows.append(
            _positioned(
                cols,
                {
                    **_envelope(tenant_uuid),
                    "_airbyte_raw_id": deterministic_uuid("task.changelog.raw", str(changelog_id)),
                    "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{changelog_id}",
                    "jira_id": planned.issue_id,
                    "id_readable": planned.id_readable,
                    "changelog_id": changelog_id,
                    "author_account_id": _account_id(planned.person),
                    "created_at": plan.close_at.isoformat(),
                    "items": json.dumps(
                        [
                            {
                                "field": "status",
                                "fieldId": "status",
                                "fieldtype": "jira",
                                "from": _STATUS_DIM["To Do"][0],
                                "fromString": "To Do",
                                "to": _STATUS_DIM[plan.close_status][0],
                                "toString": plan.close_status,
                            }
                        ]
                    ),
                },
            )
        )
    return bulk_insert(client, "bronze_jira", "jira_issue_history", cols, rows)


def seed_jira_worklogs(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    issues: Sequence[PlannedIssue],
    tenant_uuid: str,
    days: int,
) -> int:
    """Time logged against the issues this run actually created.

    The previous generator keyed worklogs on `INSIGHT-<random>`, an issue no
    row ever carried; a bronze worklog names its issue by id and the staging
    model joins it.
    """
    truncate(client, "bronze_jira", "jira_worklogs")
    cols = [
        *AIRBYTE_COLS,
        "unique_key",
        "tenant_id",
        "source_id",
        "collected_at",
        "worklog_id",
        "jira_id",
        "id_readable",
        "author_account_id",
        "started",
        "created",
        "updated",
        "time_spent_seconds",
        "timeSpentSeconds",
    ]
    by_person: dict[str, list[PlannedIssue]] = {}
    for planned in issues:
        by_person.setdefault(planned.person.uuid, []).append(planned)

    rows: list[tuple[object, ...]] = []
    for person in task_persons(roster):
        persona = persona_multiplier(person.uuid)
        primary_w = task_weight(person.team or "")
        if primary_w <= 0:
            continue
        owned = by_person.get(person.uuid, [])
        if not owned:
            continue
        for day in days_window(days):
            rng = seeded_rng(person.uuid, day, "task.worklogs")
            mean = 4 * persona * primary_w * weekday_multiplier(day)
            n_logs = min(poisson(rng, mean), 12)
            if n_logs == 0:
                continue
            # Each worklog 15min-2h. Cap total at 8h/day.
            day_cap = 8 * 3600
            spent = 0
            for index in range(n_logs):
                if spent >= day_cap:
                    break
                duration = min(rng.randint(900, 7200), day_cap - spent)
                spent += duration
                worklog_id = deterministic_int(
                    "task.worklog", person.uuid, day.isoformat(), str(index)
                )
                # The draw that used to invent an issue key now picks one of
                # the person's own issues, so the rng sequence is unchanged.
                issue = owned[rng.randint(1000, 9999) % len(owned)]
                at = _dt.datetime.combine(day, _dt.time(12, tzinfo=UTC))
                rows.append(
                    _positioned(
                        cols,
                        {
                            **_envelope(tenant_uuid),
                            "_airbyte_raw_id": deterministic_uuid(
                                "task.worklog.raw", str(worklog_id)
                            ),
                            "unique_key": f"{tenant_uuid}:{JIRA_SOURCE_ID}:{worklog_id}",
                            "worklog_id": worklog_id,
                            "jira_id": issue.issue_id,
                            "id_readable": issue.id_readable,
                            "author_account_id": _account_id(person),
                            "started": at.isoformat(),
                            "created": at.isoformat(),
                            "updated": at.isoformat(),
                            "time_spent_seconds": duration,
                            "timeSpentSeconds": duration,
                        },
                    )
                )
    return bulk_insert(client, "bronze_jira", "jira_worklogs", cols, rows)


def seed_field_value_map(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    """Operator issue-type decisions: one config.field_value_map row per
    (source, issue type) binding the type id to its issue kind. Gold resolves
    issue_kind from these rows at its own build; without them every closed
    issue reads as `unknown` and the bug / non-bug measures stay empty. The
    matching `config.field_value_defaults` rows are seeded alongside — see
    `seed_field_value_defaults`."""
    truncate(client, "config", "field_value_map")
    cols = [
        "tenant_id",
        "insight_source_id",
        "data_source",
        "field",
        "source_key",
        "valid_from",
        "recorded_at",
        "unique_key",
        "target_value",
        "display_name",
        "is_deleted",
        "note",
        "recorded_by",
    ]
    epoch = _dt.datetime(1970, 1, 1, tzinfo=UTC)
    now = anchor_datetime()
    rows: list[tuple[object, ...]] = []
    # One pass: every task person shares the single Jira instance, so a loop
    # over the roster would write the same binding once per person.
    for src_id, data_source in ((JIRA_SOURCE_ID, _DEFAULT_TASK_DATA_SOURCE),):
        for field, dim in (("issue_type", _ISSUE_TYPE_DIM), ("resolution", _RESOLUTION_DIM)):
            for name, (source_key, kind) in dim.items():
                rows.append(
                    (
                        tenant_uuid,
                        src_id,
                        data_source,
                        field,
                        source_key,
                        epoch,
                        now,
                        deterministic_uuid("task.fieldvaluemap", src_id, field, source_key),
                        kind,
                        name,
                        0,
                        "",
                        "seed",
                    )
                )
    return bulk_insert(client, "config", "field_value_map", cols, rows)


def seed_field_value_defaults(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    """The per-source fallback decision for the classified fields: one
    config.field_value_defaults row per (task source, field) saying that a
    source key the map does not cover is `unknown`.

    The seed maps every value it emits, so nothing actually falls here — the
    rows exist because `assert_task_field_value_defaults_exist` blocks the gold
    build without them. A seeded stand carries the operator decision it would
    demand of a real one, and `unknown` is that decision stated explicitly
    rather than left to the hardcoded terminal."""
    truncate(client, "config", "field_value_defaults")
    cols = [
        "tenant_id",
        "insight_source_id",
        "field",
        "valid_from",
        "recorded_at",
        "unique_key",
        "default_value",
        "is_deleted",
        "note",
        "recorded_by",
    ]
    epoch = _dt.datetime(1970, 1, 1, tzinfo=UTC)
    now = anchor_datetime()
    rows: list[tuple[object, ...]] = []
    for src_id in (JIRA_SOURCE_ID,):
        for field in ("issue_type", "resolution"):
            rows.append(
                (
                    tenant_uuid,
                    src_id,
                    field,
                    epoch,
                    now,
                    deterministic_uuid("task.fieldvaluedefault", src_id, field),
                    "unknown",
                    0,
                    "",
                    "seed",
                )
            )
    return bulk_insert(client, "config", "field_value_defaults", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    truncate(client, "staging", "jira__field_history_derived")
    truncate(client, "staging", "jira__task_worklogs")
    # jira__task_users / _statuses / _issuetypes are views: nothing to clear,
    # and TRUNCATE refuses them.
    truncate(client, "silver", "class_task_field_history")
    truncate(client, "silver", "class_task_worklogs")
    truncate(client, "silver", "class_task_users")
    truncate(client, "silver", "class_task_statuses")
    truncate(client, "silver", "class_task_issuetypes")

    issues = planned_issues(roster, days)

    totals = {
        "bronze_jira.jira_fields": seed_jira_fields(client, tenant_uuid),
        "bronze_jira.jira_statuses": seed_jira_statuses(client, tenant_uuid),
        "bronze_jira.jira_issuetypes": seed_jira_issuetypes(client, tenant_uuid),
        "bronze_jira.jira_user": seed_jira_user(client, roster, tenant_uuid),
        "bronze_jira.jira_issue_history": seed_jira_issue_history(client, issues, tenant_uuid),
        "bronze_jira.jira_worklogs": seed_jira_worklogs(client, roster, issues, tenant_uuid, days),
    }
    totals.update(seed_jira_project(client, tenant_uuid))
    totals.update(seed_jira_issues(client, issues, tenant_uuid))

    # config.* stays a direct write: the rows are operator-authored bindings
    # from a vendor value to the role and canonical value gold consumes, and no
    # connector delivers them.
    totals["config.field_value_map"] = seed_field_value_map(client, roster, tenant_uuid)
    totals["config.field_value_defaults"] = seed_field_value_defaults(client, roster, tenant_uuid)
    return totals
