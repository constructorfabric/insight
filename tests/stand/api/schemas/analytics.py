"""Analytics response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/analytics/openapi.json`, which is itself
generated from the analytics handlers' own types and drift-gated in CI. These
models therefore describe the structs that serialize the wire, and a validation
failure means the service and its published contract disagree — a contract test,
unlike the hand-written models in `identity.py`.

`extra="forbid"` throughout: an undeclared field is drift, and the models are
regenerated in the same change that would add one.

BODIES ONLY. This document's per-operation status-code lists are stamped
uniformly by `.standard_errors` and describe nothing (#1669), so no test takes a
status code from here — every one is asserted from observed behaviour.

ONE substitution is applied after generation: every `AwareDatetime` becomes
`UnzonedDatetime`, because the contract declares `format: date-time` while the
service serialises timestamps with no offset. See `common.UnzonedDatetime`.
"""

from __future__ import annotations

from .common import UnzonedDatetime
from pydantic import BaseModel, ConfigDict, Field, RootModel
from enum import StrEnum
from typing import Any
from uuid import UUID
from datetime import date as date_aliased


class AiConfigResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    admin_only: bool = Field(..., description='Only admins may ask for an explanation on this deployment.')
    enabled: bool = Field(..., description='Whether this deployment offers AI explanations at all.')
    model: str = Field(..., description='The model explanations are asked of.')
    stand_key: bool = Field(..., description='The stand pays for explanations with its own key, so nobody stores one.')


class AiCredentialResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    configured: bool
    hint: str = Field(..., description='Last four characters of the stored key; empty when none is stored.')


class AiSettingsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    is_default: bool = Field(..., description='True while the tenant has written none of its own.')
    system_prompt: str


class BreakdownWindowValueDto(BaseModel):
    """
    One group's reading over the comparison window.

    `value` and `present` are independent: a ratio over a group that IS in the
    window reads NULL whenever its denominator is zero, so absence cannot be
    inferred from the value. A reader that wants what a standalone request over
    that window would have returned keeps the rows with `present` and renders
    their `value` as it stands, NULL included.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    present: bool
    value: float | None = None


class Bucket(StrEnum):
    day = 'day'
    week = 'week'
    month = 'month'


class Computation(StrEnum):
    sum = 'sum'


class ComputationDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation


class Computation1(StrEnum):
    ratio = 'ratio'


class ComputationDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation1
    scale: float


class Computation2(StrEnum):
    median = 'median'


class ComputationDto3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation2


class Computation3(StrEnum):
    percentile = 'percentile'


class ComputationDto4(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation3
    q: float = Field(..., description='The quantile — a probability, matching the definition validation.', ge=0.0, le=1.0)


class Computation4(StrEnum):
    stddev = 'stddev'


class ComputationDto5(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation4


class Computation5(StrEnum):
    distinct_count = 'distinct_count'


class ComputationDto6(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation5


class ComputationDto(RootModel[ComputationDto1 | ComputationDto2 | ComputationDto3 | ComputationDto4 | ComputationDto5 | ComputationDto6]):
    root: ComputationDto1 | ComputationDto2 | ComputationDto3 | ComputationDto4 | ComputationDto5 | ComputationDto6


class CreateSavedQueryRequest(BaseModel):
    """
    Request to create a saved query.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    description: str | None = None
    name: str
    sql: str


class EntityType(StrEnum):
    person = 'person'
    tenant = 'tenant'


class EvidenceGranularity(StrEnum):
    event = 'event'
    source_summary = 'source_summary'
    derived_population = 'derived_population'


class ExplainResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    model: str = Field(..., description='The model that produced it.')
    person_context_entries: int = Field(..., description="How many of the caller's own entries fed the prompt.", ge=0)
    tenant_context_entries: int = Field(..., description='How many organisation entries fed the prompt.', ge=0)
    text: str = Field(..., description='The answer, as plain prose.')


