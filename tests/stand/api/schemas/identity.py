"""Identity-resolution response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/identity-resolution/openapi.json`, emitted by
`cargo run -p identity-resolution -- openapi` from the same route table the
service serves and drift-gated in CI. These models therefore describe the
structs that serialize the wire, which the hand-written ones they replaced could
not: the committed document used to be the retired .NET contract.

`extra="forbid"` throughout: an undeclared field is drift.

The four journal responses (persons-seed, persons-sync, attribute-reconcile,
policy-publish) are separate types with identical fields, because they are
separate operations whose summaries are free to diverge. `schemas/__init__.py`
re-exports one of them as `Operation` for suites that assert the shared shape.
"""

from __future__ import annotations
from uuid import UUID
from typing import Any
from pydantic import AwareDatetime, BaseModel, ConfigDict, Field
from enum import StrEnum


class AttributeReconcileOperationResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    author_person_id: UUID
    completed_at: str | None = None
    error_message: str | None = None
    insight_tenant_id: UUID
    operation_id: UUID
    operation_type: str
    request: dict[str, Any] | None = None
    started_at: str
    status: str
    summary: dict[str, Any] | None = None


class CreatePersonRoleRequest(BaseModel):
    """
    Body of `POST /v1/person-roles` — grant a role to a person.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    person_id: UUID
    reason: str | None = None
    role_id: UUID
    valid_from: AwareDatetime | None = Field(None, description='Optional assignment start; defaults to now when omitted. Accepts RFC-3339\n(`Z`/offset), zone-less, or date-only, normalised to naive-UTC.')


class CreateRoleRequest(BaseModel):
    """
    Body of `POST /v1/roles`.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str


class CreateVisibilityRequest(BaseModel):
    """
    Body of `POST /v1/visibility` — grant a viewer visibility over a target
    (or the whole tree when `viewed_person_id` is omitted).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    reason: str | None = None
    valid_from: AwareDatetime | None = Field(None, description='Optional grant start; defaults to now when omitted. Accepts RFC-3339\n(`Z`/offset), zone-less, or date-only, normalised to naive-UTC.')
    viewed_person_id: UUID | None = None
    viewer_person_id: UUID


class PersonResponse(BaseModel):
    """
    A person node in the org tree (subordinate of a profile), matching the .NET
    `PersonResponse`. Unlike `ProfileResponse`, the attribute fields are plain
    strings (empty when absent, not omitted) and the `supervisor_*`/`parent_*`
    fields serialize as `null` rather than being dropped.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    department: str
    display_name: str
    division: str
    email: str
    first_name: str
    job_title: str
    last_name: str
    parent_email: str | None = None
    parent_id: str | None = None
    parent_person_id: UUID | None = None
    person_id: UUID
    status: str
    subordinates: list[PersonResponse]
    supervisor_email: str | None = None
    supervisor_name: str | None = None


class PersonRoleResponse(BaseModel):
    """
    One role assignment.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    author_person_id: UUID
    created_at: str
    insight_tenant_id: UUID
    person_id: UUID
    person_role_id: UUID
    reason: str | None = None
    role_id: UUID
    valid_from: str
    valid_to: str | None = None


class PersonsSeedOperationResponse(BaseModel):
    """
    One operation's status. Wire shape mirrors the .NET
    `PersonsSeedOperationResponse`: `request` and `summary` are surfaced as
    parsed JSON (not double-encoded strings), the tenant/author ids are
    included, timestamps are ISO-8601, and null fields are emitted (the .NET
    serializer does not drop nulls).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    author_person_id: UUID
    completed_at: str | None = None
    error_message: str | None = None
    insight_tenant_id: UUID
    operation_id: UUID
    operation_type: str
    request: dict[str, Any] | None = None
    started_at: str
    status: str
    summary: dict[str, Any] | None = None


class PersonsSyncOperationResponse(BaseModel):
    """
    One operation's status. Wire shape matches the seed journal's:
    `request` and `summary` surfaced as parsed JSON, ISO-8601 timestamps,
    null fields emitted.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    author_person_id: UUID
    completed_at: str | None = None
    error_message: str | None = None
    insight_tenant_id: UUID
    operation_id: UUID
    operation_type: str
    request: dict[str, Any] | None = None
    started_at: str
    status: str
    summary: dict[str, Any] | None = Field(None, description='On completion: the [`SyncSummary`] — rows copied, `max_id` /\n`max_created_at` watermarks, `synced_at`.\n\n[`SyncSummary`]: crate::domain::sync_service::SyncSummary')


class PolicyPublishOperationResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    author_person_id: UUID
    completed_at: str | None = None
    error_message: str | None = None
    insight_tenant_id: UUID
    operation_id: UUID
    operation_type: str
    request: dict[str, Any] | None = None
    started_at: str
    status: str
    summary: dict[str, Any] | None = None


class PolicyResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    actor_person_id: UUID
    comparison_enabled: bool
    grouping_enabled: bool
    label_override: str | None = None
    reason: str
    retired: bool
    revision: int
    sensitivity_class: str | None = None
    value_mode: str


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


class ProfileIdEntry(BaseModel):
    """
    One source-native account id bound to the person — the latest
    `value_type='id'` observation per source instance. Ported from the .NET
    `ProfileIdEntry`.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    insight_source_id: UUID
    insight_source_type: str
    value: str


