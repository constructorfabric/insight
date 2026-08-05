"""identity-resolution response shapes, hand-written from the Rust DTOs.

Sources, field for field:

    domain/subchart.rs   SubchartNode · SubchartResponse · SubchartForestResponse
    domain/profile.rs    ProfileResponse
    api/roles.rs         RoleResponse
    api/person_roles.rs  PersonRoleResponse
    api/visibility.rs    VisibilityResponse
    api/seed.rs          PersonsSeedOperationResponse
    api/sync.rs          PersonsSyncOperationResponse

**Not generated**, and not from the committed contract — see `schemas/__init__.py`
for why `docs/components/backend/identity-resolution/openapi.json` cannot be
trusted. These models therefore describe OBSERVED behaviour, and `extra` is left
at its default so an added field does not fail the suite. When that contract is
regenerated from the service, this module should be deleted in favour of
generated models with `extra="forbid"`.

Timestamps stay `str`. The service serialises them itself (`api/datetime.rs`
normalises to naive-UTC on the way in), and coercing to `datetime` here would
make the tests assert a parse this suite does not perform — the wire format is
the contract, not Python's reading of it.
"""

from __future__ import annotations

from collections.abc import Sequence
from uuid import UUID

from insight_stand import JsonValue
from pydantic import BaseModel, Field

from .common import ListResponse

# ---------------------------------------------------------------------------
# Org subchart
# ---------------------------------------------------------------------------


class SubchartNode(BaseModel):
    """One person in an org tree. Self-referential through `subordinates`.

    Everything but `person_id` is nullable: a node exists because an `org_chart`
    edge points at it, and the person's attributes are a separate observation
    that may be absent.
    """

    person_id: UUID
    email: str | None = None
    display_name: str | None = None
    job_title: str | None = None
    status: str | None = None
    subordinates: list[SubchartNode] = Field(default_factory=list)

    def walk(self) -> list[SubchartNode]:
        """This node and every descendant, at any depth."""
        found = [self]
        for child in self.subordinates:
            found += child.walk()
        return found

    def emails(self) -> set[str]:
        """Every email in this subtree — the shape scope assertions compare."""
        return {node.email for node in self.walk() if node.email}


class SubchartForest(BaseModel):
    """`GET /v1/subchart` — the forest the CALLER can see.

    Empty when the caller has no visible membership, which is the normal state
    for an account outside the org chart rather than an error.
    """

    roots: list[SubchartNode] = Field(default_factory=list)

    def emails(self) -> set[str]:
        return {email for root in self.roots for email in root.emails()}


class Subchart(BaseModel):
    """`GET /v1/subchart/{person_id}` — one named person's subtree.

    A `root` object rather than a bare node so the response can gain sibling
    fields without breaking clients.
    """

    root: SubchartNode

    def emails(self) -> set[str]:
        return self.root.emails()


# ---------------------------------------------------------------------------
# Profiles
# ---------------------------------------------------------------------------


class Profile(BaseModel):
    """`POST /v1/profiles` — a person resolved by email or source-native id.

    Only the fields the tests assert are declared. The DTO carries many more
    attribute fields, all omitted from JSON when null, and modelling them would
    be describing the seed rather than the contract.
    """

    person_id: UUID
    insight_tenant_id: UUID
    email: str | None = None
    display_name: str | None = None


class IdentityValue(BaseModel):
    """`GET /internal/persons/by-email/{email}` — the login-bootstrap lookup.

    NOT a `Profile`, though both are "a person looked up by email". This route
    answers the identity VALUE that matched — the alias row, pointing at what it
    resolved to — because at login the caller has an email and needs to learn
    which person it belongs to, not to read that person's attributes. Hence
    `insight_source_id` rather than `person_id`, and no tenant at all: the
    tenant is exactly what is still unknown at that point.


    """

    value_type: str
    value: str
    insight_source_type: str
    insight_source_id: UUID


# ---------------------------------------------------------------------------
# Admin: roles, assignments, visibility
# ---------------------------------------------------------------------------


class Role(BaseModel):
    """An entry in the global role catalogue. Deleted, not revoked."""

    role_id: UUID
    name: str


class PersonRole(BaseModel):
    """A role assignment. Temporal: `DELETE` sets `valid_to` rather than removing.

    `valid_to is None` is therefore the only meaning of "in force", and it is
    what the leak sweep in `scratch.py` checks.
    """

    person_role_id: UUID
    insight_tenant_id: UUID
    person_id: UUID
    role_id: UUID
    valid_from: str
    valid_to: str | None = None
    author_person_id: UUID
    reason: str | None = None
    created_at: str

    @property
    def in_force(self) -> bool:
        return self.valid_to is None


class Visibility(BaseModel):
    """A visibility grant. Temporal, exactly like `PersonRole`.

    `viewed_person_id is None` is a grant over everything the viewer's source
    membership covers rather than one named person.
    """

    visibility_id: UUID
    insight_tenant_id: UUID
    viewer_person_id: UUID
    viewed_person_id: UUID | None = None
    valid_from: str
    valid_to: str | None = None
    author_person_id: UUID
    reason: str | None = None
    created_at: str

    @property
    def in_force(self) -> bool:
        return self.valid_to is None


class VisiblePersons(BaseModel):
    """`POST /v1/visible-persons` — the subset of the asked-about person ids.

    A list of what survived, not a per-id verdict: a person the caller may not
    see is absent rather than present-and-false, which is the same
    non-disclosure choice `/v1/subchart/{id}` makes by answering 404.

    Person UUIDs since the identity cutover (#2098), like every other
    person-keyed route.
    """

    visible: list[UUID]


# ---------------------------------------------------------------------------
# Seed / sync journals
# ---------------------------------------------------------------------------


class Operation(BaseModel):
    """One persons-seed or persons-sync run.

    The two DTOs are field-identical, so one model serves both journals. `request`
    and `summary` are free-form objects the service echoes back, kept as
    `JsonValue` rather than modelled — their shape belongs to whichever seed
    version wrote them.
    """

    operation_id: UUID
    operation_type: str
    status: str
    insight_tenant_id: UUID
    author_person_id: UUID
    request: JsonValue = None
    summary: JsonValue = None
    error_message: str | None = None
    started_at: str
    completed_at: str | None = None


# ---------------------------------------------------------------------------
# Listings
# ---------------------------------------------------------------------------

RoleList = ListResponse[Role]
PersonRoleList = ListResponse[PersonRole]
VisibilityList = ListResponse[Visibility]
OperationList = ListResponse[Operation]


__all__: Sequence[str] = (
    "Operation",
    "OperationList",
    "PersonRole",
    "PersonRoleList",
    "Profile",
    "Role",
    "RoleList",
    "Subchart",
    "SubchartForest",
    "SubchartNode",
    "Visibility",
    "VisibilityList",
)
