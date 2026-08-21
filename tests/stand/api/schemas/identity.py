"""Identity Resolution response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/identity-resolution/openapi.json`, generated
offline by `cargo run -p identity-resolution -- openapi` and drift-gated in CI
beside the analytics and authenticator documents. Until that subcommand existed
this module was hand-written from the Rust DTOs, because the committed contract
was still the retired .NET one; these models now describe the structs that
serialize the wire, so a validation failure is a contract disagreement rather
than a stale transcription.

The names are the contract's, not the suite's: `SubchartResponse` where the
hand-written module said `Subchart`. `stand/api/schemas/__init__.py` re-exports
them under the names the tests already use, so the rename stops at this package.

BODIES ONLY — no status code comes from this document. Its per-operation lists
are stamped uniformly by `.standard_errors` and describe nothing (#1669), the
same limitation the analytics and authenticator documents carry.
"""

from __future__ import annotations
from pydantic import AwareDatetime, BaseModel, ConfigDict, Field
from uuid import UUID
from typing import Any
from enum import StrEnum


class AccountRef(BaseModel):
    """
    A source-native account, as named by the caller.

    Addressing by an observed value (e-mail / username) instead of the account
    triple is the reserved extension for importing a prepared matching table:
    the fields arrive optional, exactly one form is required per item, a value
    resolving to zero or several active accounts is reported per item and never
    guessed. The response already carries per-item outcomes, so adding it does
    not change the shape of this contract.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    id: str = Field(..., description='Account id within that instance.')
    source: str = Field(..., description='Connector type, e.g. `github`.')
    source_id: UUID = Field(..., description='Connector instance id.')


class AccountRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    account: AccountRef
    comment: str | None = None


class BindItem(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    account: AccountRef
    person_id: UUID


class BindRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    bindings: list[BindItem] = Field(..., description='One or more bindings; a prepared matching table is submitted as one call.')
    comment: str | None = None


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


class ItemResult(BaseModel):
    """
    What happened to one requested account.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    account_id: str
    outcome: str = Field(..., description='`applied` — the binding is in force;\n`already_decided` — the same operator decision was already recorded;\n`refused` — the write could not place the row (a concurrent operation\nheld the key); the account keeps its previous binding.\nOpen vocabulary: value-addressed items will report their skip reasons\n(`ambiguous_value`, `unknown_value`) here.')
    source: str
    source_id: UUID


class MeRoleResponse(BaseModel):
    """
    One active role assignment of the caller.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str
    role_id: UUID


class MergeRequest(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    comment: str | None = None
    source_person_id: UUID = Field(..., description='The person being absorbed — its accounts move to the target.')
    target_person_id: UUID = Field(..., description='The surviving person, named explicitly by the operator.')


class PersonAccountEntry(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    account_id: str
    bound_by_operator: bool = Field(..., description="`true` when the account's current binding was made by a person.")
    email: str | None = None
    source: str
    source_id: UUID
    username: str | None = None


class PersonAccountsResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    accounts: list[PersonAccountEntry]
    person_id: UUID


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


class PersonSummaryResponse(BaseModel):
    """
    A person as operator surfaces display them: enough to recognise and pick,
    nothing more. Every field but the id may be null — a person the journal
    knows only through bindings still appears, as the id alone.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    display_name: str | None = None
    email: str | None = None
    job_title: str | None = None
    person_id: UUID
    provisional: bool | None = Field(None, description='The journal holds nothing but an automatic mint for this person — a\nsign-in that needed somebody to enter as, or a roster listing an account\nwith no address. They may duplicate one the roster knows, so they are not\na merge target: the history is on the other side.')
    status: str | None = None
    username: str | None = Field(None, description='Source-native handle (e.g. a git login) — often the only recognisable\nfield of an identity no HR system has observed yet.')


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


class QueueItemResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    account_id: str
    bound_to: UUID | None = Field(None, description='Who holds the account right now. Absent = nobody, which is what an\nunbound account on the queue means; present, it names which of the\ncandidates below is the one being disagreed with.')
    candidates: list[PersonSummaryResponse] = Field(..., description='Persons this account could belong to, if any are known — hydrated into\ncards so the operator UI never has to resolve bare ids itself.')
    department: str | None = None
    display_name: str | None = Field(None, description='How the source describes the account. Nothing here is matchable — it is\nwhat lets an operator recognise whose account this is when automation\ncannot, which is exactly the case for the ones only they can bind.')
    email: str | None = None
    job_title: str | None = None
    kind: str = Field(..., description='`contested` | `binding_conflict` | `provisioned_at_login` |\n`minted_from_roster` | `no_source_id` | `no_evidence`.')
    manager_email: str | None = None
    source: str
    source_id: UUID
    status: str | None = None
    username: str | None = None