class ProfileResponse(BaseModel):
    """
    Response body of `POST /v1/profiles` — the resolved person's profile:
    current attributes, the org tree (`supervisor_*` / `parent_*` /
    `subordinates[]`), and every current source-native id (`ids[]`). Null
    attribute fields are omitted from JSON; `subordinates`/`ids` are always
    present (empty when none), matching the .NET contract.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    department: str | None = None
    display_name: str | None = None
    division: str | None = None
    email: str | None = None
    employee_id: str | None = None
    first_name: str | None = None
    ids: list[ProfileIdEntry] = Field(..., description='Every current source-native id for the person (one per source instance).\nAlways serialized — an empty array when the person has no ids — matching\nthe .NET contract (unlike the attributes above, which are omitted).')
    insight_tenant_id: UUID
    job_title: str | None = None
    last_name: str | None = None
    parent_email: str | None = None
    parent_id: str | None = None
    parent_person_id: UUID | None = None
    person_id: UUID
    status: str | None = None
    subordinates: list[PersonResponse] = Field(..., description='Recursive subordinates subtree (direct reports and their reports), on the\nconfigured `org_chart` source. Always serialized (empty when none).')
    supervisor_email: str | None = None
    supervisor_name: str | None = None
    username: str | None = None


class ResolveProfileRequest(BaseModel):
    """
    Body of `POST /v1/profiles`. `value_type = "email"` matches across all
    sources for the tenant; `value_type = "id"` matches a source-native account
    id within one source instance (needs `insight_source_type` + `insight_source_id`);
    `value_type = "person_id"` takes the canonical person UUID itself — the key
    the metrics runtime and its routes use since the identity cutover.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    insight_source_id: UUID | None = Field(None, description='Required when `value_type = "id"`.')
    insight_source_type: str | None = Field(None, description='Required when `value_type = "id"` — the source instance to scope to.')
    value: str
    value_type: str


class RoleResponse(BaseModel):
    """
    One role in the catalogue.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str
    role_id: UUID


class SubchartNode(BaseModel):
    """
    One node in the org subchart tree.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    display_name: str | None = None
    email: str | None = None
    job_title: str | None = None
    person_id: UUID
    status: str | None = None
    subordinates: list[SubchartNode]


class SubchartResponse(BaseModel):
    """
    `{ "root": { … } }` — single-root wrapper (locked by the #348 acceptance
    criteria so the response can gain sibling fields without breaking clients).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    root: SubchartNode


class ValueModeDto(StrEnum):
    single = 'single'
    multi = 'multi'


class VisibilityResponse(BaseModel):
    """
    One visibility grant.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    author_person_id: UUID
    created_at: str
    insight_tenant_id: UUID
    reason: str | None = None
    valid_from: str
    valid_to: str | None = None
    viewed_person_id: UUID | None = None
    viewer_person_id: UUID
    visibility_id: UUID


class VisiblePersonsRequest(BaseModel):
    """
    Canonical person UUIDs to check (the metric runtime's key since the
    identity cutover — the earlier email-based draft of this endpoint never
    shipped).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    person_ids: list[UUID]


class VisiblePersonsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    visible: list[UUID]


class AttributeReconcileListResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[AttributeReconcileOperationResponse]
    next_cursor: str | None = None


class PersonAttributeResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    first_observed_at: str
    id: UUID
    last_observed_at: str
    policy: PolicyResponse
    source_field_id: str
    source_instance: str
    source_type: str


class PersonRoleListResponse(BaseModel):
    """
    List wrapper.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PersonRoleResponse]
    next_cursor: str | None = Field(None, description='Wire parity with the .NET `ListResponse`: the cursor is declared\nbut pagination is not implemented — always `null` (both\nimplementations return every row; consumers already tolerate it).')


class PersonsSeedListResponse(BaseModel):
    """
    List response wrapper (typed for OpenAPI).
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PersonsSeedOperationResponse]
    next_cursor: str | None = Field(None, description='Wire parity with the .NET `ListResponse`: the cursor is declared\nbut pagination is not implemented — always `null` (both\nimplementations return every row; consumers already tolerate it).')


class PersonsSyncListResponse(BaseModel):
    """
    List response wrapper (typed for OpenAPI). `next_cursor` is declared but
    always `null` — same non-paginating contract as the seed journal.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PersonsSyncOperationResponse]
    next_cursor: str | None = None


class PolicyPublishListResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PolicyPublishOperationResponse]
    next_cursor: str | None = None


class PolicyUpdateRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    comparison_enabled: bool
    expected_revision: int
    grouping_enabled: bool
    label_override: str | None = None
    reason: str
    retired: bool
    sensitivity_class: str | None = None
    value_mode: ValueModeDto


class RoleListResponse(BaseModel):
    """
    List wrapper.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[RoleResponse]
    next_cursor: str | None = Field(None, description='Wire parity with the .NET `ListResponse`: the cursor is declared\nbut pagination is not implemented — always `null` (both\nimplementations return every row; consumers already tolerate it).')


class SubchartForestResponse(BaseModel):
    """
    `{ "roots": [ … ] }` — forest wrapper (#344). Empty when the caller has no
    visible-in-source membership.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    roots: list[SubchartNode]


class VisibilityListResponse(BaseModel):
    """
    List wrapper.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[VisibilityResponse]
    next_cursor: str | None = Field(None, description='Wire parity with the .NET `ListResponse`: the cursor is declared\nbut pagination is not implemented — always `null` (both\nimplementations return every row; consumers already tolerate it).')


class PersonAttributeListResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PersonAttributeResponse]


PersonResponse.model_rebuild()
SubchartNode.model_rebuild()
