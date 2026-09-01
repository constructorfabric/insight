"""Shared floor for the Insight deployed-stand test suite (`tests/stand/`).

This package holds:

* `manifest` — the typed model of the stand's self-description
  (`src/ingestion/tools/seed/manifest.json`), the only source of fixture names, capabilities
  and seeded facts.
* `stand` — where the stand is: base-URL resolution for a host-side or
  in-network runner.
* `session` — `LoginSession`, the one way a request proves who it is.
* `api` — the gateway-fronted HTTP client.
* `wait` — bounded polling for eventually-consistent state.

This package is deliberately test-framework agnostic: no pytest import, no
fixtures, no assertions. `tests/stand/conftest.py` is what turns it into a
suite, and phases 6-8 add person fixtures and browser journeys on top. Nothing
here imports from `src/ingestion/tests/e2e/**` — that rig owns in-process
correctness and feeds four blocking coverage gates; it is read-only reference.
"""

from __future__ import annotations

from collections.abc import Sequence

from .api import (
    ANALYTICS_PREFIX,
    GATEWAY_API_PREFIXES,
    IDENTITY_PREFIX,
    ApiClient,
    ApiResponse,
    JsonValue,
    QueryParams,
    QueryValue,
    StandSession,
    analytics_path,
    identity_path,
)
from .errors import (
    LoginNotCompletedError,
    ManifestError,
    PersonaError,
    StandConnectionError,
    StandError,
)
from .manifest import (
    BOOLEAN_CAPABILITIES,
    MANIFEST_PATH,
    MANIFEST_PATH_ENV,
    SUPPORTED_MANIFEST_VERSION,
    Capabilities,
    Manifest,
    Person,
    Realm,
    Tenants,
    default_manifest_path,
    load_manifest,
)
from .personas import (
    ADMIN_OPERATOR_FIXTURE,
    ADMIN_ROLE,
    LEAD_ROLE,
    MEMBER_ROLE,
    OTHER_TENANT_FIXTURE,
    PASSWORD_ENV,
    REALM_EXPORT_PATH,
    ROLE_TO_REALM_ROLES,
    PersonaSession,
    expected_realm_roles,
    open_session,
    persona_password,
    resolve_by_realm_role,
    verify_realm_roles,
)
from .scratch_identity import (
    RUN_TAG,
    SCRATCH_PREFIX,
    SCRATCH_SOURCE_ID,
    SCRATCH_SOURCE_TYPE,
    issued_names,
    scratch_name,
)
from .service_token import (
    ASSERTION_TYPE,
    IDENTITY_URL_ENV,
    KEY_PATH_ENV,
    SERVICE_NAME,
    TOKEN_URL_ENV,
    ServiceTokenSession,
    default_audience,
    default_identity_url,
    default_token_url,
    open_service_session,
)
from .session import (
    CALLBACK_PATH,
    LOGIN_PATH,
    SESSION_COOKIE_NAME,
    LoginSession,
)
from .stand import (
    ARTIFACT_DIR_ENV,
    BASE_URL_ENV,
    StandEndpoint,
    artifact_dir,
    resolve_base_url,
    resolve_endpoint,
)
from .vectors import distinct_vectors, governs_vector, quality_vectors
from .wait import wait_for, wait_until

__all__: Sequence[str] = (
    "ADMIN_OPERATOR_FIXTURE",
    "ADMIN_ROLE",
    "ANALYTICS_PREFIX",
    "ARTIFACT_DIR_ENV",
    "ASSERTION_TYPE",
    "BASE_URL_ENV",
    "BOOLEAN_CAPABILITIES",
    "CALLBACK_PATH",
    "GATEWAY_API_PREFIXES",
    "IDENTITY_PREFIX",
    "IDENTITY_URL_ENV",
    "KEY_PATH_ENV",
    "LEAD_ROLE",
    "LOGIN_PATH",
    "MANIFEST_PATH",
    "MANIFEST_PATH_ENV",
    "MEMBER_ROLE",
    "OTHER_TENANT_FIXTURE",
    "PASSWORD_ENV",
    "REALM_EXPORT_PATH",
    "ROLE_TO_REALM_ROLES",
    "RUN_TAG",
    "SCRATCH_PREFIX",
    "SCRATCH_SOURCE_ID",
    "SCRATCH_SOURCE_TYPE",
    "SERVICE_NAME",
    "SESSION_COOKIE_NAME",
    "SUPPORTED_MANIFEST_VERSION",
    "TOKEN_URL_ENV",
    "ApiClient",
    "ApiResponse",
    "Capabilities",
    "JsonValue",
    "LoginNotCompletedError",
    "LoginSession",
    "Manifest",
    "ManifestError",
    "Person",
    "PersonaError",
    "PersonaSession",
    "QueryParams",
    "QueryValue",
    "Realm",
    "ServiceTokenSession",
    "StandConnectionError",
    "StandEndpoint",
    "StandError",
    "StandSession",
    "Tenants",
    "analytics_path",
    "artifact_dir",
    "default_audience",
    "default_identity_url",
    "default_manifest_path",
    "default_token_url",
    "distinct_vectors",
    "expected_realm_roles",
    "governs_vector",
    "identity_path",
    "issued_names",
    "load_manifest",
    "open_service_session",
    "open_session",
    "persona_password",
    "quality_vectors",
    "resolve_base_url",
    "resolve_by_realm_role",
    "resolve_endpoint",
    "scratch_name",
    "verify_realm_roles",
    "wait_for",
    "wait_until",
)
