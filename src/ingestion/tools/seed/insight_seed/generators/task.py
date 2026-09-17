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
from collections.abc import Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING

from ..profiles import TEAM_PROFILES, Person
from .base import (
    UTC,
    anchor_date,
    anchor_datetime,
    days_window,
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


def seed_task_worklogs(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_task_worklogs")
    cols = [
        "insight_tenant_id",
        "insight_source_id",
        "worklog_id",
        "issue_id",
        "author_id",
        "author_email",
        "work_date",
        "duration_seconds",
        "worklog_seconds",
        "unique_key",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in task_persons(roster):
        persona = persona_multiplier(p.uuid)
        jira_w = TEAM_PROFILES[p.team or ""].weights.get("jira", 0)
        zendesk_w = TEAM_PROFILES[p.team or ""].weights.get("zendesk-placeholder", 0)
        # Pick the dominant data_source for the row's `insight_source_id`
        # — the support team gets zendesk-placeholder, everyone else jira.
        primary_w = max(jira_w, zendesk_w)
        if primary_w <= 0:
            continue
        for d in days_window(days):
            rng = seeded_rng(p.uuid, d, "task.worklogs")
            mean = 4 * persona * primary_w * weekday_multiplier(d)
            n_logs = min(poisson(rng, mean), 12)
            if n_logs == 0:
                continue
            # Each worklog 15min-2h. Cap total at 8h/day.
            day_cap = 8 * 3600
            spent = 0
            for i in range(n_logs):
                if spent >= day_cap:
                    break
                duration = min(rng.randint(900, 7200), day_cap - spent)
                spent += duration
                worklog_id = deterministic_uuid("task.worklog", p.uuid, d.isoformat(), str(i))
                issue_id = f"INSIGHT-{rng.randint(1000, 9999)}"
                rows.append(
                    (
                        tenant_uuid,
                        deterministic_uuid("task.source", p.uuid),
                        worklog_id,
                        issue_id,
                        p.email,
                        p.email,
                        d,
                        float(duration),
                        float(duration),
                        worklog_id,
                        version,
                    )
                )
    return bulk_insert(client, "silver", "class_task_worklogs", cols, rows)


def seed_task_users(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    """Required so the `task_worklog_flow` gold model (INNER JOIN on
    insight_source_id + user_id) actually emits rows."""
    truncate(client, "silver", "class_task_users")
    cols = [
        "tenant_id",
        "insight_source_id",
        "user_id",
        "email",
        "unique_key",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
        # author_id in class_task_worklogs == p.email — mirror that here so
        # the JOIN matches.
        rows.append(
            (
                tenant_uuid,
                src_id,
                p.email,
                p.email,
                deterministic_uuid("task.user", p.uuid),
                version,
            )
        )
    return bulk_insert(client, "silver", "class_task_users", cols, rows)


_ISSUE_TYPES = ("Bug", "Task", "Story", "Improvement")

# Issue-type dimension. issuetype field-history events MUST carry an issue type
# id (value_ids[1]) matching a seeded row, or gold reads every issue as an
# unclassified type.
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


def seed_task_field_history(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    """Synth issue lifecycle events. Each issue: 7 synthetic_initial
    rows (one per field the MV reads) + an optional 'changelog' row
    closing it."""
    truncate(client, "silver", "class_task_field_history")
    cols = [
        "unique_key",
        "insight_source_id",
        "data_source",
        "issue_id",
        "id_readable",
        "event_id",
        "event_at",
        "event_kind",
        "_seq",
        "author_id",
        "field_id",
        "field_name",
        "field_cardinality",
        "delta_action",
        "value_ids",
        "value_displays",
        "value_id_type",
        "collected_at",
        "_version",
    ]
    rows: list[tuple[object, ...]] = []
    window = days_window(days)

    for p in task_persons(roster):
        weight = task_weight(p.team or "")
        if weight <= 0:
            continue
        src_id = deterministic_uuid("task.source", p.uuid)
        data_source = _task_data_source(p.team)
        for created_day in window:
            for plan in plan_issues(p.uuid, weight, created_day, anchor_date()):
                # 7 synthetic_initial rows.
                base_fields = [
                    ("status", "Status", _STATUS_DIM["To Do"][0], "To Do"),
                    ("assignee", "Assignee", p.email, p.email),
                    (
                        "issuetype",
                        "Issue Type",
                        _ISSUE_TYPE_DIM[plan.issue_type][0],
                        plan.issue_type,
                    ),
                    ("priority", "Priority", None, plan.priority),
                    ("duedate", "Due Date", None, plan.due_date),
                    ("timeoriginalestimate", "Original Estimate", None, str(int(plan.est_seconds))),
                    ("timespent", "Time Spent", None, str(int(plan.spent_seconds))),
                ]
                for seq, (fid, fname, vid, vdisp) in enumerate(base_fields):
                    rows.append(
                        _fh_row(
                            tenant_uuid=tenant_uuid,
                            src_id=src_id,
                            data_source=data_source,
                            issue_id=plan.issue_id,
                            event_at=plan.created_at,
                            event_kind="synthetic_initial",
                            field_id=fid,
                            field_name=fname,
                            value_id=vid,
                            value_display=vdisp,
                            author_id=p.email,
                            seq=seq,
                        )
                    )
                if plan.close_status is not None and plan.close_at is not None:
                    rows.append(
                        _fh_row(
                            tenant_uuid=tenant_uuid,
                            src_id=src_id,
                            data_source=data_source,
                            issue_id=plan.issue_id,
                            event_at=plan.close_at,
                            event_kind="changelog",
                            field_id="status",
                            field_name="Status",
                            value_id=_STATUS_DIM[plan.close_status][0],
                            value_display=plan.close_status,
                            author_id=p.email,
                            seq=100,
                        )
                    )

    return bulk_insert(client, "silver", "class_task_field_history", cols, rows)


def seed_class_task_statuses(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
) -> int:
    """Status dimension: one row per (source, status) mapping a status_id to
    its lifecycle status_category. task_issue_state joins this on
    (insight_source_id, status_id); without it status_category is NULL and
    every closed-task measure (which filters status_category = 'done') is
    empty."""
    truncate(client, "silver", "class_task_statuses")
    cols = [
        "insight_source_id",
        "data_source",
        "status_id",
        "status_name",
        "category_id",
        "category_key",
        "status_category",
        "collected_at",
        "unique_key",
        "_version",
    ]
    now = anchor_datetime()
    rows: list[tuple[object, ...]] = []
    for p in task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
        data_source = _task_data_source(p.team)
        for name, (status_id, category) in _STATUS_DIM.items():
            rows.append(
                (
                    src_id,
                    data_source,
                    status_id,
                    name,
                    _STATUS_CATEGORY_ID[category],
                    category,
                    category,
                    now,
                    deterministic_uuid("task.status", src_id, status_id),
                    1,
                )
            )
    return bulk_insert(client, "silver", "class_task_statuses", cols, rows)


def seed_class_task_issuetypes(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
) -> int:
    """Issue-type dimension: one raw catalogue row per (source, issue type).
    Gold joins this on (insight_source_id, issue_type_id) for the type names;
    the kind comes from config.field_value_map (seed_field_value_map)."""
    truncate(client, "silver", "class_task_issuetypes")
    cols = [
        "unique_key",
        "insight_source_id",
        "data_source",
        "issue_type_id",
        "issue_type_name",
        "untranslated_name",
        "collected_at",
        "_version",
    ]
    now = anchor_datetime()
    rows: list[tuple[object, ...]] = []
    for p in task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
        data_source = _task_data_source(p.team)
        for name, (issue_type_id, _kind) in _ISSUE_TYPE_DIM.items():
            rows.append(
                (
                    deterministic_uuid("task.issuetype", src_id, issue_type_id),
                    src_id,
                    data_source,
                    issue_type_id,
                    name,
                    name,
                    now,
                    1,
                )
            )
    return bulk_insert(client, "silver", "class_task_issuetypes", cols, rows)


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
    for p in task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
        data_source = _task_data_source(p.team)
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
    for p in task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
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
    # NOTE: the refreshable MVs these rows feed (see refresh_dependent_mvs)
    # are created by apply-ch-migrations.sh, which the orchestrator runs
    # AFTER seeding — so the refresh is triggered by silver.run() once the
    # migrations exist, not here. See silver.py.
    return {
        "silver.class_task_worklogs": seed_task_worklogs(client, roster, tenant_uuid, days),
        "silver.class_task_users": seed_task_users(client, roster, tenant_uuid),
        "silver.class_task_field_history": seed_task_field_history(
            client, roster, tenant_uuid, days
        ),
        "silver.class_task_statuses": seed_class_task_statuses(client, roster),
        "silver.class_task_issuetypes": seed_class_task_issuetypes(client, roster),
        "config.field_value_map": seed_field_value_map(client, roster, tenant_uuid),
        "config.field_value_defaults": seed_field_value_defaults(client, roster, tenant_uuid),
    }
