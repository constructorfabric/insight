"""Salesforce REST client, OAuth token provider, and describe-based field discovery.

The ``Salesforce`` class handles auth via OAuth 2.0 Client Credentials flow
(operator supplies ``instance_url``, ``client_id``, ``client_secret``) and
exposes ``describe()`` plus ``field_names()`` used by streams to build SOQL.
``SalesforceTokenProvider`` keeps the access token fresh across long syncs.
"""

import logging
import threading
import time
from typing import Any, List, Mapping, Optional, Tuple

import requests
from requests import adapters as request_adapters

from airbyte_cdk.models import FailureType, StreamDescriptor
from airbyte_cdk.sources.declarative.auth.token_provider import TokenProvider
from airbyte_cdk.sources.streams.http import HttpClient
from airbyte_cdk.sources.streams.http.requests_native_auth.abstract_token import (
    AbstractHeaderAuthenticator,
)
from airbyte_cdk.utils import AirbyteTracedException

from source_salesforce.constants import (
    API_VERSION,
    CRM_STREAMS,
    PARALLEL_TASKS_SIZE,
    QUERY_INCOMPATIBLE_SALESFORCE_OBJECTS,
    QUERY_RESTRICTED_SALESFORCE_OBJECTS,
    TOKEN_REFRESH_INTERVAL_SECONDS,
    UNSUPPORTED_STREAMS,
)
from source_salesforce.rate_limiting import SalesforceErrorHandler
from source_salesforce.schema_loader import available_stream_names

logger = logging.getLogger("airbyte")


class SalesforceTokenProvider(TokenProvider):
    """Token provider that proactively refreshes the Salesforce access token.

    The default CDK InterpolatedStringTokenProvider captures the token as a
    static string at init time and never refreshes. For long-running syncs
    that exceed the Salesforce session timeout (default 2 hours), the stale
    token causes INVALID_SESSION_ID. This provider wraps the Salesforce client
    and re-calls ``login()`` every :data:`TOKEN_REFRESH_INTERVAL_SECONDS`.

    Also exposes ``force_refresh()`` so the error handler can refresh on 401
    before CDK retries the failing request.
    """

    def __init__(self, sf_api: "Salesforce") -> None:
        self._sf_api = sf_api
        self._last_refresh_time: float = time.monotonic()
        # Protects concurrent login() calls when multiple stream workers race
        # through the refresh window simultaneously. Cheap lock; held only
        # around the HTTP login request.
        self._lock = threading.Lock()

    def get_token(self) -> str:
        if self._sf_api.access_token is None:
            # Authenticate on demand: discover advertises static schemas and
            # issues no request, so nothing should log in until a caller
            # actually needs a bearer token. A failure here has no previous
            # token to fall back on and must surface.
            with self._lock:
                if self._sf_api.access_token is None:
                    self._sf_api.login()
                    self._last_refresh_time = time.monotonic()

        elapsed = time.monotonic() - self._last_refresh_time
        if elapsed >= TOKEN_REFRESH_INTERVAL_SECONDS:
            with self._lock:
                # Re-check inside the lock — another worker may have just
                # refreshed.
                elapsed = time.monotonic() - self._last_refresh_time
                if elapsed >= TOKEN_REFRESH_INTERVAL_SECONDS:
                    try:
                        logger.info(
                            "Refreshing Salesforce OAuth token (%.0fs since last refresh)",
                            elapsed,
                        )
                        self._sf_api.login()
                        self._last_refresh_time = time.monotonic()
                    except Exception:
                        logger.warning(
                            "Proactive token refresh failed; will use existing token",
                            exc_info=True,
                        )
        return self._sf_api.access_token

    def force_refresh(self) -> None:
        """Force an immediate token refresh after INVALID_SESSION_ID."""
        with self._lock:
            try:
                logger.info(
                    "Forcing Salesforce OAuth token refresh (INVALID_SESSION_ID)"
                )
                self._sf_api.login()
                self._last_refresh_time = time.monotonic()
            except Exception:
                logger.error(
                    "Forced token refresh failed; subsequent requests will likely fail",
                    exc_info=True,
                )


class SalesforceAuthenticator(AbstractHeaderAuthenticator):
    """Per-request bearer auth that re-reads the token from the provider.

    A static ``TokenAuthenticator`` freezes the token string at stream
    construction, so neither the proactive 30-minute refresh nor the 401
    ``force_refresh()`` ever reaches in-flight requests — syncs longer than
    the Salesforce session timeout die in a retry loop. Reading the token
    through :class:`SalesforceTokenProvider` on every request picks up both
    refresh paths.
    """

    def __init__(self, token_provider: SalesforceTokenProvider) -> None:
        self._token_provider = token_provider

    @property
    def auth_header(self) -> str:
        return "Authorization"

    @property
    def token(self) -> str:
        return f"Bearer {self._token_provider.get_token()}"


