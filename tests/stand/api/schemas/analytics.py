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
from enum import StrEnum
from pydantic import BaseModel, ConfigDict, Field, RootModel
from typing import Any
from uuid import UUID
from datetime import date as date_aliased


class Aggregation(StrEnum):
    count = 'count'
    sum = 'sum'
    avg = 'avg'
    min = 'min'
    max = 'max'
    count_distinct = 'count_distinct'


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


class Bucket(StrEnum):
    day = 'day'
    week = 'week'
    month = 'month'


class Type(StrEnum):
    direct = 'direct'


class CatalogComputation1(BaseModel):
    """
    How the value is computed, named without the measures it is computed from.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type


class Type1(StrEnum):
    ratio = 'ratio'


class CatalogComputation2(BaseModel):
    """
    How the value is computed, named without the measures it is computed from.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type1


class Type2(StrEnum):
    percentile = 'percentile'


class CatalogComputation3(BaseModel):
    """
    How the value is computed, named without the measures it is computed from.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type2


class Type3(StrEnum):
    stddev = 'stddev'


class CatalogComputation4(BaseModel):
    """
    How the value is computed, named without the measures it is computed from.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type3


class Type4(StrEnum):
    derived = 'derived'


class CatalogComputation5(BaseModel):
    """
    How the value is computed, named without the measures it is computed from.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type4


class CatalogComputation(RootModel[CatalogComputation1 | CatalogComputation2 | CatalogComputation3 | CatalogComputation4 | CatalogComputation5]):
    root: CatalogComputation1 | CatalogComputation2 | CatalogComputation3 | CatalogComputation4 | CatalogComputation5 = Field(..., description='How the value is computed, named without the measures it is computed from.')


class CatalogDimension(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    key: str = Field(..., description='What a filter, a split or a display dimension names.')
    label: str


class ColumnKind(StrEnum):
    """
    How a column's values read, so a caller renders a page without matching key
    spellings.
    """
    text = 'text'
    number = 'number'
    date = 'date'
    timestamp = 'timestamp'


class CompareOffset(StrEnum):
    """
    How far back the compared window sits: `previous_period` shifts it by its
    own length, and a calendar offset shifts its first day back by that many
    calendar months, clamping a day the earlier month does not have to that
    month's last day. Either way the compared window spans as many days as the
    one it is compared against.
    """
    previous_period = 'previous_period'
    month = 'month'
    quarter = 'quarter'
    year = 'year'


class Comparison(BaseModel):
    """
    The compared window's value and the two ways of reading the change.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    delta: float | None = Field(None, description='Current minus compared; absent when either side is unknown.')
    ratio: float | None = Field(None, description='Current over compared; absent when the compared value is unknown or\nzero, which no ratio is defined against.')
    value: float | None = Field(None, description='What the same question answered over the compared window.')


class Type5(StrEnum):
    direct = 'direct'


class Computation1(BaseModel):
    """
    A percentile is metric-level because a percentile of pre-aggregated values
    is not a percentile; a standard deviation is metric-level for the same
    reason.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    measure: str
    type: Type5


class Type6(StrEnum):
    ratio = 'ratio'


class Computation2(BaseModel):
    """
    A percentile is metric-level because a percentile of pre-aggregated values
    is not a percentile; a standard deviation is metric-level for the same
    reason.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    denominator: str
    numerator: str
    type: Type6


class Type7(StrEnum):
    percentile = 'percentile'


class Computation3(BaseModel):
    """
    A percentile is metric-level because a percentile of pre-aggregated values
    is not a percentile; a standard deviation is metric-level for the same
    reason.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    measure: str
    quantile: float = Field(..., description='The quantile to serve, in `(0, 1)`.')
    type: Type7


class Type8(StrEnum):
    stddev = 'stddev'


class Computation4(BaseModel):
    """
    A percentile is metric-level because a percentile of pre-aggregated values
    is not a percentile; a standard deviation is metric-level for the same
    reason.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    measure: str
    type: Type8


