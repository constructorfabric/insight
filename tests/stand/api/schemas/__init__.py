"""Pydantic models for the stand's response shapes.

Two halves, and a reader should be able to tell which one they are in:

* `common.py` — the error envelope and the listing wrapper, hand-written from
  the bodies the stand returns.
* `identity.py` — hand-written from the Rust DTOs in
  `src/backend/services/identity-resolution/src/api/`. **Not** generated,
  because the committed contract for that service is still the .NET document:
  it declares `/v1/persons/{email}` (which identity answers 404 for), declares
  `POST /v1/persons-seed` (405), spells the
  subchart parameter `{personId}` where the service serves `{person_id}`, omits
  both persons-sync operations, and lists only `200` for all 18 operations.
  Generating from it would record every one of those errors as fact.
* `analytics.py`, `authenticator.py` — GENERATED from documents the services
  emit themselves (`cargo run -p <service> -- openapi`) and CI drift-gates in
  `.github/workflows/openapi-specs.yml`, so the models describe the very structs
  that serialize the wire. `authenticator.py` is currently just the error
  envelope, because that document declares every `/auth/*` success body as a bare
  `type: object`.

  **Bodies from the spec; status codes never.** The per-operation status-code
  lists are stamped uniformly by `.standard_errors` and describe nothing (#1669)
  — the identity contract fails the same way by listing only `200` — so every
  status code stays asserted per test, from observed behaviour.

That asymmetry is a real difference in what the models mean. The generated ones
are a **contract test** — a mismatch says the service and its published contract
disagree. The hand-written ones are a **description of observed behaviour**, and
they should be deleted in favour of generated ones once the identity contract is
regenerated from the service.

The strictness follows from that. Generated models set `extra="forbid"`: they are
regenerated in the same change that adds a field, so strictness costs nothing and
an undeclared field is exactly the drift worth catching. Hand-written models
leave `extra` at its default, because there they would only tax every benign
upstream addition.
"""

from __future__ import annotations

from collections.abc import Sequence

from .analytics import (
    MetricDefinitionListResponse,
    MetricResultsResponse,
    RunResponse,
    SavedQuery,
    SavedQueryListResponse,
)
from .analytics import (
    MetricResultViewDto1 as PeriodView,
)
from .common import (
    EXTRACTOR_REJECTION_CONTENT_TYPE,
    PROBLEM_CONTENT_TYPE,
    ListResponse,
    ProblemDocument,
)
from .identity import (
    IdentityValue,
    Operation,
    OperationList,
    PersonRole,
    PersonRoleList,
    Profile,
    Role,
    RoleList,
    Subchart,
    SubchartForest,
    SubchartNode,
    Visibility,
    VisibilityList,
    VisiblePersons,
)

__all__: Sequence[str] = (
    "EXTRACTOR_REJECTION_CONTENT_TYPE",
    "PROBLEM_CONTENT_TYPE",
    "IdentityValue",
    "ListResponse",
    "MetricDefinitionListResponse",
    "MetricResultsResponse",
    "Operation",
    "OperationList",
    "PeriodView",
    "PersonRole",
    "PersonRoleList",
    "ProblemDocument",
    "Profile",
    "Role",
    "RoleList",
    "RunResponse",
    "SavedQuery",
    "SavedQueryListResponse",
    "Subchart",
    "SubchartForest",
    "SubchartNode",
    "Visibility",
    "VisibilityList",
    "VisiblePersons",
)
