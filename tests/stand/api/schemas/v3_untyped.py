"""insight-v3-core success bodies the service's own document leaves untyped — hand-written.

`docs/components/backend/insight-v3-core/openapi.json` types only the alert
operations; every other JSON success body is declared as a bare
`{"type": "object"}`, so there is nothing to generate these from. They are
written instead from the Rust that serializes each body, under
`src/backend/services/insight-v3-core/src/`: the `serde_json::json!` literals and
response structs in `api/` (`definitions.rs`, `duplicate.rs`, `sharing.rs`,
`dashboard_widgets.rs`, `folders.rs`, `tags.rs`, `pins.rs`, `datasets.rs`,
`tables.rs`, `metric_run.rs`, `metric_drilldown.rs`, `chat.rs`), plus the domain
types they embed — `RunResult` (`domain/query/metric_query/runner.rs`),
`EffectiveClock` (`domain/kinds/metric/answerable.rs`) and `Record`
(`store/dataset_tables.rs`).
"""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime
from typing import Literal
from uuid import UUID

from insight_stand import JsonValue
from pydantic import BaseModel, ConfigDict, Field, RootModel

ClockSource = Literal["metric", "dataset"]
Access = Literal["owner", "editor", "viewer", "ownerless"]
Level = Literal["viewer", "editor"]
Visibility = Literal["private", "people", "everyone"]
DefinitionKind = Literal["metrics", "widgets", "dashboards"]
Layer = Literal["bronze", "silver", "gold", "identity", "other"]
ColumnType = Literal["string", "number", "date"]
Direction = Literal["asc", "desc"]


class Page[T](BaseModel):
    model_config = ConfigDict(extra="forbid")

    items: list[T]
    total: int
    limit: int
    offset: int


class Items[T](BaseModel):
    model_config = ConfigDict(extra="forbid")

    items: list[T]