class Type9(StrEnum):
    derived = 'derived'


class Computation5(BaseModel):
    """
    Arithmetic over measures the expression names by alias.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    expr: str
    inputs: dict[str, str] = Field(..., description='Alias to measure key. The alias is what `expr` may reference.')
    type: Type9


class Computation(RootModel[Computation1 | Computation2 | Computation3 | Computation4 | Computation5]):
    root: Computation1 | Computation2 | Computation3 | Computation4 | Computation5 = Field(..., description='A percentile is metric-level because a percentile of pre-aggregated values\nis not a percentile; a standard deviation is metric-level for the same\nreason.')


class Computation6(StrEnum):
    sum = 'sum'


class ComputationDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation6


class Computation7(StrEnum):
    ratio = 'ratio'


class ComputationDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation7
    scale: float


class Computation8(StrEnum):
    median = 'median'


class ComputationDto3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation8


class Computation9(StrEnum):
    percentile = 'percentile'


class ComputationDto4(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation9
    q: float = Field(..., description='The quantile — a probability, matching the definition validation.', ge=0.0, le=1.0)


class Computation10(StrEnum):
    stddev = 'stddev'


class ComputationDto5(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation10


class Computation11(StrEnum):
    distinct_count = 'distinct_count'


class ComputationDto6(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation11


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


class DimensionBinding(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    key: str
    label_field: str | None = None
    value_field: str


class DimensionFilter(BaseModel):
    """
    Keeps only the rows whose dimension holds one of the named values.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    dimension: str = Field(..., description="A dimension key the metric's grain measure declares.")
    values: list[str]


class Direction(StrEnum):
    higher_is_better = 'higher_is_better'
    lower_is_better = 'lower_is_better'
    neutral = 'neutral'


