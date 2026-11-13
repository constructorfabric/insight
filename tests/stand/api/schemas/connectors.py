"""Connectors response shapes — GENERATED, do not edit.

Regenerate: uv run --project tests --frozen python tests/generate_schemas.py
Source: docs/components/backend/connectors/openapi.json
"""

from __future__ import annotations

from enum import StrEnum
from typing import Any
from uuid import UUID

from pydantic import AwareDatetime, BaseModel, ConfigDict, Field


class Grain(StrEnum):
    """
    Bucket width. The set is closed and validated server-side: the view is a
    `merge()` over every bronze database, so an operator-supplied interval
    expression would be a direct route into that scan.
    """
    field_15m = '15m'
    field_1s = '1s'


class IngestionPoint(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket: str = Field(..., description="Bucket start as `YYYY-MM-DD HH:MM:SS`, always UTC — the reader's own\nzone would re-cut buckets the server already decided.")
    key: str = Field(..., description='Connector slug, stream name, or `all`, per the resolved `series`.')
    rows: int = Field(..., ge=0)


class PendingReason(StrEnum):
    no_cronworkflow = 'no_cronworkflow'
    awaiting_reconcile = 'awaiting_reconcile'
    different_schedule = 'different_schedule'


class Problem(BaseModel):
    """
    RFC 9457 problem+json. `context` varies by error category.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    context: dict[str, Any]
    detail: str
    instance: str | None = None
    status: int
    title: str
    trace_id: str | None = None
    type: str


class RequestedSchedule(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    crons: list[str]
    updated_at: AwareDatetime
    updated_by: UUID


class ScheduleOrigin(StrEnum):
    api = 'api'
    annotation = 'annotation'
    descriptor = 'descriptor'
    default = 'default'


class ScheduleRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    crons: list[str] = Field(..., max_length=10, min_length=1)


class ScheduleStatus(StrEnum):
    applied = 'applied'
    pending = 'pending'
    inherited = 'inherited'
    paused = 'paused'


class Series(StrEnum):
    """
    What one plotted band counts. `Total` exists because the full-period trend
    plots a single line: grouping it by connector would multiply 400 days of
    15-minute buckets by the connector count for a series the chart then sums
    back down anyway.
    """
    connector = 'connector'
    stream = 'stream'
    total = 'total'


class SyncFact(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    duration_ms: int | None = Field(None, description='Absent for a job still in flight, and for one the mover gave no usable\npair of stamps for. Never zero to mean absent.', ge=0)
    job_id: str = Field(..., description="The mover's own job identity.")
    records_reported: int | None = Field(None, description='What the mover states it moved. Absent where it reported no count at\nall, which is a different answer from a reported zero.', ge=0)
    started_at: str | None = Field(None, description='Absent for a job the mover had not started.')
    status: str = Field(..., description="The mover's own word for how the sync ended, or `unknown` where the\nrecorded word was outside its documented vocabulary.")


class SyncHistoryResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    connector: str
    source_id: str | None = Field(None, description="The installation's own id within that tenant. Absent with `tenant_id`.")
    syncs: list[SyncFact] = Field(..., description='A bounded window, newest first — not the full retained history.')
    tenant_id: str | None = Field(None, description='Tenant of the installation the window was narrowed to, echoed back.\nAbsent where the caller asked for the connector rather than one\ninstallation of it, in which case the window spans every instance under\nthat name.')
    window: int = Field(..., description='How many rows this window holds at most, so the page can say the list\nis a window rather than everything.', ge=0)


class AppliedSchedule(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    crons: list[str] = Field(..., description="The non-empty list recorded from the instance's `CronWorkflow`.", min_length=1)
    origin: ScheduleOrigin | None = None
    suspended: bool = Field(..., description='Whether the recorded `CronWorkflow` is suspended.')
    timezone: str | None = Field(None, description="Absent when the `CronWorkflow` uses the Argo controller's timezone.")


class IngestionIntensityResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    from_: str = Field(..., alias='from')
    grain: Grain = Field(..., description='Echoed resolved, not as asked: the caller may have pinned neither bound.')
    points: list[IngestionPoint]
    scope: str | None = Field(None, description='The `source_database` the read was scoped to; absent when org-wide.')
    series: Series
    to: str
    truncated: bool = Field(..., description='The group cap clipped the tail: the window is too wide for this grain\nand series to plot honestly. Never silently true — the UI says so.')


class InheritedSchedule(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    crons: list[str]
    origin: ScheduleOrigin


class LiveSchedule(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    crons: list[str]
    observed_at: AwareDatetime
    origin: ScheduleOrigin | None = None
    suspended: bool
    timezone: str | None = None


class ScheduleObservation(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    observed_at: AwareDatetime
    status: ScheduleStatus


class ScheduleResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    connector: str
    inherited: InheritedSchedule | None = None
    live: LiveSchedule | None = None
    next_reconcile_at: AwareDatetime | None = None
    pending_reason: PendingReason | None = None
    read_error: str | None = None
    reconcile_interval_secs: int | None = None
    requested: RequestedSchedule | None = None
    revision: int = Field(..., ge=0)
    source_id: str
    status: ScheduleStatus | None = None


class HealthScheduleRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    crons: list[str]
    last_observation: ScheduleObservation | None = None
    read_error: str | None = None
    revision: int = Field(..., ge=0)


class ConnectorHealth(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    configured: bool = Field(..., description='Present in the newest sealed snapshot of the set the controller manages.')
    connector: str
    last_sync: SyncFact | None = None
    schedule: AppliedSchedule | None = None
    schedule_request: HealthScheduleRequest | None = None
    source_id: str | None = Field(None, description="The installation's own id within that tenant. Absent with `tenant_id`.")
    tenant_id: str | None = Field(None, description='Tenant of the installation this row is. Absent together with\n`source_id` on history recorded before the ledger carried the identity\nthat no single instance could be shown to own — unattributed, which is a\ndifferent answer from attributed to something named "".')


class ConnectorHealthResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    as_of: str = Field(..., description='When this answer was computed. Dates the answer; `checked_at` dates the\nfacts in it.')
    checked_at: str | None = Field(None, description='When the mover was last read. Absent before the first sweep sealed.')
    connectors: list[ConnectorHealth]
    history_available: bool = Field(..., description='False when nothing has been recorded at all, so the page can say so\ninstead of implying health.')
    schedule_read_error: str | None = None
    typical_read_interval_ms: int | None = Field(None, description='The median gap between the recent sealed ticks. Measured, not\nconfigured — nothing on this path knows what cadence was intended.\nAbsent where too few ticks are recorded to establish one.', ge=0)
