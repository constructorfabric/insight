"""Salesforce REST client, OAuth token provider, and describe-based schema generation.

The ``Salesforce`` class handles auth via OAuth 2.0 Client Credentials flow
(operator supplies ``instance_url``, ``client_id``, ``client_secret``) and
exposes ``describe()`` plus ``generate_schema()`` used by streams to build
SOQL and advertise shapes to the destination. ``SalesforceTokenProvider``
keeps the access token fresh across long Bulk API syncs.
"""

import concurrent.futures
import logging
import threading
import time
from typing import Any, List, Mapping, Optional, Tuple

import requests
from requests import adapters as request_adapters
from requests.exceptions import RequestException

from airbyte_cdk.models import ConfiguredAirbyteCatalog, FailureType, StreamDescriptor
from airbyte_cdk.sources.declarative.auth.token_provider import TokenProvider
from airbyte_cdk.sources.streams.http import HttpClient
from airbyte_cdk.utils import AirbyteTracedException

from source_salesforce.constants import (
    API_VERSION,
    CRM_STREAMS,
    DATE_TYPES,
    LOOSE_TYPES,
    NUMBER_TYPES,
    PARALLEL_TASKS_SIZE,
    QUERY_INCOMPATIBLE_SALESFORCE_OBJECTS,
    QUERY_RESTRICTED_SALESFORCE_OBJECTS,
    STRING_TYPES,
    TOKEN_REFRESH_INTERVAL_SECONDS,
    UNSUPPORTED_STREAMS,
)
from source_salesforce.exceptions import TypeSalesforceException
from source_salesforce.rate_limiting import SalesforceErrorHandler

logger = logging.getLogger("airbyte")


