"""
task-tracking silver generator: worklogs + users + field-history events.

`class_task_field_history` is the substrate the
`insight.task_issue_current_state` MV groups into per-issue records
— driving bugs_fixed / tasks_closed / on_time_count / etc. Every
issue gets one row per relevant field (status, assignee, issuetype,
priority, duedate, timeoriginalestimate, timespent) tagged
event_kind='synthetic_initial'; closed issues add a follow-up
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
from typing import TYPE_CHECKING

from generators.base import (
    bulk_insert,
    days_window,
    deterministic_uuid,
    persona_multiplier,
    poisson,
    seeded_rng,
    truncate,
    weekday_multiplier,
)
from profiles import TEAM_PROFILES, Person

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


def _task_persons(roster: Sequence[Person]) -> list[Person]:
    return [
        p for p in roster
        if p.team and (
            TEAM_PROFILES[p.team].weights.get("jira", 0) > 0
            or TEAM_PROFILES[p.team].weights.get("zendesk-placeholder", 0) > 0
        )
    ]


def seed_task_worklogs(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> int:
    truncate(client, "silver", "class_task_worklogs")
    cols = [
        "insight_tenant_id", "insight_source_id", "worklog_id",
        "issue_id", "author_id", "author_email", "work_date",
        "duration_seconds", "worklog_seconds", "unique_key", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _task_persons(roster):
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
                rows.append((
                    tenant_uuid,
                    deterministic_uuid("task.source", p.uuid),
                    worklog_id, issue_id,
                    p.email, p.email, d,
                    float(duration), float(duration),
                    worklog_id, version,
                ))
    return bulk_insert(client, "silver", "class_task_worklogs", cols, rows)


def seed_task_users(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    """Required so `insight.task_worklog_seconds_per_day` (INNER JOIN
    on insight_source_id + user_id) actually emits rows."""
    truncate(client, "silver", "class_task_users")
    cols = [
        "insight_tenant_id", "insight_source_id", "user_id", "email",
        "unique_key", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    version = 1
    for p in _task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
        # author_id in class_task_worklogs == p.email — mirror that here so
        # the JOIN matches.
        rows.append((
            tenant_uuid, src_id, p.email, p.email,
            deterministic_uuid("task.user", p.uuid), version,
        ))
    return bulk_insert(client, "silver", "class_task_users", cols, rows)


_ISSUE_TYPES = ("Bug", "Task", "Story", "Improvement")
_PRIORITIES = ("Highest", "High", "Medium", "Medium", "Low")
_CLOSE_STATUSES = ("Closed", "Resolved", "Verified")

# Status dimension. The task_issue_current_state MV resolves a status to a
# lifecycle category by joining class_task_statuses on
# (insight_source_id, status_id), and gold detects a closed task via
# status_category = 'done' (never a localized name — see issue #1541). So
# status field-history events MUST carry a status_id (value_ids[1]) that
# matches a class_task_statuses row, and that dimension must be seeded.
# status_category values are the reconciled set: new / in_progress / done.
_STATUS_DIM = {
    # status_name: (status_id, status_category)
    "To Do":    ("1",     "new"),
    "Closed":   ("6",     "done"),
    "Resolved": ("5",     "done"),
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
        src_id, data_source, issue_id,
        f"INSIGHT-{issue_id[-4:]}",
        event_id, event_at, event_kind, seq,
        author_id, author_id,
        field_id, field_name, "single", "set",
        value_id, value_display,
        value_ids, value_displays,
        _value_id_type(field_id), event_at, 1,
    )


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
        "unique_key", "insight_source_id", "data_source", "issue_id",
        "id_readable", "event_id", "event_at", "event_kind", "_seq",
        "author_id", "author_display", "field_id", "field_name",
        "field_cardinality", "delta_action", "delta_value_id",
        "delta_value_display", "value_ids", "value_displays",
        "value_id_type", "collected_at", "_version",
    ]
    rows: list[tuple[object, ...]] = []
    window = days_window(days)

    for p in _task_persons(roster):
        persona = persona_multiplier(p.uuid)
        # ~0.5 new issue/business-day for medium-load persons.
        jira_w = TEAM_PROFILES[p.team or ""].weights.get("jira", 0)
        zd_w = TEAM_PROFILES[p.team or ""].weights.get("zendesk-placeholder", 0)
        weight = max(jira_w, zd_w)
        if weight <= 0:
            continue
        src_id = deterministic_uuid("task.source", p.uuid)
        data_source = _task_data_source(p.team)
        for created_day in window:
            rng = seeded_rng(p.uuid, created_day, "task.fh")
            mean = 0.6 * persona * weight * weekday_multiplier(created_day)
            n_new = poisson(rng, mean)
            for i in range(n_new):
                issue_id = deterministic_uuid("task.issue", p.uuid, created_day.isoformat(), str(i))
                issue_type = _ISSUE_TYPES[rng.randint(0, len(_ISSUE_TYPES) - 1)]
                priority = _PRIORITIES[rng.randint(0, len(_PRIORITIES) - 1)]
                created_at = _dt.datetime.combine(
                    created_day, _dt.time(9 + rng.randint(0, 8), rng.randint(0, 59)),
                )
                est_seconds = float(rng.randint(2, 16) * 3600)
                spent_seconds = float(est_seconds * rng.uniform(0.5, 1.5))
                due_date = (created_day + _dt.timedelta(days=rng.randint(7, 30))).isoformat()
                # 7 synthetic_initial rows.
                base_fields = [
                    ("status", "Status", _STATUS_DIM["To Do"][0], "To Do"),
                    ("assignee", "Assignee", p.email, p.email),
                    ("issuetype", "Issue Type", None, issue_type),
                    ("priority", "Priority", None, priority),
                    ("duedate", "Due Date", None, due_date),
                    ("timeoriginalestimate", "Original Estimate",
                     None, str(int(est_seconds))),
                    ("timespent", "Time Spent",
                     None, str(int(spent_seconds))),
                ]
                for seq, (fid, fname, vid, vdisp) in enumerate(base_fields):
                    rows.append(_fh_row(
                        tenant_uuid=tenant_uuid, src_id=src_id,
                        data_source=data_source, issue_id=issue_id,
                        event_at=created_at, event_kind="synthetic_initial",
                        field_id=fid, field_name=fname,
                        value_id=vid, value_display=vdisp,
                        author_id=p.email, seq=seq,
                    ))
                # ~55% of issues get closed before today.
                if rng.random() < 0.55:
                    days_to_close = rng.randint(3, 28)
                    close_day = created_day + _dt.timedelta(days=days_to_close)
                    today = _dt.datetime.now(_dt.UTC).date()
                    if close_day < today:
                        close_status = _CLOSE_STATUSES[rng.randint(0, len(_CLOSE_STATUSES) - 1)]
                        close_at = _dt.datetime.combine(
                            close_day, _dt.time(rng.randint(10, 17), rng.randint(0, 59)),
                        )
                        rows.append(_fh_row(
                            tenant_uuid=tenant_uuid, src_id=src_id,
                            data_source=data_source, issue_id=issue_id,
                            event_at=close_at, event_kind="changelog",
                            field_id="status", field_name="Status",
                            value_id=_STATUS_DIM[close_status][0],
                            value_display=close_status,
                            author_id=p.email, seq=100,
                        ))

    return bulk_insert(client, "silver", "class_task_field_history", cols, rows)


def seed_class_task_statuses(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
) -> int:
    """Status dimension: one row per (source, status) mapping a status_id to
    its lifecycle status_category. task_issue_current_state joins this on
    (insight_source_id, status_id); without it status_category is NULL and
    jira_closed_tasks (which filters status_category = 'done') is empty."""
    truncate(client, "silver", "class_task_statuses")
    cols = [
        "insight_source_id", "data_source", "status_id", "status_name",
        "category_id", "category_key", "status_category", "collected_at",
        "unique_key", "_version",
    ]
    now = _dt.datetime.now(_dt.UTC).replace(tzinfo=None)
    rows: list[tuple[object, ...]] = []
    for p in _task_persons(roster):
        src_id = deterministic_uuid("task.source", p.uuid)
        data_source = _task_data_source(p.team)
        for name, (status_id, category) in _STATUS_DIM.items():
            rows.append((
                src_id, data_source, status_id, name,
                _STATUS_CATEGORY_ID[category], category, category, now,
                deterministic_uuid("task.status", src_id, status_id), 1,
            ))
    return bulk_insert(client, "silver", "class_task_statuses", cols, rows)


# Refreshable materialized views that derive from class_task_field_history.
# Listed explicitly so a CH version mismatch errors loudly here rather than
# leaving downstream metrics stale until the scheduled refresh tick.
_TASK_REFRESHABLE_MVS = (
    "insight.task_issue_current_state",
    "insight.task_status_intervals",
)


def refresh_dependent_mvs(client: clickhouse_connect.driver.client.Client) -> None:
    """Force-refresh every task-pipeline refreshable MV. Default schedule
    is 1h cadence; without an explicit refresh, freshly-seeded
    class_task_field_history rows don't surface in
    insight.jira_closed_tasks / .task_delivery_bullet_rows until the
    next tick."""
    for mv in _TASK_REFRESHABLE_MVS:
        client.command(f"SYSTEM REFRESH VIEW {mv}")


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
    days: int,
) -> dict[str, int]:
    totals = {
        "silver.class_task_worklogs":      seed_task_worklogs(client, roster, tenant_uuid, days),
        "silver.class_task_users":         seed_task_users(client, roster, tenant_uuid),
        "silver.class_task_field_history": seed_task_field_history(client, roster, tenant_uuid, days),
        "silver.class_task_statuses":      seed_class_task_statuses(client, roster),
    }
    # NOTE: the refreshable MVs these rows feed (see refresh_dependent_mvs)
    # are created by apply-ch-migrations.sh, which the orchestrator runs
    # AFTER seeding — so the refresh is triggered by silver.run() once the
    # migrations exist, not here. See silver.py.
    return totals
