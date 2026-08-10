"""Pydantic models for the stand's response shapes.

Two halves, and a reader should be able to tell which one they are in:

* `common.py` — the error envelope and the listing wrapper, hand-written from
  the bodies the stand returns.
* `identity_internal.py` — hand-written, and only for the `/internal/*` routes
  the published document deliberately omits (service-to-service only, mounted
  outside the OpenAPI registry).
* `identity.py`, `analytics.py`, `authenticator.py` — GENERATED from documents the services
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
disagree. The hand-written ones are a **description of observed behaviour**, kept
only where no contract describes the route.

The identity models are re-exported below under the names the suites already
use: the generated ones carry the DTO names the service serializes
(`RoleResponse`, `ProfileResponse`, …), and the four journal responses are
field-identical, so one of them is re-exported as `Operation` for assertions
about the shared journal shape.

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
    PersonRoleListResponse as PersonRoleList,
)
from .identity import (
    PersonRoleResponse as PersonRole,
)
from .identity import (
    PersonsSeedListResponse as OperationList,
)
from .identity import (
    PersonsSeedOperationResponse as Operation,
)
from .identity import (
    ProfileResponse as Profile,
)
from .identity import (
    RoleListResponse as RoleList,
)
from .identity import (
    RoleResponse as Role,
)
from .identity import (
    SubchartForestResponse as SubchartForest,
)
from .identity import (
    SubchartNode,
)
from .identity import (
    SubchartResponse as Subchart,
)
from .identity import (
    VisibilityListResponse as VisibilityList,
)
from .identity import (
    VisibilityResponse as Visibility,
)
from .identity import (
    VisiblePersonsResponse as VisiblePersons,
)
from .identity_internal import (
    IdentityValue,
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
