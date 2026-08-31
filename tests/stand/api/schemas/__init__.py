"""Pydantic models for the stand's response shapes.

Two halves, and a reader should be able to tell which one they are in:

* `common.py` — the error envelope and the listing wrapper, hand-written from
  the bodies the stand returns.
* `identity_internal.py` — hand-written from the Rust DTO, because the two
  `/internal/persons/*` S2S routes are registered raw and stay out of the
  generated document by design.
* `analytics.py`, `authenticator.py`, `identity.py` — GENERATED from documents
  the services emit themselves (`cargo run -p <service> -- openapi`) and CI
  drift-gates in `.github/workflows/openapi-specs.yml`, so the models describe
  the very structs that serialize the wire. `authenticator.py` is currently just
  the error envelope, because that document declares every `/auth/*` success
  body as a bare `type: object`.

  The generated identity models carry the contract's names — `SubchartResponse`,
  `ProfileResponse` — where the hand-written module said `Subchart`, `Profile`.
  This package re-exports them under the names the suite already uses, so the
  rename stops here.

  **Bodies from the spec; status codes never.** The per-operation status-code
  lists are stamped uniformly by `.standard_errors` and describe nothing (#1669)
  — the identity contract fails the same way by listing only `200` — so every
  status code stays asserted per test, from observed behaviour.

That asymmetry is a real difference in what the models mean. The generated ones
are a **contract test** — a mismatch says the service and its published contract
disagree. The hand-written ones are a **description of observed behaviour**, kept
only where no document describes the route at all.

The strictness follows from that. Generated models set `extra="forbid"`: they are
regenerated in the same change that adds a field, so strictness costs nothing and
an undeclared field is exactly the drift worth catching. Hand-written models
leave `extra` at its default, because there they would only tax every benign
upstream addition.
"""

from __future__ import annotations

from collections.abc import Sequence

from .analytics import (
    ConnectorHealthResponse,
    CustomMetric,
    CustomMetricInput,
    CustomMetricListResponse,
    CustomMetricSummary,
    ExportCustomMetricsResponse,
    FeedbackListResponse,
    ImportCustomMetricsRequest,
    ImportCustomMetricsResponse,
    MetricDefinitionListResponse,
    MetricResultsResponse,
    RunResponse,
    SavedQuery,
    SavedQueryListResponse,
    SyncHistoryResponse,
    UsageConfigResponse,
    UsageSummaryResponse,
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
    AccountBindingResponse,
    AccountSearchResponse,
    AttentionResponse,
    CorrectionResponse,
    MeResponse,
    PersonAccountsResponse,
    PersonListResponse,
    SubchartNode,
    VisibilityPolicy,
)
from .identity import (
    PersonRoleListResponse as PersonRoleList,
)
from .identity import (
    PersonRoleResponse as PersonRole,
)
from .identity import (
    PersonsSeedListResponse as SeedOperationList,
)
from .identity import (
    PersonsSeedOperationResponse as Operation,
)
from .identity import (
    PersonsSyncListResponse as SyncOperationList,
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
    SubchartResponse as Subchart,
)
from .identity import (
    VisibilityListResponse as VisibilityList,
)
from .identity import (
    VisibilityResponse as Visibility,
)
from .identity import (
    VisiblePersonsPageResponse as VisiblePersonsPage,
)
from .identity import (
    VisiblePersonsResponse as VisiblePersons,
)
from .identity_internal import IdentityValue

__all__: Sequence[str] = (
    "EXTRACTOR_REJECTION_CONTENT_TYPE",
    "PROBLEM_CONTENT_TYPE",
    "AccountBindingResponse",
    "AccountSearchResponse",
    "AttentionResponse",
    "ConnectorHealthResponse",
    "CorrectionResponse",
    "CustomMetric",
    "CustomMetricInput",
    "CustomMetricListResponse",
    "CustomMetricSummary",
    "ExportCustomMetricsResponse",
    "FeedbackListResponse",
    "IdentityValue",
    "ImportCustomMetricsRequest",
    "ImportCustomMetricsResponse",
    "ListResponse",
    "MeResponse",
    "MetricDefinitionListResponse",
    "MetricResultsResponse",
    "Operation",
    "PeriodView",
    "PersonAccountsResponse",
    "PersonListResponse",
    "PersonRole",
    "PersonRoleList",
    "ProblemDocument",
    "Profile",
    "Role",
    "RoleList",
    "RunResponse",
    "SavedQuery",
    "SavedQueryListResponse",
    "SeedOperationList",
    "Subchart",
    "SubchartForest",
    "SubchartNode",
    "SyncHistoryResponse",
    "SyncOperationList",
    "UsageConfigResponse",
    "UsageSummaryResponse",
    "Visibility",
    "VisibilityList",
    "VisibilityPolicy",
    "VisiblePersons",
    "VisiblePersonsPage",
)
