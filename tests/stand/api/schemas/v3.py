"""insight-v3-core response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/insight-v3-core/openapi.json`, generated offline
by `cargo run -p insight-v3-core -- openapi` from the same registrations that
serve the routes, and drift-gated in CI. These models describe the structs that
serialize the wire.

The document types the alert operations' bodies only. Every other JSON success
body it declares as a bare `type: object`, so those are modelled by hand in
`v3_untyped.py`; `generate_schemas.py --check` fails the day the document types
one of them, or adds an untyped one.
"""

from __future__ import annotations

from enum import StrEnum
from typing import Any

from pydantic import BaseModel, ConfigDict, Field, RootModel


class CountShape(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    op: str = Field(..., description='One of `>`, `>=`, `<`, `<=`, `==`, `!=`.')
    value: int = Field(..., ge=0)


class DefinitionRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    body: Any = Field(..., description='The definition body.')
    name: str = Field(..., description='What a reader calls it: trimmed, 1 to 128 characters, and shared by\nas many definitions as like.')


class DestinationResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str
    provider: str


class DestinationsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    destinations: list[DestinationResponse]


class Direction(StrEnum):
    asc = 'asc'
    desc = 'desc'


class DuplicateRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str = Field(..., description='What the copy is called: trimmed, 1 to 128 characters.')


class EnabledRequest(BaseModel):
    """
    Turning a rule on or off names the revision it expects to change.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    expected_revision: int = Field(..., ge=0)


class GrantRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    level: str = Field(..., description='`viewer` or `editor`. An editor who is not an administrator only\nviews.')


class NameRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str = Field(..., description='Trimmed, then 1 to 64 characters.')


class OperatorShape(StrEnum):
    field_ = '>'
    field__ = '>='
    field__1 = '<'
    field___1 = '<='
    field___2 = '=='
    field___3 = '!='
    between = 'between'
    contains = 'contains'
    starts_with = 'starts_with'
    ends_with = 'ends_with'
    in_ = 'in'
    not_in = 'not_in'
    before = 'before'
    after = 'after'
    older_than = 'older_than'
    newer_than = 'newer_than'


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


class RawDataRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    dataset: str = Field(..., description='The dataset this record belongs to, which must already be declared.')
    raw_data: Any


class RowsShape1(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    count: CountShape


class RowsWord(StrEnum):
    one = 'one'
    any = 'any'
    all = 'all'
    none = 'none'


class SortBody(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    direction: Direction
    key: str


class SummaryResponse(BaseModel):
    """
    One alert as a listing shows it.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    enabled: bool
    id: str
    metric: str
    name: str


class TagsRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    tags: list[str] = Field(..., description='Every tag the dashboard carries from now on, each trimmed to 1 to 32\ncharacters, at most 10; `[]` clears them.')


class TransferRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    to: str = Field(..., description='The person id of the current administrator to own it from now on.')


class Turn(BaseModel):
    """
    One turn of the conversation so far. The reader's panel keeps the thread
    and sends it back, because the service stores no session.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    content: str
    role: str = Field(..., description='`user` or `assistant`; anything else is dropped before the call.')


class Value(RootModel[Any]):
    root: Any


class WitnessResponse(BaseModel):
    """
    The rows a check judged: how many passed of how many, and up to five of
    them, holding the metric's group-by columns, then the columns the
    condition reads, then the rule's context columns.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    answered: int = Field(..., ge=0)
    columns: list[str]
    matched: int = Field(..., ge=0)
    rows: list[list[Any]] = Field(..., description='One list of cells per row, in `columns` order, as the run answered\nthem; text is cut to 256 characters.')


class AlertsPage(BaseModel):
    """
    One page of alerts, by name, and how many match in all.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    alerts: list[SummaryResponse]
    limit: int = Field(..., ge=0)
    offset: int = Field(..., ge=0)
    total: int = Field(..., ge=0)


class ChatRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    history: list[Turn] | None = Field(None, description='The turns before this one. The service keeps no session, so the panel\nsends the thread back; without it the model answered a follow-up with\nno idea what came before it.')
    message: str


class DrilldownBody(BaseModel):
    """
    What a caller may ask a page for. An absent body asks for the first page
    of the whole result, in the metric's own order.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    bucket: bool | None = None
    cursor: str | None = None
    limit: int | None = Field(None, ge=0)
    range: str | None = None
    sort: SortBody | None = None


class LeafShape(BaseModel):
    """
    One test on one result column. Which operators apply depends on what
    the column holds: a number takes `>`, `>=`, `<`, `<=`, `==`, `!=`,
    `between`; text takes `==`, `!=`, `contains`, `starts_with`,
    `ends_with`, `in`, `not_in`, comparing exactly; a date takes `before`,
    `after`, `between`, `older_than`, `newer_than`.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    column: str = Field(..., description='The result column, by the `as_name` the metric gives it.')
    op: OperatorShape
    value: Any = Field(..., description='A number, or an integer written as digits; text up to 256\ncharacters; an RFC 3339 timestamp; a list of two for `between`; a\nlist of 1 to 50 texts for `in` and `not_in`; a duration such as\n`P7D`, `P2W`, `PT6H` or `PT30M` for `older_than` and `newer_than`,\nmeasured back from the check.')


class PreviewResponse(BaseModel):
    """
    What a check of the draft would find now. Nothing is recorded.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    outcome: str = Field(..., description='`breach`, `no_breach` or `unknown`.')
    reason: str | None = Field(None, description='Why the check could not tell; null unless the outcome is `unknown`.')
    witness: WitnessResponse | None = None


class RowsShape(RootModel[RowsWord | RowsShape1]):
    root: RowsWord | RowsShape1


class StateResponse(BaseModel):
    """
    What the latest checks left: when, what was found, and whether the
    last valid check saw the condition met.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    breached_since: str | None = None
    last_evaluated_at: str | None = None
    last_outcome: str | None = Field(None, description='`breach`, `no_breach` or `unknown`; absent before the first check.')
    last_reason: str | None = None
    last_valid_breached: bool | None = None
    last_witness: WitnessResponse | None = None


class ConditionShape(BaseModel):
    """
    Which rows must pass, and the test each row takes.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    rows: RowsShape = Field(..., description='`one`: the metric must answer exactly one row, and it must pass.\n`any`, `all`, `none`: of the rows answered. `{"count": {"op": ">",\n"value": 3}}`: the number of passing rows, compared. `any` over no\nrows is not met; `all` and `none` over no rows are unknown.')
    where: NodeShape = Field(..., description='A test on one column, or a group of tests. Groups nest at most 3\ndeep and hold at most 20 tests in all.')


class NodeShape1(BaseModel):
    """
    Met when every test in it is.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    all: list[NodeShape]


class NodeShape2(BaseModel):
    """
    Met when one test in it is.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    any: list[NodeShape]


class NodeShape3(BaseModel):
    """
    Met when the test in it is not.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    not_: NodeShape = Field(..., alias='not')


class NotificationResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    attempts: int = Field(..., ge=0)
    condition: ConditionShape = Field(..., description='The condition the notification was owed under.')
    context_columns: list[str] = Field(..., description="The columns the rule asked the witness to keep beside the condition's.")
    created_at: str
    destination: str
    evaluated_at: str
    id: str
    last_error: str | None = None
    metric: str = Field(..., description='The id of the metric checked.')
    metric_name: str | None = Field(None, description='What the metric was called when the notification was owed; null\nwhere only its id is known.')
    provider_receipt: str | None = None
    rule_revision: int = Field(..., ge=0)
    status: str = Field(..., description='`pending`, `cancelled`, `sent` or `failed`.')
    witness: WitnessResponse = Field(..., description='The rows the check saw that owed it.')


class NotificationsPage(BaseModel):
    """
    One page of a rule's notifications, newest first.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    limit: int = Field(..., ge=0)
    notifications: list[NotificationResponse]
    offset: int = Field(..., ge=0)


class PreviewDraft(BaseModel):
    """
    What the form asks before a rule is saved: what a check of this metric,
    window and condition would find now.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    condition: ConditionShape = Field(..., description='Which rows must pass, and the test each row takes.')
    context_columns: list[str] | None = Field(None, description="Result columns to show beside the condition's, as a rule names them.")
    metric: str = Field(..., description='The id of the stored metric to check.')
    range: str | None = Field(None, description='The window the metric is run over, as a rule names it. Omit it to\nrun the metric unbounded.')


class RuleDraft(BaseModel):
    """
    A rule as an administrator writes it.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    condition: ConditionShape = Field(..., description='Which rows must pass, and the test each row takes.')
    context_columns: list[str] | None = Field(None, description="Result columns, by `as_name`, shown beside the condition's in every\nnotification: up to 5.")
    destination: str = Field(..., description='The configured destination the notification goes to.')
    enabled: bool | None = Field(None, description='Whether checks run. Left out, a new alert is enabled and a replaced\none keeps what it had.')
    expected_revision: int | None = Field(None, description='The revision this write expects to replace. Required to change an\nalert that exists; refused when creating one.', ge=0)
    interval_secs: int = Field(..., description='How often to check, in seconds.', ge=0)
    metric: str = Field(..., description='The id of the stored metric to check.')
    name: str = Field(..., description='What the alert is called, as people read it: up to 200 characters.')
    range: str | None = Field(None, description='The window the metric is run over, as `run_metric` takes it. Omit it\nto run the metric unbounded.')


class RuleResponse(BaseModel):
    """
    A rule as a reader is shown it.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    condition: ConditionShape = Field(..., description='As a draft writes it. A number past ±(2^53 − 1) is a digit string,\nwhich a JavaScript reader would otherwise round; a draft takes it\nback as it is.')
    context_columns: list[str] = Field(..., description="Result columns the witness keeps beside the condition's.")
    created_at: str
    destination: str
    enabled: bool
    id: str = Field(..., description='The handle every other operation takes.')
    interval_secs: int = Field(..., ge=0)
    metric: str
    name: str
    range: str | None = None
    revision: int = Field(..., ge=0)
    state: StateResponse
    updated_at: str


class NodeShape(RootModel[NodeShape1 | NodeShape2 | NodeShape3 | LeafShape]):
    root: NodeShape1 | NodeShape2 | NodeShape3 | LeafShape


ConditionShape.model_rebuild()
NodeShape1.model_rebuild()
NodeShape2.model_rebuild()
NodeShape3.model_rebuild()