class FeedbackEntry(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    display_name: str = Field(..., description='Empty when the sender has not been mirrored into the identity rows yet.')
    feedback_id: str
    message: str
    path: str
    person_id: str
    ts: str
    username: str = Field(..., description='The account handle, empty when no identity row carries one.')


class FeedbackListResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[FeedbackEntry]
    since: str
    until: str


class FeedbackRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    message: str
    path: str | None = Field(None, description='The screen the sender was on. Empty when the SPA cannot name one.')


class HistogramBinDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    count: int = Field(..., ge=0)
    hi: float
    lo: float


class ImportCustomMetricsResponse(BaseModel):
    """
    `POST /v1/metrics/import` result — counts landed and the `metric_key`s
    skipped because they already existed for the tenant.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    imported: int = Field(..., ge=0)
    skipped: list[str]


class MetricComputation(StrEnum):
    sum = 'sum'
    ratio = 'ratio'
    median = 'median'
    percentile = 'percentile'
    stddev = 'stddev'
    distinct_count = 'distinct_count'


class MetricDimensionDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    href: str | None = None
    key: str
    label: str | None = None
    value: str


class MetricDimensionFilterDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimension: str
    values: list[str]


class MetricDimensionFilterRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimension: str
    values: list[str]


class MetricDirection(StrEnum):
    higher_is_better = 'higher_is_better'
    lower_is_better = 'lower_is_better'
    neutral = 'neutral'


class MetricDrilldownCapability(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    granularity: list[EvidenceGranularity]


class MetricDrilldownColumnType(StrEnum):
    string = 'string'
    date = 'date'
    number = 'number'


class Type(StrEnum):
    person = 'person'


class MetricDrilldownEntity1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    id: str
    type: Type


class Type1(StrEnum):
    persons = 'persons'


class MetricDrilldownEntity2(BaseModel):
    """
    The records behind a figure a surface reports for a GROUP of people —
    an org rollup card, a team total. Every id is authorized individually,
    exactly as the single-person shape is.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    ids: list[str]
    type: Type1


class Type2(StrEnum):
    tenant = 'tenant'


class MetricDrilldownEntity3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type2


class MetricDrilldownEntity(RootModel[MetricDrilldownEntity1 | MetricDrilldownEntity2 | MetricDrilldownEntity3]):
    root: MetricDrilldownEntity1 | MetricDrilldownEntity2 | MetricDrilldownEntity3


class MetricDrilldownExportFormat(StrEnum):
    csv = 'csv'
    xlsx = 'xlsx'


class MetricDrilldownFilter(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimension: str
    values: list[str]


class MetricDrilldownPeriod(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    from_: str = Field(..., alias='from')
    to: str


class MetricDrilldownRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    cursor: str | None = None
    display_dimensions: list[str] | None = None
    entity: MetricDrilldownEntity
    filters: list[MetricDrilldownFilter] | None = None
    limit: int | None = Field(None, ge=0)
    metric_key: str
    period: MetricDrilldownPeriod


class MetricDrilldownRow(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    links: dict[str, str]
    values: dict[str, Any]


class MetricDrilldownSelection(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    display_dimensions: list[str]
    entity: MetricDrilldownEntity
    filters: list[MetricDrilldownFilter]
    metric_key: str
    period: MetricDrilldownPeriod


class MetricFormat(StrEnum):
    integer = 'integer'
    decimal = 'decimal'
    currency = 'currency'
    percent = 'percent'


class MetricGroupLimitRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    count: int = Field(..., ge=0)
    include_remainder: bool
    rank_by_metric: str | None = None


class MetricInputRole(StrEnum):
    value = 'value'
    numerator = 'numerator'
    denominator = 'denominator'


class MetricOrigin(StrEnum):
    builtin = 'builtin'
    custom = 'custom'


class Computation6(StrEnum):
    sum = 'sum'


class MetricResultDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation6


class Computation7(StrEnum):
    ratio = 'ratio'


class MetricResultDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation7
    scale: float


class Computation8(StrEnum):
    median = 'median'


class MetricResultDto3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation8


class Computation9(StrEnum):
    percentile = 'percentile'


class MetricResultDto4(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation9
    q: float = Field(..., description='The quantile — a probability, matching the definition validation.', ge=0.0, le=1.0)


class Computation10(StrEnum):
    stddev = 'stddev'


class MetricResultDto5(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation10


class Computation11(StrEnum):
    distinct_count = 'distinct_count'


class MetricResultDto6(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation11


class View(StrEnum):
    period = 'period'


class View1(StrEnum):
    timeseries = 'timeseries'


class View2(StrEnum):
    peer = 'peer'


class View3(StrEnum):
    breakdown = 'breakdown'


class View4(StrEnum):
    rollup = 'rollup'


class View5(StrEnum):
    histogram = 'histogram'


class View6(StrEnum):
    error = 'error'


class Type3(StrEnum):
    person = 'person'


class MetricResultsEntity1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    ids: list[str]
    type: Type3


class Type4(StrEnum):
    tenant = 'tenant'


class MetricResultsEntity2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type4


class MetricResultsEntity(RootModel[MetricResultsEntity1 | MetricResultsEntity2]):
    root: MetricResultsEntity1 | MetricResultsEntity2


class Type5(StrEnum):
    person = 'person'


class MetricResultsEntityDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    ids: list[str]
    type: Type5


class Type6(StrEnum):
    tenant = 'tenant'


class MetricResultsEntityDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type6


class MetricResultsEntityDto(RootModel[MetricResultsEntityDto1 | MetricResultsEntityDto2]):
    root: MetricResultsEntityDto1 | MetricResultsEntityDto2


class MetricResultsPeriod(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    from_: str = Field(..., alias='from')
    to: str


class MetricResultsPeriodDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    from_: str = Field(..., alias='from')
    to: str


class MetricSchemaErrorCode(StrEnum):
    table_not_found = 'table_not_found'
    column_not_found = 'column_not_found'
    detail_key_not_found = 'detail_key_not_found'
    dimension_not_covered = 'dimension_not_covered'
    unknown = 'unknown'


class MetricViewErrorCode(StrEnum):
    SOURCE_RELATION_MISSING = 'SOURCE_RELATION_MISSING'
    RESOURCE_EXHAUSTED = 'RESOURCE_EXHAUSTED'
    QUERY_TIMEOUT = 'QUERY_TIMEOUT'
    RESULT_PARSE_FAILED = 'RESULT_PARSE_FAILED'
    QUERY_FAILED = 'QUERY_FAILED'


class View7(StrEnum):
    period = 'period'


class MetricViewRequest1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    view: View7


class View8(StrEnum):
    peer = 'peer'


class MetricViewRequest2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    cohort_key: str | None = None
    view: View8


class View9(StrEnum):
    timeseries = 'timeseries'


class MetricViewRequest3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket: Bucket | None = None
    dimensions: list[str] | None = None
    group_limit: MetricGroupLimitRequest | None = None
    view: View9


class View10(StrEnum):
    breakdown = 'breakdown'


class MetricViewRequest4(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str]
    view: View10


class View11(StrEnum):
    rollup = 'rollup'


class MetricViewRequest5(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str]
    group_limit: MetricGroupLimitRequest | None = None
    view: View11


class View12(StrEnum):
    histogram = 'histogram'


class MetricViewRequest6(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str] | None = None
    view: View12


class MetricViewRequest(RootModel[MetricViewRequest1 | MetricViewRequest2 | MetricViewRequest3 | MetricViewRequest4 | MetricViewRequest5 | MetricViewRequest6]):
    root: MetricViewRequest1 | MetricViewRequest2 | MetricViewRequest3 | MetricViewRequest4 | MetricViewRequest5 | MetricViewRequest6


class PeerValueDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    entity_id: str
    max: float | None = None
    median: float | None = None
    min: float | None = None
    n: int = Field(..., ge=0)
    p25: float | None = None
    p75: float | None = None
    target_value: float | None = None


class PeriodValueDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare_to: float | None = Field(None, description='The same reading over `compare_to`. Omitted both when no comparison\nwindow was asked for and when the entity has no value in it — the two\nare not distinguished on the wire, and a reader that asked knows which\ncase it is in.')
    entity_id: str
    value: float | None = None


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


class PutCredentialRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    token: str


class PutSettingsRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    system_prompt: str


class RollupValueDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    contributing_entity_count: int = Field(..., ge=0)
    dimensions: list[MetricDimensionDto]
    label: str | None = None
    rank: int | None = Field(None, ge=0)
    remainder: bool | None = None
    value: float | None = None


class RunResponse(BaseModel):
    """
    Result of `POST /v1/queries/{id}/run`.

    `rows` carry a per-query dynamic schema (the `SELECT` columns vary), so each
    row is an untyped JSON object — the same shape as the metric query path.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    rows: list[Any]


class RunSavedQueryRequest(BaseModel):
    """
    Optional parameters for `POST /v1/queries/{id}/run` (#1966).

    The `{tenant}` parameter is always injected from the session context and is
    never client-settable, so it is absent here. `period` is the first optional
    named parameter an author can reference as `{period:<Type>}`; it is bound as
    a ClickHouse server-side parameter, never interpolated into the SQL text.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    period: str | None = None


class SavedQuery(BaseModel):
    """
    A saved query row.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    created_at: UnzonedDatetime
    description: str | None = None
    id: UUID
    insight_tenant_id: UUID
    name: str
    sql: str
    updated_at: UnzonedDatetime


class SavedQuerySummary(BaseModel):
    """
    Summary returned by the list endpoint (no `sql` body).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    description: str | None = None
    id: UUID
    name: str


class SchemaStatus(StrEnum):
    ok = 'ok'
    error = 'error'
    unchecked = 'unchecked'


class Scope(StrEnum):
    """
    Who a context entry belongs to.
    """
    tenant = 'tenant'
    person = 'person'


class SnapshotScope(StrEnum):
    """
    Whose reading this is.
    """
    person = 'person'
    organisation = 'organisation'


class SnapshotSeries(BaseModel):
    """
    One line of a chart, as it is drawn.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    label: str
    points: list[float | None] = Field(..., description='Readings per bucket, oldest first; a gap is null.')


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
    syncs: list[SyncFact] = Field(..., description='A bounded window, newest first — not the full retained history.')
    window: int = Field(..., description='How many rows this window holds at most, so the page can say the list\nis a window rather than everything.', ge=0)


class TelemetryRecord(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    context_app_name: str | None = None
    context_app_version: str | None = None
    context_session_id: str | None = None
    data: Any | None = None
    name: str | None = None
    time_sent: int | None = Field(None, description='Epoch milliseconds on the same clock: when the batch was flushed.')
    time_triggered: int | None = Field(None, description="Epoch milliseconds on the browser's clock: when the event happened.")


class TimeseriesPointDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket_start: str
    value: float | None = None


class UpdateContextRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    body: str | None = None
    title: str | None = None


class UpdateSavedQueryRequest(BaseModel):
    """
    Request to update a saved query.

    `description` uses double-Option (absent → unchanged, `null` → clear,
    value → set), matching [`super::metric::UpdateMetricRequest`].
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    description: str | None = None
    name: str | None = None
    sql: str | None = None


class UsageConfigResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    enabled: bool = Field(..., description='Whether this instance records usage at all.')


class UsageDay(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    day: str
    visitors: int = Field(..., ge=0)
    visits: int = Field(..., ge=0)


class UsageEvent(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    event_name: str
    opens: int = Field(..., ge=0)
    people: int = Field(..., ge=0)
    target: str


class UsageIngestRequest(BaseModel):
    """
    SDK v2 body. Fields shared by every record are hoisted out of them into
    `meta`, so a record carries only what differs.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    meta: TelemetryRecord | None = None
    records: list[TelemetryRecord] | None = None


class UsagePage(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    path: str
    views: int = Field(..., ge=0)
    visitors: int = Field(..., ge=0)


class UsagePerson(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    display_name: str = Field(..., description='Empty when the visitor has not been mirrored into the identity rows yet.')
    last_seen: str
    page_views: int = Field(..., ge=0)
    person_id: str
    username: str = Field(..., description='The account handle, empty when no identity row carries one.')
    visits: int = Field(..., ge=0)


class UsageTotals(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    page_views: int = Field(..., ge=0)
    visitors: int = Field(..., ge=0)
    visits: int = Field(..., ge=0)


class ValueTransform(BaseModel):
    """
    Affine + clamp shaping for a computed metric value:
    `y = clamp(clamp_min, clamp_max, multiplier * x + offset)`.
    Absent fields are identity (multiplier 1, offset 0, no bound).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    clamp_max: float | None = None
    clamp_min: float | None = None
    multiplier: float | None = None
    offset: float | None = None


class BreakdownValueDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare_to: BreakdownWindowValueDto | None = None
    dimensions: list[MetricDimensionDto]
    entity_id: str
    present: bool | None = Field(None, description='Whether this group has any observation inside the primary period.\nPresent only on a windowed response, where the group set spans every\nwindow and a reader has to know which of them each group belongs to.')
    value: float | None = None


class ConnectorHealth(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    configured: bool = Field(..., description='Present in the newest sealed snapshot of the set the controller manages.')
    connector: str
    last_sync: SyncFact | None = None


class ConnectorHealthResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    as_of: str = Field(..., description='When this answer was computed. Dates the answer; `checked_at` dates the\nfacts in it.')
    checked_at: str | None = Field(None, description='When the mover was last read. Absent before the first sweep sealed.')
    connectors: list[ConnectorHealth]
    history_available: bool = Field(..., description='False when nothing has been recorded at all, so the page can say so\ninstead of implying health.')
    typical_read_interval_ms: int | None = Field(None, description='The median gap between the recent sealed ticks. Measured, not\nconfigured — nothing on this path knows what cadence was intended.\nAbsent where too few ticks are recorded to establish one.', ge=0)


class ContextEntryResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    body: str
    id: str
    scope: Scope
    title: str
    updated_at: str


class ContextListResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[ContextEntryResponse]


class CreateContextRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    body: str
    scope: Scope
    title: str


class CustomMetricInput(BaseModel):
    """
    One role→measure binding of a custom metric.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    measure_key: str
    role: MetricInputRole


class CustomMetricSummary(BaseModel):
    """
    List item — display fields only, no SQL body.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: MetricComputation
    entity_type: str
    label: str
    metric_key: str
    subject: str | None = Field(None, description='Grouping subject, so the management list can partition custom metrics\nby topic like the definitions listing; absent when none is declared.')


class HistogramValueDto(BaseModel):
    """
    One histogram row. Per-entity shape: `entity_id` set, `dimensions` absent,
    every requested entity listed. Pooled shape (dimensioned request):
    `dimensions` set, `entity_id` absent, one row per observed dimension tuple
    over all selected entities' events — no entity grain, like rollup.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    bins: list[HistogramBinDto] = Field(..., description="Empty when a listed entity has no events in the period — the entity is\nstill listed, mirroring the period view's every-requested-entity rule.")
    dimensions: list[MetricDimensionDto] | None = None
    entity_id: str | None = None


class MetricDefinitionView(BaseModel):
    """
    One metric definition, display fields only.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    description: str | None = None
    dimensions: list[str]
    direction: MetricDirection
    drilldown: MetricDrilldownCapability | None = None
    entity_type: EntityType
    explanation: str | None = None
    format: MetricFormat
    is_enabled: bool
    label: str
    last_observed_date: date_aliased | None = Field(None, description="Newest `metric_date` ever observed across the definition's input\nmeasures; absent when no observation has ever been seen. Freshness\nsignal, orthogonal to `schema_status`. Not maintained for `custom`\nmetrics (see `origin`).")
    metric_key: str
    origin: MetricOrigin = Field(..., description='`builtin` metrics read managed observation relations; `custom` metrics\nexecute inline SQL at query time. The validator stamps `schema_status`\nand `last_observed_date` from materialized relations only, so for\n`custom` those fields stay `unchecked` / absent regardless of data —\nreaders must not interpret them as "never measured" for custom metrics.')
    revision_window_days: int | None = Field(None, description='How many days back from `last_observed_date` the suppliers may still\nrevise. Absent where the source declares none, and for `custom` metrics,\nwhich read no managed source — absence means "settles on arrival", not\n"revised forever". Registry knowledge, not tenant state, so it is read\nfrom the seed rather than stored per row.', ge=0)
    schema_error_code: MetricSchemaErrorCode | None = None
    schema_status: SchemaStatus
    short_label: str | None = Field(None, description='Compact label for dense surfaces; absent when the full label is\nalready compact enough.')
    subject: str | None = Field(None, description='The single topic this metric belongs to within its family, so a surface\nlisting a family can partition it into topics rather than only sorting\nby name. Exactly one per metric; absent only for metrics that declare\nnone.')
    tags: list[str] = Field(..., description='Cross-cutting labels a surface can filter or search by; many per metric,\nunlike the singular `subject`. Empty when the metric declares none.')
    unit: str | None = None


class MetricDrilldownColumn(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    key: str
    label: str
    type: MetricDrilldownColumnType


class MetricDrilldownExportRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    display_dimensions: list[str] | None = None
    entity: MetricDrilldownEntity
    filters: list[MetricDrilldownFilter] | None = None
    format: MetricDrilldownExportFormat
    metric_key: str
    period: MetricDrilldownPeriod


class MetricDrilldownResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    columns: list[MetricDrilldownColumn]
    next_cursor: str | None = None
    rows: list[MetricDrilldownRow]
    selection: MetricDrilldownSelection


class MetricRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    filters: list[MetricDimensionFilterRequest] | None = None
    metric_key: str
    views: list[MetricViewRequest]


class MetricResultSelectionDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    entity: MetricResultsEntityDto
    filters: list[MetricDimensionFilterDto]
    metric_key: str
    period: MetricResultsPeriodDto


class MetricResultViewDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    values: list[PeriodValueDto]
    view: View


class MetricResultViewDto3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    values: list[PeerValueDto]
    view: View2


class MetricResultViewDto4(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str]
    values: list[BreakdownValueDto]
    view: View3


class MetricResultViewDto5(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str]
    values: list[RollupValueDto]
    view: View4


class MetricResultViewDto6(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str] | None = Field(None, description='Present only for the pooled (dimensioned) shape; absent for the\nper-entity shape, keeping that wire form unchanged.')
    values: list[HistogramValueDto]
    view: View5


class MetricResultViewDto7(BaseModel):
    """
    This view's computation failed; sibling views and metrics are
    unaffected. `message` detail depends on the caller's role: admins get
    the underlying description, everyone else a generic one.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    code: MetricViewErrorCode
    message: str
    view: View6


class MetricResultsRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare_to: MetricResultsPeriod | None = None
    entity: MetricResultsEntity
    metrics: list[MetricRequest]
    period: MetricResultsPeriod


class MetricSnapshot(BaseModel):
    """
    The reading as the viewer sees it, handed to the model as the thing to explain.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket_starts: list[str] | None = Field(None, description='Bucket start dates every series is indexed by, oldest first.')
    delta: str | None = Field(None, description="The tile's own change line, empty when it has none.")
    help: str | None = Field(None, description="The catalog's description of the metric, empty when it has none.")
    label: str = Field(..., description='The label the tile shows.')
    metric_key: str = Field(..., description='Catalog key, e.g. `tasks.closed`.')
    peer: str | None = Field(None, description="The tile's peer-comparison line, empty when it has none.")
    period: str = Field(..., description='What the period is called on screen, e.g. `month`.')
    scope: SnapshotScope | None = Field(None, description="Whose reading this is. Absent means one person's.")
    series: list[SnapshotSeries] | None = Field(None, description="The chart's lines, when the reading is a chart rather than a tile.")
    since: str = Field(..., description='Inclusive start of the window, `YYYY-MM-DD`.')
    trend: list[float | None] | None = Field(None, description="The sparkline's readings, oldest first.")
    until: str = Field(..., description='Inclusive end of the window, `YYYY-MM-DD`.')
    value: str = Field(..., description='The formatted value the tile shows.')


class SavedQueryListResponse(BaseModel):
    """
    Response envelope for `GET /v1/queries` (`{ "items": [SavedQuerySummary] }`).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[SavedQuerySummary]


class TimeseriesDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[MetricDimensionDto]
    entity_id: str
    label: str | None = None
    points: list[TimeseriesPointDto]
    rank: int | None = Field(None, ge=0)
    remainder: bool | None = None
    total: float | None


class UsageSummaryResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    by_day: list[UsageDay]
    by_event: list[UsageEvent]
    by_page: list[UsagePage]
    by_person: list[UsagePerson]
    since: str
    totals: UsageTotals
    until: str


class CustomMetric(BaseModel):
    """
    A portable custom-metric graph — the create/update body, the export item,
    and the get/list detail shape. `origin` is response-only: always `"custom"`
    on output, omitted from exports, and ignored on input (writes force
    `custom`).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: MetricComputation
    description: str | None = None
    dimensions: list[str]
    direction: MetricDirection
    entity_type: str
    explanation: str | None = None
    format: MetricFormat
    inputs: list[CustomMetricInput]
    label: str
    measures: list[str]
    metric_key: str
    observation_sql: str
    origin: str | None = None
    peer_cohort_key: str | None = None
    scale: float | None = None
    short_label: str | None = None
    source_key: str
    subject: str | None = Field(None, description='The single topic this metric groups under within its family; a\nlowercase snake-case slug. Optional for custom metrics.')
    tags: list[str] | None = Field(None, description='Cross-cutting filter labels; lowercase snake-case slugs, unique per\nmetric. Optional — defaults to empty.')
    transform: ValueTransform | None = None
    unit: str | None = None


class CustomMetricListResponse(BaseModel):
    """
    `GET /v1/metrics` envelope.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[CustomMetricSummary]


class ExplainRequest(MetricSnapshot):
    model_config = ConfigDict(
        extra='forbid',
    )


class ExportCustomMetricsResponse(BaseModel):
    """
    `GET /v1/metrics/export` envelope — the tenant's custom metric graphs.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    metrics: list[CustomMetric]


class ImportCustomMetricsRequest(BaseModel):
    """
    `POST /v1/metrics/import` body.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    metrics: list[CustomMetric]


class MetricDefinitionListResponse(BaseModel):
    """
    Response body for `GET /v1/metric-definitions`. Metrics are sorted by
    `metric_key` ascending so the payload is byte-stable for caching and
    diff tooling.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    metrics: list[MetricDefinitionView]


class MetricResultViewDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket: Bucket
    series: list[TimeseriesDto]
    view: View1


class MetricResultViewDto(RootModel[MetricResultViewDto1 | MetricResultViewDto2 | MetricResultViewDto3 | MetricResultViewDto4 | MetricResultViewDto5 | MetricResultViewDto6 | MetricResultViewDto7]):
    root: MetricResultViewDto1 | MetricResultViewDto2 | MetricResultViewDto3 | MetricResultViewDto4 | MetricResultViewDto5 | MetricResultViewDto6 | MetricResultViewDto7


class MetricResultDto7(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    description: str | None = None
    direction: MetricDirection
    drilldown: MetricDrilldownCapability | None = None
    explanation: str | None = None
    format: MetricFormat
    label: str
    metric_key: str
    selection: MetricResultSelectionDto
    short_label: str | None = None
    unit: str | None = None
    views: list[MetricResultViewDto]


class MetricResultDto8(MetricResultDto1, MetricResultDto7):
    model_config = ConfigDict(
        extra='forbid',
    )


class MetricResultDto9(MetricResultDto2, MetricResultDto7):
    model_config = ConfigDict(
        extra='forbid',
    )


class MetricResultDto10(MetricResultDto3, MetricResultDto7):
    model_config = ConfigDict(
        extra='forbid',
    )


class MetricResultDto11(MetricResultDto4, MetricResultDto7):
    model_config = ConfigDict(
        extra='forbid',
    )


class MetricResultDto12(MetricResultDto5, MetricResultDto7):
    model_config = ConfigDict(
        extra='forbid',
    )


class MetricResultDto13(MetricResultDto6, MetricResultDto7):
    model_config = ConfigDict(
        extra='forbid',
    )


class MetricResultDto(RootModel[MetricResultDto8 | MetricResultDto9 | MetricResultDto10 | MetricResultDto11 | MetricResultDto12 | MetricResultDto13]):
    root: MetricResultDto8 | MetricResultDto9 | MetricResultDto10 | MetricResultDto11 | MetricResultDto12 | MetricResultDto13


class MetricResultsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    metrics: list[MetricResultDto]