class Salesforce:
    """Thin Salesforce REST client: login, describe, schema generation.

    Not an Airbyte stream itself — used by the source at construction time to
    discover field shapes and to build the auth token used by every stream.
    """

    logger = logging.getLogger("airbyte")
    version = API_VERSION
    parallel_tasks_size = PARALLEL_TASKS_SIZE

    # SOQL query length cap (query URL + body). Drives property chunking.
    REQUEST_SIZE_LIMITS = 16_384

    def __init__(
        self,
        *,
        instance_url: str,
        client_id: str,
        client_secret: str,
        start_date: Optional[str] = None,
        **_: Any,
    ) -> None:
        if not instance_url:
            raise ValueError("instance_url is required")
        self.instance_url = instance_url.rstrip("/")
        self.client_id = client_id
        self.client_secret = client_secret
        self.access_token: Optional[str] = None
        self.start_date = start_date

        self.session = requests.Session()
        # Pool sized for parallel describe() + parallel slice fetches; matches PARALLEL_TASKS_SIZE.
        adapter = request_adapters.HTTPAdapter(
            pool_connections=self.parallel_tasks_size,
            pool_maxsize=self.parallel_tasks_size,
        )
        self.session.mount("https://", adapter)

        # Shared by describe-time HTTP traffic here and by streams below —
        # keeps a single source of truth for proactive refresh timing.
        self._token_provider = SalesforceTokenProvider(self)
        self._http_client = HttpClient(
            "sf_api",
            self.logger,
            session=self.session,
            error_handler=SalesforceErrorHandler(token_provider=self._token_provider),
        )

        # Cache of full describe() responses per sobject, so building a
        # stream's SOQL field list costs one describe call.
        self._sobject_describes: dict = {}

    # ------- Auth ------------------------------------------------------------

    def _get_standard_headers(self) -> Mapping[str, str]:
        # Through the provider, not the raw attribute: describe is often the
        # first authenticated call of a sync, and nothing has logged in for it.
        return {"Authorization": f"Bearer {self._token_provider.get_token()}"}

    def login(self) -> None:
        """Obtain an access token via OAuth 2.0 Client Credentials flow.

        Hits ``{instance_url}/services/oauth2/token`` — we trust the
        operator-supplied ``instance_url`` over any value echoed in the
        response (some managed identities return an internal domain).
        """
        login_url = f"{self.instance_url}/services/oauth2/token"
        body = {
            "grant_type": "client_credentials",
            "client_id": self.client_id,
            "client_secret": self.client_secret,
        }
        _, resp = self._http_client.send_request(
            "POST",
            login_url,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            data=body,
            request_kwargs={},
        )
        if resp.status_code != 200:
            raise AirbyteTracedException(
                message=(
                    "Salesforce OAuth login failed — check instance_url / "
                    "client_id / client_secret and Run-As user on the External "
                    "Client App."
                ),
                internal_message=f"HTTP {resp.status_code}: {resp.text[:500]}",
                failure_type=FailureType.config_error,
            )
        try:
            auth = resp.json()
        except ValueError as exc:
            raise AirbyteTracedException(
                message="Salesforce OAuth login returned non-JSON response.",
                internal_message=f"body={resp.text[:500]!r}",
                failure_type=FailureType.system_error,
            ) from exc
        token = auth.get("access_token")
        if not token:
            raise AirbyteTracedException(
                message="Salesforce OAuth login response missing access_token.",
                internal_message=f"keys={list(auth.keys())}",
                failure_type=FailureType.system_error,
            )
        self.access_token = token

    def _make_request(
        self,
        http_method: str,
        url: str,
        headers: Optional[dict] = None,
        body: Optional[dict] = None,
    ) -> requests.Response:
        _, resp = self._http_client.send_request(
            http_method, url, headers=headers, data=body, request_kwargs={}
        )
        return resp

    # ------- Describe + schema -----------------------------------------------

    def describe(
        self,
        sobject: Optional[str] = None,
        sobject_options: Optional[Mapping[str, Any]] = None,
        allow_missing: bool = False,
    ) -> Optional[Mapping[str, Any]]:
        """Describe all sobjects (``sobject`` None) or a specific sobject.

        Raises on 404 for a named sobject rather than returning a bad payload —
        callers depend on ``fields``/``sobjects`` keys being present. With
        ``allow_missing`` a 404 returns None instead, for callers that treat an
        absent sobject as a per-org fact rather than a failure.
        """
        headers = self._get_standard_headers()
        endpoint = "sobjects" if not sobject else f"sobjects/{sobject}/describe"
        url = f"{self.instance_url}/services/data/{self.version}/{endpoint}"
        resp = self._make_request("GET", url, headers=headers)
        if resp.status_code == 404 and sobject:
            if allow_missing:
                return None
            raise AirbyteTracedException(
                message=(
                    f"Salesforce sobject '{sobject}' not found. Check the "
                    f"Run-As user's Field-Level Security and Object Access."
                ),
                internal_message=f"options={sobject_options}, body={resp.text[:500]}",
                failure_type=FailureType.config_error,
                stream_descriptor=StreamDescriptor(name=sobject),
            )
        if resp.status_code != 200:
            raise AirbyteTracedException(
                message=f"Salesforce describe('{sobject or 'global'}') failed",
                internal_message=f"HTTP {resp.status_code}: {resp.text[:500]}",
                failure_type=FailureType.system_error,
            )
        return resp.json()

    def sobject_describe(self, sobject: str) -> Optional[Mapping[str, Any]]:
        """Cached describe for one sobject; None when the org does not expose it.

        A 404 means the sobject is absent from this org or the Run-As user has
        no object access. Both are per-org facts the connector reports and
        works around, not errors that should fail a sync.
        """
        if sobject not in self._sobject_describes:
            self._sobject_describes[sobject] = self.describe(sobject, allow_missing=True)
        return self._sobject_describes[sobject]

    def is_queryable(self, sobject: str) -> bool:
        """Whether this org exposes ``sobject`` to SOQL."""
        desc = self.sobject_describe(sobject)
        return bool(desc) and bool(desc.get("queryable", True))

    def field_names(self, sobject: str) -> Tuple[str, ...]:
        """Every field the org exposes on ``sobject``, standard and custom.

        SOQL selects the full set so custom and undeclared standard values still
        reach the record envelope, which preserves them in ``raw_data``. Empty
        when the org does not expose the sobject.
        """
        desc = self.sobject_describe(sobject)
        if desc is None:
            return ()
        return tuple(f["name"] for f in desc.get("fields", []) if f.get("name"))

    # ------- Stream discovery -----------------------------------------------

    def get_streams_black_list(self) -> List[str]:
        return (
            QUERY_RESTRICTED_SALESFORCE_OBJECTS
            + QUERY_INCOMPATIBLE_SALESFORCE_OBJECTS
        )

    def filter_streams(self, stream_name: str) -> bool:
        if stream_name.endswith("ChangeEvent") or stream_name in self.get_streams_black_list():
            return False
        if stream_name not in available_stream_names():
            self.logger.warning(
                "Stream %s has no static schema and is skipped.", stream_name
            )
            return False
        return True

    def syncable_streams(self) -> List[str]:
        """The curated stream set, minus anything this connector cannot sync.

        Org-independent by construction: :data:`CRM_STREAMS` is a code-level
        contract and the remaining filters read only local state, so the
        advertised catalog costs no API call. Whether a given org exposes a
        sobject is settled per stream at read time, off the describe the stream
        already makes to build its SOQL field list.
        """
        return [
            name
            for name in CRM_STREAMS
            if name not in UNSUPPORTED_STREAMS and self.filter_streams(name)
        ]

    def unavailable_streams(self) -> List[str]:
        """Curated streams this org does not expose to SOQL, from one describe.

        Reported by ``check`` so an operator sees the gaps while configuring the
        connection instead of discovering them in sync logs.
        """
        global_describe = self.describe() or {}
        queryable = {
            so["name"] for so in global_describe.get("sobjects", []) if so.get("queryable")
        }
        return [name for name in self.syncable_streams() if name not in queryable]

    @staticmethod
    def get_pk_and_replication_key(
        json_schema: Mapping[str, Any],
    ) -> Tuple[Optional[str], Optional[str]]:
        """Return (primary_key, cursor_field) for an sobject schema.

        Cursor priority: SystemModstamp > LastModifiedDate > CreatedDate > LoginTime.
        A stream with none of these becomes full-refresh.
        """
        fields = json_schema.get("properties", {}).keys()
        pk = "Id" if "Id" in fields else None
        for cand in ("SystemModstamp", "LastModifiedDate", "CreatedDate", "LoginTime"):
            if cand in fields:
                return pk, cand
        return pk, None