class CreatedId(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str


class Named(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str


class EffectiveClock(BaseModel):
    model_config = ConfigDict(extra="forbid")

    field: str
    from_: ClockSource = Field(alias="from")


class MetricDefinition(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    body: dict[str, JsonValue]
    clock: EffectiveClock | None = None


class WidgetDefinition(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    body: dict[str, JsonValue]


class HoldingFolder(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: UUID
    name: str


class DashboardDefinition(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    body: dict[str, JsonValue]
    owner_person_id: UUID | None
    owner_name: str | None
    access: Access
    visibility: Visibility
    carries: bool
    folders: list[HoldingFolder]
    tags: list[str]
    updated_at: datetime


class MetricEntry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    updated_at: datetime
    body: dict[str, JsonValue]


class WidgetEntry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    updated_at: datetime
    body: dict[str, JsonValue]
    metric_name: str | None


class DashboardEntry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    owner_person_id: UUID | None
    owner_name: str | None
    access: Access
    updated_at: datetime
    visibility: Visibility
    title: str | None
    tags: list[str]


class Holder(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kind: DefinitionKind
    id: str
    name: str


class Dependents(BaseModel):
    model_config = ConfigDict(extra="forbid")

    holders: list[Holder]
    hidden: int


class Grant(BaseModel):
    model_config = ConfigDict(extra="forbid")

    grantee: UUID | Literal["everyone"]
    level: Level
    granted_by: UUID
    created_at: datetime


class Orphan(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    owner_person_id: UUID
    updated_at: datetime


class DrawnWidget(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: str
    name: str
    body: dict[str, JsonValue]
    clock: EffectiveClock | None


class RunResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    columns: list[str]
    rows: list[list[JsonValue]]
    percents: list[str]
    undated: int | None = None
    clock: EffectiveClock | None = None


class DrilldownSort(BaseModel):
    model_config = ConfigDict(extra="forbid")

    key: str
    direction: Direction


class DrilldownSelection(BaseModel):
    model_config = ConfigDict(extra="forbid")

    metric: str
    range: str | None = None
    bucket: bool | None = None
    sort: DrilldownSort


class DrilldownColumn(BaseModel):
    model_config = ConfigDict(extra="forbid")

    key: str
    label: str
    type: ColumnType
    sortable: bool
    percent: bool
    exact: bool


class DrilldownRow(BaseModel):
    model_config = ConfigDict(extra="forbid")

    values: dict[str, JsonValue]


class Drilldown(BaseModel):
    model_config = ConfigDict(extra="forbid")

    selection: DrilldownSelection
    columns: list[DrilldownColumn]
    rows: list[DrilldownRow]
    next_cursor: str | None


class DatasetEntry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    title: str | None
    fields: int
    updated_at: datetime


class DatasetPage(BaseModel):
    model_config = ConfigDict(extra="forbid")

    names: list[str]
    items: list[DatasetEntry]
    total: int


class Dataset(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    declaration: dict[str, JsonValue]


class DatasetDependents(BaseModel):
    model_config = ConfigDict(extra="forbid")

    metrics: list[str]


class DatasetRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: UUID
    received_at: datetime
    raw_data: JsonValue


class DatasetRecords(BaseModel):
    model_config = ConfigDict(extra="forbid")

    records: list[DatasetRecord]
    total: int
    limit: int


class Folder(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: UUID
    name: str
    owner_person_id: UUID | None
    access: Access


class FolderSummary(Folder):
    dashboards: int


class FolderList(BaseModel):
    model_config = ConfigDict(extra="forbid")

    folders: list[FolderSummary]
    unfiled: int


class FolderEntry(BaseModel):
    model_config = ConfigDict(extra="forbid")

    carries: bool


class Pins(BaseModel):
    model_config = ConfigDict(extra="forbid")

    pins: list[Named]


class Table(BaseModel):
    model_config = ConfigDict(extra="forbid")

    database: str
    table: str
    layer: Layer


class TableList(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tables: list[Table]
    total: int


class TableColumn(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    type: str


class TableDetail(BaseModel):
    model_config = ConfigDict(extra="forbid")

    database: str
    table: str
    layer: Layer
    engine: str
    columns: list[TableColumn]


class TagSummary(BaseModel):
    model_config = ConfigDict(extra="forbid")

    name: str
    dashboards: int


class TagList(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tags: list[TagSummary]


class Written(BaseModel):
    model_config = ConfigDict(extra="forbid")

    metric: Named | None
    widgets: list[Named]
    dashboard: Named | None


class ChatAnswer(BaseModel):
    model_config = ConfigDict(extra="forbid")

    reply: str
    result: RunResult | None = None


class ChatCreated(BaseModel):
    model_config = ConfigDict(extra="forbid")

    reply: str
    created: Written
    updated: Written


class ChatReply(RootModel[ChatAnswer | ChatCreated]):
    pass


__all__: Sequence[str] = (
    "ChatAnswer",
    "ChatCreated",
    "ChatReply",
    "CreatedId",
    "DashboardDefinition",
    "DashboardEntry",
    "Dataset",
    "DatasetDependents",
    "DatasetEntry",
    "DatasetPage",
    "DatasetRecord",
    "DatasetRecords",
    "Dependents",
    "DrawnWidget",
    "Drilldown",
    "DrilldownColumn",
    "DrilldownRow",
    "DrilldownSelection",
    "DrilldownSort",
    "EffectiveClock",
    "Folder",
    "FolderEntry",
    "FolderList",
    "FolderSummary",
    "Grant",
    "Holder",
    "HoldingFolder",
    "Items",
    "MetricDefinition",
    "MetricEntry",
    "Named",
    "Orphan",
    "Page",
    "Pins",
    "RunResult",
    "Table",
    "TableColumn",
    "TableDetail",
    "TableList",
    "TagList",
    "TagSummary",
    "WidgetDefinition",
    "WidgetEntry",
    "Written",
)