class SalesforceTokenProvider(TokenProvider):
    """Token provider that proactively refreshes the Salesforce access token.

    The default CDK InterpolatedStringTokenProvider captures the token as a
    static string at init time and never refreshes. For long-running Bulk syncs
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
        # Pool sized for parallel describe() + parallel slice fetches. Official
        # connector uses 100; matches our PARALLEL_TASKS_SIZE.
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

        # Cache of full describe() responses per sobject. Populated by
        # generate_schemas(); read by get_custom_field_names() so callers can
        # split records into (standard, custom) without a second describe call.
        self._sobject_describes: dict = {}

    # ------- Auth ------------------------------------------------------------

    def _get_standard_headers(self) -> Mapping[str, str]:
        return {"Authorization": f"Bearer {self.access_token}"}

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
    ) -> Mapping[str, Any]:
        """Describe all sobjects (``sobject`` None) or a specific sobject.

        Raises on 404 for a named sobject rather than returning a bad payload —
        callers depend on ``fields``/``sobjects`` keys being present.
        """
        headers = self._get_standard_headers()
        endpoint = "sobjects" if not sobject else f"sobjects/{sobject}/describe"
        url = f"{self.instance_url}/services/data/{self.version}/{endpoint}"
        resp = self._make_request("GET", url, headers=headers)
        if resp.status_code == 404 and sobject:
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

    def generate_schema(
        self,
        stream_name: Optional[str] = None,
        stream_options: Optional[Mapping[str, Any]] = None,
    ) -> Mapping[str, Any]:
        response = self.describe(stream_name, stream_options)
        if stream_name:
            self._sobject_describes[stream_name] = response
        schema = {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": True,
            "properties": {},
        }
        for field in response["fields"]:
            schema["properties"][field["name"]] = self.field_to_property_schema(field)
        return schema

    def get_custom_field_names(self, sobject: str) -> frozenset:
        """Return the set of field names for which describe reports ``custom=True``.

        Requires that ``generate_schema(sobject)`` (or ``generate_schemas``
        bulk-parallel variant) has been called first; results come from the
        per-sobject describe cache populated by :meth:`generate_schema`.
        """
        desc = self._sobject_describes.get(sobject)
        if not desc:
            # Fallback: fetch on demand. Keeps the call site simple even if
            # generate_schemas wasn't called for this sobject for any reason.
            desc = self.describe(sobject)
            self._sobject_describes[sobject] = desc
        return frozenset(
            f["name"] for f in desc.get("fields", []) if f.get("custom") is True
        )

    def generate_schemas(
        self, stream_objects: Mapping[str, Any]
    ) -> Mapping[str, Any]:
        """Describe-driven schema generation, parallelized via ThreadPoolExecutor.

        Chunks stream names into batches of ``parallel_tasks_size`` so we don't
        open more sockets than our connection pool can hold.
        """

        def load_schema(
            name: str, stream_options: Mapping[str, Any]
        ) -> Tuple[str, Optional[Mapping[str, Any]], Optional[str]]:
            try:
                result = self.generate_schema(
                    stream_name=name, stream_options=stream_options
                )
            except RequestException as e:
                return name, None, str(e)
            return name, result, None

        stream_names = list(stream_objects.keys())
        stream_schemas: dict = {}
        for i in range(0, len(stream_names), self.parallel_tasks_size):
            chunk = stream_names[i : i + self.parallel_tasks_size]
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=min(len(chunk), self.parallel_tasks_size)
            ) as executor:
                for name, schema, err in executor.map(
                    lambda args: load_schema(*args),
                    [(n, stream_objects[n]) for n in chunk],
                ):
                    if err:
                        self.logger.error(f"Loading error for {name} schema: {err}")
                        raise AirbyteTracedException(
                            message=(
                                f"Schema could not be extracted for stream {name}. "
                                "Please retry later."
                            ),
                            internal_message=str(err),
                            failure_type=FailureType.system_error,
                            stream_descriptor=StreamDescriptor(name=name),
                        )
                    stream_schemas[name] = schema
        return stream_schemas

    # ------- Stream discovery -----------------------------------------------

    def get_streams_black_list(self) -> List[str]:
        return (
            QUERY_RESTRICTED_SALESFORCE_OBJECTS
            + QUERY_INCOMPATIBLE_SALESFORCE_OBJECTS
        )

    def filter_streams(self, stream_name: str) -> bool:
        if stream_name.endswith("ChangeEvent") or stream_name in self.get_streams_black_list():
            return False
        return True

    def get_validated_streams(
        self,
        config: Mapping[str, Any],
        catalog: Optional[ConfiguredAirbyteCatalog] = None,
    ) -> Mapping[str, Any]:
        """Return ``{stream_name: sobject_options}`` for streams to sync.

        Selection precedence:
        1. If catalog is provided (incremental sync), honor it intersected with
           queryable sobjects.
        2. Else if config.salesforce_streams is non-empty, use that list.
        3. Else fall back to :data:`CRM_STREAMS` (curated default).

        In every case the full global describe is used to filter out sobjects
        that are not queryable, are ChangeEvents, or are on our blocklists.
        """
        stream_objects: dict = {}
        for so in self.describe()["sobjects"]:
            if so["name"] in UNSUPPORTED_STREAMS:
                self.logger.warning(
                    f"Stream {so['name']} needs an object ID and is skipped."
                )
                continue
            if so["queryable"]:
                stream_objects[so.pop("name")] = so
            else:
                self.logger.warning(f"Stream {so['name']} is not queryable; skipped.")

        if catalog:
            return {
                cs.stream.name: stream_objects[cs.stream.name]
                for cs in catalog.streams
                if cs.stream.name in stream_objects
            }

        requested: List[str] = list(config.get("salesforce_streams") or []) or list(CRM_STREAMS)
        missing = [n for n in requested if n not in stream_objects]
        if missing:
            self.logger.warning(
                "Requested streams not queryable in this org (skipped): %s",
                ", ".join(missing),
            )

        validated = [n for n in requested if n in stream_objects and self.filter_streams(n)]
        return {name: stream_objects[name] for name in validated}

    # ------- Field-type -> JSON-schema mapping -------------------------------

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

    @staticmethod
    def field_to_property_schema(field_params: Mapping[str, Any]) -> Mapping[str, Any]:
        """Map a describe() field entry to a JSON-schema property."""
        sf_type = field_params["type"]

        if sf_type in STRING_TYPES:
            return {"type": ["string", "null"]}
        if sf_type in DATE_TYPES:
            return {
                "type": ["string", "null"],
                "format": "date-time" if sf_type == "datetime" else "date",
            }
        if sf_type in NUMBER_TYPES:
            return {"type": ["number", "null"]}
        if sf_type == "int":
            return {"type": ["integer", "null"]}
        if sf_type == "boolean":
            return {"type": ["boolean", "null"]}
        if sf_type == "base64":
            return {"type": ["string", "null"], "format": "base64"}
        if sf_type == "address":
            return {
                "type": ["object", "null"],
                "properties": {
                    "street": {"type": ["null", "string"]},
                    "state": {"type": ["null", "string"]},
                    "postalCode": {"type": ["null", "string"]},
                    "city": {"type": ["null", "string"]},
                    "country": {"type": ["null", "string"]},
                    "longitude": {"type": ["null", "number"]},
                    "latitude": {"type": ["null", "number"]},
                    "geocodeAccuracy": {"type": ["null", "string"]},
                },
            }
        if sf_type == "location":
            return {
                "type": ["object", "null"],
                "properties": {
                    "longitude": {"type": ["null", "number"]},
                    "latitude": {"type": ["null", "number"]},
                },
            }
        if sf_type in LOOSE_TYPES:
            # >99% of values are strings; normalize to string to avoid
            # destination type conflicts.
            return {"type": ["string", "null"]}
        raise TypeSalesforceException(f"Unsupported Salesforce field type: {sf_type}")