class ResolutionRatesResponse(BaseModel):
    """
    How the tenant's observed accounts are split across the resolution states.

    Deliberately no person total. A journal-wide count answers "how many ids have
    we ever written", which after a merge never falls and after a detach only
    rises — so it read as a roster size while measuring something else. A figure
    that would have to be explained every time it is read is worse than none.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    bound: int = Field(..., ge=0)
    excluded: int = Field(..., ge=0)
    no_evidence: int = Field(..., ge=0)
    no_source_id: int = Field(..., ge=0)
    observed: int = Field(..., ge=0)
    pending: int = Field(..., ge=0)


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


class RevokeReasonRequest(BaseModel):
    """
    Optional `DELETE` body carrying a revoke reason.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    reason: str | None = None


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


class VisibilityPolicy(StrEnum):
    org_chart = 'org_chart'
    flat = 'flat'


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


class VisiblePersonsPageResponse(BaseModel):
    """
    One page of the persons the caller may see.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PersonSummaryResponse]
    next_cursor: str | None = Field(None, description='Pass back as `?cursor=` for the next page; absent on the last one.')


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


class AccountMatchResponse(BaseModel):
    """
    One account as a search answers for it: what it is, and whose it is.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    account_id: str
    bound_by_operator: bool = Field(..., description='`true` when a person decided this binding rather than automation.')
    display_name: str | None = None
    email: str | None = None
    excluded: bool = Field(..., description='The account is deliberately excluded from person metrics (a bot, CI, a\nservice account). Without this an exclusion — an operator\'s recorded\ndecision — would read as "bound to nobody" and invite undoing it.')
    person: PersonSummaryResponse | None = None
    source: str
    source_id: UUID
    username: str | None = None


class AccountOperationResponse(BaseModel):
    """
    One operator call that named this account.

    The binding journal says what a decision did; this says who ran it, the
    reason they gave, and how far it reached — a merge lands one row here and
    one in every other account it moved.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    accounts_touched: int = Field(..., description='Accounts the call named, this one included.', ge=0)
    author: PersonSummaryResponse | None = None
    author_person_id: UUID
    comment: str | None = Field(None, description='What the operator typed, when they typed anything.')
    operation_id: UUID
    outcome: str | None = Field(None, description='What the call did to THIS account: `applied` | `already_decided` |\n`refused`. A refusal changed nothing and still belongs in the trail.')
    recorded_at: str
    verb: str = Field(..., description='`operator-bind` | `operator-merge` | `operator-detach` | `operator-exclude`.')


class AccountSearchResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[AccountMatchResponse]
    next_cursor: str | None = Field(None, description='Pass back as `?cursor=` for the next page; absent on the last one. Only\nvalid for the query that issued it — narrowing `q` starts over.')


class AttentionResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[QueueItemResponse]
    items_truncated: bool = Field(..., description='`limit` cut the item list — more accounts await a decision than are\nlisted here. Distinct from `truncated`: the rates stay whole-tenant,\nonly this page is short.')
    rates: ResolutionRatesResponse
    truncated: bool = Field(..., description='The evidence read hit its safety cap: the queue and the rates describe\nonly the first accounts of the tenant, not all of them. Consumers must\nnot present these numbers as tenant-wide. (The binding read cannot be a\nprefix — a partial one would misclassify, so it fails the request.)')


class CorrectionResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    already_decided: int = Field(..., ge=0)
    applied: int = Field(..., ge=0)
    items: list[ItemResult]
    new_person_id: UUID | None = Field(None, description='Set by `detach` when the account reached the new person; absent when\nthe write was refused, since no binding points at that id.')


class HistoryEntry(BaseModel):
    """
    One decision in an account's history.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    author: PersonSummaryResponse | None = None
    author_person_id: UUID
    by_operator: bool = Field(..., description='`true` when a person made this decision, `false` for automation.')
    person: PersonSummaryResponse | None = None
    person_id: UUID
    reason: str | None = None
    recorded_at: str


class MeResponse(BaseModel):
    """
    The caller as the gateway JWT identifies them, with their active roles.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    insight_tenant_id: UUID
    person_id: UUID
    roles: list[MeRoleResponse]
    visibility_policy: VisibilityPolicy


class PersonListResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    items: list[PersonSummaryResponse]
    next_cursor: str | None = Field(None, description='Pass back as `?cursor=` for the next page; absent on the last one. Only\nvalid for the query that issued it — narrowing the terms starts over.')


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


class AccountBindingResponse(BaseModel):
    model_config = ConfigDict(
        extra='forbid',
    )
    account_id: str
    history: list[HistoryEntry]
    operations: list[AccountOperationResponse] = Field(..., description='Operator calls that named this account, newest first. Absent from the\nbinding journal by design: one call can move many accounts, and only\nthe call knows why it was made.')
    person_id: UUID | None = Field(None, description='The binding in force now, if the account has one.')
    source: str
    source_id: UUID


PersonResponse.model_rebuild()
SubchartNode.model_rebuild()