class DistributionQuestions(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    admitted: bool = Field(..., description="Whether the metric's computation is taken over per-row values, which is\nwhat having a distribution means.")


class EntityType(StrEnum):
    person = 'person'
    tenant = 'tenant'


class EvidenceGranularity(StrEnum):
    event = 'event'
    source_summary = 'source_summary'
    derived_population = 'derived_population'


class Executor(StrEnum):
    semantic = 'semantic'


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


class FilterOp(StrEnum):
    eq = 'eq'
    neq = 'neq'
    gt = 'gt'
    gte = 'gte'
    lt = 'lt'
    lte = 'lte'
    in_ = 'in'
    not_in = 'not_in'
    is_null = 'is_null'
    not_null = 'not_null'


class Fold(StrEnum):
    """
    Whether each subject keeps its own value or the subjects fold into one.
    """
    per_subject = 'per_subject'
    combined = 'combined'


class Format(StrEnum):
    integer = 'integer'
    decimal = 'decimal'
    currency = 'currency'
    percent = 'percent'


class Grain(StrEnum):
    """
    How finely the window is cut. `total` folds it whole; the rest report a
    point per bucket the metric observed an event in, beside the window total.
    """
    total = 'total'
    day = 'day'
    week = 'week'
    month = 'month'


class Type10(StrEnum):
    dimensions = 'dimensions'


class Type11(StrEnum):
    remainder = 'remainder'


class Group2(BaseModel):
    """
    Everything outside the groups a cap kept, folded into one.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type11


class GroupDimension(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    key: str
    label: str | None = None
    value: str


class HistogramBin(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    count: int = Field(..., ge=0)
    hi: float
    lo: float


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


class Type12(StrEnum):
    person = 'person'


class MetricDrilldownEntity1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    id: str
    type: Type12


class Type13(StrEnum):
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
    type: Type13


class Type14(StrEnum):
    tenant = 'tenant'


class MetricDrilldownEntity3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type14


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


class Computation12(StrEnum):
    sum = 'sum'


class MetricResultDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation12


class Computation13(StrEnum):
    ratio = 'ratio'


class MetricResultDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation13
    scale: float


class Computation14(StrEnum):
    median = 'median'


class MetricResultDto3(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation14


class Computation15(StrEnum):
    percentile = 'percentile'


class MetricResultDto4(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation15
    q: float = Field(..., description='The quantile — a probability, matching the definition validation.', ge=0.0, le=1.0)


class Computation16(StrEnum):
    stddev = 'stddev'


class MetricResultDto5(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation16


class Computation17(StrEnum):
    distinct_count = 'distinct_count'


class MetricResultDto6(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    computation: Computation17


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


class Type15(StrEnum):
    person = 'person'


class MetricResultsEntity1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    ids: list[str]
    type: Type15


class Type16(StrEnum):
    tenant = 'tenant'


class MetricResultsEntity2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type16


class MetricResultsEntity(RootModel[MetricResultsEntity1 | MetricResultsEntity2]):
    root: MetricResultsEntity1 | MetricResultsEntity2


class Type17(StrEnum):
    person = 'person'


class MetricResultsEntityDto1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    ids: list[str]
    type: Type17


class Type18(StrEnum):
    tenant = 'tenant'


class MetricResultsEntityDto2(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type18


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
    entity_id: str
    value: float | None = None


class Point(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    date: str = Field(..., description="The bucket's first day, `YYYY-MM-DD`.")
    value: float | None = None


class Type19(StrEnum):
    cohort = 'cohort'


class Population1(BaseModel):
    """
    Who a target is compared against, internally tagged on `type`. `cohort`
    takes the metric's own declared cohort; a metric that declares none has no
    cohort to compare within.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type19


class Type20(StrEnum):
    tenant = 'tenant'


class Population2(BaseModel):
    """
    Who a target is compared against, internally tagged on `type`. `cohort`
    takes the metric's own declared cohort; a metric that declares none has no
    cohort to compare within.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type20


class Population(RootModel[Population1 | Population2]):
    root: Population1 | Population2 = Field(..., description="Who a target is compared against, internally tagged on `type`. `cohort`\ntakes the metric's own declared cohort; a metric that declares none has no\ncohort to compare within.")


class PopulationSpread(BaseModel):
    """
    The spread of the population, withheld below the disclosure floor. The
    size is reported whatever it is, so a consumer can say why the rest is
    absent.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    max: float | None = None
    median: float | None = None
    min: float | None = None
    n: int = Field(..., description='How many of the population were observed at all.', ge=0)
    p25: float | None = None
    p75: float | None = None


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


class Quantile(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    q: float = Field(..., description='The position asked for, strictly between 0 and 1.')
    value: float | None = None


class Shape(StrEnum):
    values = 'values'


class Shape1(StrEnum):
    series = 'series'


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


class RowColumn(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    key: str
    kind: ColumnKind
    label: str = Field(..., description="The column's name as a reader sees it, derived from its key.")


class RowsQuestions(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    inputs: list[str] = Field(..., description='The parts of the computation a page of rows may be asked for.')


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


class Scalar(RootModel[bool | float | str]):
    root: bool | float | str


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


class ServedFrom(StrEnum):
    """
    Where the rows behind this answer came from.
    """
    computed = 'computed'


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


class SortDirection(StrEnum):
    """
    Which way a sorted column runs. Rows carrying no value in it are reported
    last either way.
    """
    asc = 'asc'
    desc = 'desc'


class SplitLimit(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    rank_by: str | None = Field(None, description='The metric the groups are ranked by. Defaults to the metric being read.')
    remainder: bool = Field(..., description='Whether everything outside the kept groups is one group, or dropped.')
    top: int = Field(..., description='How many groups to keep.', ge=0)


class Type21(StrEnum):
    persons = 'persons'


class Subjects1(BaseModel):
    """
    Whose values a question is about, internally tagged on `type` so a subject
    kind carries only its own fields.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    ids: list[str]
    type: Type21


class Type22(StrEnum):
    tenant = 'tenant'


class Subjects2(BaseModel):
    """
    Whose values a question is about, internally tagged on `type` so a subject
    kind carries only its own fields.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    type: Type22


class Subjects(RootModel[Subjects1 | Subjects2]):
    root: Subjects1 | Subjects2 = Field(..., description='Whose values a question is about, internally tagged on `type` so a subject\nkind carries only its own fields.')


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


class TargetComparison(BaseModel):
    """
    INVARIANT: the population is the one this target sits in, which under a
    declared cohort is that target's own and need not be another target's.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    population: PopulationSpread
    subject: str
    value: float | None = None


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


class TimeRange(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    from_: str = Field(..., alias='from', description='Inclusive first day, `YYYY-MM-DD`.')
    to: str = Field(..., description='Inclusive last day, `YYYY-MM-DD`.')


class TimeseriesPointDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket_start: str
    value: float | None = None


class Transform(BaseModel):
    """
    Post-aggregation shaping: `clamp(min, max, multiplier * value + offset)`.
    Absent fields are the identity.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    clamp_max: float | None = None
    clamp_min: float | None = None
    multiplier: float | None = None
    offset: float | None = None


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


class ValidationErrorKind(StrEnum):
    """
    Which rule was broken, as a discriminant a machine can branch on.
    """
    key_shape = 'key_shape'
    metric_key_shape = 'metric_key_shape'
    duplicate_key = 'duplicate_key'
    dataset_not_found = 'dataset_not_found'
    field_not_found = 'field_not_found'
    role_mismatch = 'role_mismatch'
    filter = 'filter'
    expression = 'expression'
    operand = 'operand'
    measure_not_found = 'measure_not_found'
    quantile_out_of_range = 'quantile_out_of_range'
    mixed_datasets = 'mixed_datasets'
    distribution_without_value = 'distribution_without_value'
    no_derived_inputs = 'no_derived_inputs'
    metric_expression = 'metric_expression'
    unknown_derived_input = 'unknown_derived_input'
    unused_derived_input = 'unused_derived_input'
    dimension_bindings_disagree = 'dimension_bindings_disagree'


class ValidationFailure(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    kind: ValidationErrorKind
    message: str


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


class ValuesQuestions(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare: list[CompareOffset] = Field(..., description='The earlier windows the same question may be set beside.')
    folds: list[Fold]
    grains: list[Grain]
    split: bool = Field(..., description='Whether the metric declares a dimension to break its value out by.')


class BreakdownValueDto(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[MetricDimensionDto]
    entity_id: str
    value: float | None = None


class Compare(BaseModel):
    """
    Asks the same question again over an earlier window, and reports the change.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    offset: CompareOffset


class ComparisonQuery(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    filters: list[DimensionFilter] | None = Field(None, description='Narrows every measure the metric reads, for the targets and the\npopulation alike. Absent means no narrowing.')
    metric: str = Field(..., description='The metric key the semantic definitions carry, such as `git.commits`.')
    population: Population
    targets: list[str] = Field(..., description='The people the answer reports a value for.')
    time: TimeRange


class ComparisonQuestions(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    populations: list[Population] = Field(..., description="Written as a comparison question's `population` field takes them.")


class ComparisonsRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    queries: list[ComparisonQuery]


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


class DistributionQuery(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bins: int | None = Field(None, description="How many bins each subject's own range is cut into. Absent means ten,\nunless the question asks for quantiles alone.", ge=0)
    filters: list[DimensionFilter] | None = Field(None, description='Narrows every measure the metric reads. Absent means no narrowing.')
    metric: str = Field(..., description="The metric key the semantic definitions carry. Only a metric whose\ncomputation is taken over its measure's own per-row values has a\ndistribution.")
    quantiles: list[float] | None = Field(None, description='The positions to report, each strictly between 0 and 1. Absent means\nnone are.')
    subjects: Subjects
    time: TimeRange


class DistributionsRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    queries: list[DistributionQuery]


class FilterValue(RootModel[Scalar | list[Scalar]]):
    root: Scalar | list[Scalar]


class Group1(BaseModel):
    """
    Which slice of the split a row belongs to.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[GroupDimension]
    type: Type10


class Group(RootModel[Group1 | Group2]):
    root: Group1 | Group2 = Field(..., description='Which slice of the split a row belongs to.')


class GroupedSeries(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare: Comparison | None = None
    group: Group | None = None
    points: list[Point]
    subject: str | None = None
    total: float | None = Field(None, description='The whole window folded once, not the sum of the points.')


class GroupedValue(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare: Comparison | None = None
    group: Group | None = None
    subject: str | None = Field(None, description='Absent when the subjects folded into one value.')
    value: float | None = None


class Histogram(BaseModel):
    """
    The subject's own range, cut into bins of equal width. INVARIANT: the last
    bin closes on the maximum rather than opening one more.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    bins: list[HistogramBin] = Field(..., description='Empty when the subject was observed for no event in the window.')
    hi: float | None = Field(None, description='The largest value observed; absent when nothing was.')
    lo: float | None = Field(None, description='The smallest value observed; absent when nothing was.')


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


class MetricDefinition(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    cohort_key: str | None = None
    computation: Computation
    description: str | None = None
    direction: Direction
    entity_type: str
    format: Format
    key: str
    label: str | None = None
    transform: Transform | None = None


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


class MetricQuestions(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    comparisons: ComparisonQuestions
    distributions: DistributionQuestions
    rows: RowsQuestions
    values: ValuesQuestions


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


class Provenance(BaseModel):
    """
    What produced this answer.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    definition_version: int | None = Field(None, description='The definition version the store holds; absent when it carries no row.')
    executor: Executor
    served_from: ServedFrom


class ResultBody1(BaseModel):
    """
    The answer's shape, decided by the question's grain: `total` answers with
    values, every other grain answers with series.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    shape: Shape
    values: list[GroupedValue]


class ResultBody2(BaseModel):
    """
    The answer's shape, decided by the question's grain: `total` answers with
    values, every other grain answers with series.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    series: list[GroupedSeries]
    shape: Shape1


class ResultBody(RootModel[ResultBody1 | ResultBody2]):
    root: ResultBody1 | ResultBody2 = Field(..., description="The answer's shape, decided by the question's grain: `total` answers with\nvalues, every other grain answers with series.")


class RowSort(BaseModel):
    """
    Orders a page by one of the columns it reports.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    column: str = Field(..., description='A column the page reports, spelled as its `columns[].key`.')
    direction: SortDirection


class RowsRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    cursor: str | None = Field(None, description='Where to resume, as the previous page reported it. Absent asks for the\nfirst page.')
    display_dimensions: list[str] | None = Field(None, description="Dimension keys to report beyond the ones the metric's measure declares.")
    filters: list[DimensionFilter] | None = Field(None, description='Narrows the scan exactly as it narrows the value. Absent means none.')
    input: str | None = Field(None, description="Which input of the metric's computation to page. A metric composing one\ninput needs none; one composing several names which.")
    metric: str = Field(..., description='The metric key the semantic definitions carry, such as `git.commits`.')
    page_size: int | None = Field(None, description='Rows per page. Absent means 100.', ge=0)
    sort: RowSort | None = None
    subjects: Subjects
    time: TimeRange


class RowsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    columns: list[RowColumn]
    input: str = Field(..., description="The part of the metric's computation these rows were folded into.")
    metric: str
    next_cursor: str | None = Field(None, description='Absent when this page is the last one.')
    provenance: Provenance
    rows: list[list[Any]] = Field(..., description="One entry per row, holding one value per column, in the columns' order.")


class SavedQueryListResponse(BaseModel):
    """
    Response envelope for `GET /v1/queries` (`{ "items": [SavedQuerySummary] }`).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[SavedQuerySummary]


class Split(BaseModel):
    """
    Which dimensions the value is broken out by, and how many of their groups
    the answer keeps.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    dimensions: list[str]
    limit: SplitLimit | None = None


class SubjectDistribution(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    histogram: Histogram | None = None
    quantiles: list[Quantile] | None = Field(None, description='Absent when the question named no quantiles.')
    subject: str


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


class ValidateDefinitionsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    errors: list[ValidationFailure] = Field(..., description='Every offender, not the first one.')
    valid: bool


class ValuesQuery(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    compare: Compare | None = None
    filters: list[DimensionFilter] | None = Field(None, description='Narrows every measure the metric reads. Absent means no narrowing.')
    fold: Fold
    metric: str = Field(..., description='The metric key the semantic definitions carry, such as `git.commits`.')
    split: Split | None = None
    subjects: Subjects
    time: TimeRange


class ValuesRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    queries: list[ValuesQuery]


class CatalogMetric(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    cohort_key: str | None = Field(None, description='The grouping a cohort comparison reads; absent when the metric declares\nnone, and then no cohort comparison is offered.')
    computation: CatalogComputation
    description: str | None = None
    dimensions: list[CatalogDimension]
    direction: Direction
    entity_type: str = Field(..., description="What the metric's values are keyed by, such as `person`.")
    format: Format
    key: str = Field(..., description='The key a question names, such as `git.commits`.')
    label: str | None = None
    questions: MetricQuestions


class ComparisonResult(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    metric: str
    provenance: Provenance
    targets: list[TargetComparison] = Field(..., description='One entry per requested target, in the order they were asked.')


class ComparisonsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    results: list[ComparisonResult] = Field(..., description='One entry per requested query, in the order they were asked.')


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


class DistributionResult(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    metric: str
    provenance: Provenance
    subjects: list[SubjectDistribution] = Field(..., description='One entry per requested subject, in the order they were asked.')


class DistributionsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    results: list[DistributionResult] = Field(..., description='One entry per requested query, in the order they were asked.')


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


class FilterLeaf(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    field: str
    op: FilterOp
    value: FilterValue | None = None


class ImportCustomMetricsRequest(BaseModel):
    """
    `POST /v1/metrics/import` body.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    metrics: list[CustomMetric]


class MetricCatalogResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    metrics: list[CatalogMetric] = Field(..., description='Every metric the definitions carry, in key order.')


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


class QueryResult(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    metric: str
    provenance: Provenance
    result: ResultBody


class ValuesResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    results: list[QueryResult] = Field(..., description='One entry per requested query, in the order they were asked.')


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


class AllNode(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    all: list[FilterTree]


class AnyNode(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    any: list[FilterTree]


class MeasureDefinition(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    aggregation: Aggregation
    dataset: str
    description: str | None = None
    dimensions: list[DimensionBinding] | None = None
    entity: str
    event_time: str
    filter: FilterTree | None = None
    key: str
    subject_expr: str | None = Field(None, description='What `count_distinct` counts one of.')
    value_expr: str | None = Field(None, description='The operand for the numeric folds; absent for `count`/`count_distinct`.')


class NotNode(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    not_: FilterTree = Field(..., alias='not')


class ValidateDefinitionsRequest(BaseModel):
    """
    Definitions to judge, in the shape the authored YAML parses into.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    measures: list[MeasureDefinition] | None = None
    metrics: list[MetricDefinition] | None = None


class FilterTree(RootModel[AllNode | AnyNode | NotNode | FilterLeaf]):
    root: AllNode | AnyNode | NotNode | FilterLeaf


AllNode.model_rebuild()
AnyNode.model_rebuild()
MeasureDefinition.model_rebuild()
NotNode.model_rebuild()
