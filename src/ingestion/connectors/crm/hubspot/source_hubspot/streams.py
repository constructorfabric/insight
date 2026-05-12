"""HubSpot stream classes.

Four stream shapes — live and archived are split so each lands in its own
Bronze table and runs on its own state. All streams sync sequentially via the
classic ``AbstractSource`` path; HubSpot's search endpoint is rate-limited to
4 rps portal-wide so concurrency adds 429 retries rather than throughput.

- ``CrmSearchStream`` — incremental via ``/crm/v3/objects/{type}/search``.
  One search window per sync ``[state, init_sync]`` filtered on
  ``hs_lastmodifieddate``, sorted by ``hs_object_id ASC``, paged via ``after``;
  when ``after >= 10_000`` (HubSpot's search-result hard cap) the loop
  restarts the same window with ``hs_object_id > last_seen_id`` keyset.
- ``CrmArchivedListStream`` — two-pass list + batch_read against
  ``/crm/v3/objects/{type}?archived=true`` (no server-side ``archivedAt``
  filter exists). Pass 1 collects ids of records with ``archivedAt > state``;
  Pass 2 fetches full property values via
  ``POST /crm/v3/objects/{type}/batch/read?archived=true``.
- ``OwnersStream`` — ``/crm/v3/owners/`` page-cursor listing for live owners;
  no property discovery, no search endpoint, no associations. Incremental on
  ``updatedAt`` via client-side filter.
- ``OwnersArchivedStream`` — same endpoint with ``archived=true``;
  incremental on ``archivedAt`` via client-side filter.

All streams apply the Insight envelope via :func:`envelope.envelope` before
yielding to Bronze, and each tracks its own cursor state via the CDK
``Stream.state`` property.
"""

from __future__ import annotations

import copy
import logging
from abc import ABC, abstractmethod
from typing import Any, Iterable, List, Mapping, MutableMapping, Optional, Tuple

import pendulum
import requests

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.core import Stream, StreamData
from airbyte_cdk.sources.streams.http import HttpClient

from source_hubspot.api import Hubspot, _TimeoutSession
from source_hubspot.associations import AssociationFetcher
from source_hubspot.constants import (
    BASE_URL,
    BATCH_READ_LIMIT,
    LIST_PAGE_LIMIT,
    SEARCH_AFTER_HARD_CAP,
    SEARCH_PAGE_LIMIT,
    STREAM_REGISTRY,
)
from source_hubspot.envelope import envelope, inject_envelope_properties
from source_hubspot.rate_limiting import HubspotErrorHandler


logger = logging.getLogger("airbyte")


class HubspotStream(Stream, ABC):
    """Base class: envelope injection, shared HttpClient wiring, schema, state.

    Concrete subclasses implement :meth:`_generate_records` to yield raw
    HubSpot dicts; :meth:`read_records` handles envelope + association
    enrichment + cursor advancement.

    State follows the classic CDK pattern: each stream tracks ``self._state``
    (a pendulum.DateTime) and exposes it as ``{cursor_field: ISO-8601 string}``
    via the :attr:`state` property; the CDK persists it as part of the
    connector state messages.
    """

    def __init__(
        self,
        *,
        stream_name: str,
        hubspot_api: Hubspot,
        access_token: str,
        tenant_id: str,
        source_id: str,
        start_date: pendulum.DateTime,
    ) -> None:
        self._stream_name = stream_name
        self._hubspot = hubspot_api
        self._registry = STREAM_REGISTRY[stream_name]
        self._object_type = self._registry["object_type"]
        self._tenant_id = tenant_id
        self._source_id = source_id
        self._start_date = start_date
        self._envelope_collisions_seen: set = set()
        # Cursor state — populated by the state setter from prior sync output;
        # advanced by read_records as records flow through.
        self._state: Optional[pendulum.DateTime] = None
        # Stamped at construction so search streams have a stable upper bound
        # (records that arrive after ``_init_sync`` are picked up next run).
        self._init_sync = pendulum.now("UTC")

        # Every stream gets its own HttpClient so the error handler can
        # attribute failures to the right stream name. _TimeoutSession injects
        # connect/read timeouts; a small pool (main + association + one retry)
        # is sufficient for sequential CDK execution.
        session = _TimeoutSession()
        session.headers.update({"Authorization": f"Bearer {access_token}"})
        adapter = requests.adapters.HTTPAdapter(
            pool_connections=2, pool_maxsize=8
        )
        session.mount("https://", adapter)
        self._http_client = HttpClient(
            name=f"hubspot_{stream_name}",
            logger=logger,
            session=session,
            error_handler=HubspotErrorHandler(stream_name),
        )

        # Association fetcher is wired only when the registry declares any.
        assoc_targets = list(self._registry.get("associations") or [])
        self._associations: Optional[AssociationFetcher] = (
            AssociationFetcher(
                from_object_type=self._object_type,
                to_object_types=assoc_targets,
                http_client=self._http_client,
            )
            if assoc_targets
            else None
        )

    # ------- Stream identity ------------------------------------------------

    @property
    def name(self) -> str:
        return self._stream_name

    @property
    def primary_key(self) -> Optional[str]:
        return "id"

    @property
    def cursor_field(self) -> Optional[str]:
        return self._registry["cursor_field"]

    # ------- State management ----------------------------------------------

    @property
    def state(self) -> Mapping[str, Any]:
        if self._state is None or self.cursor_field is None:
            return {}
        return {self.cursor_field: self._state.to_iso8601_string()}

    @state.setter
    def state(self, value: Mapping[str, Any]) -> None:
        if not value or self.cursor_field is None:
            return
        raw = value.get(self.cursor_field)
        if not raw:
            return
        try:
            parsed = pendulum.parse(str(raw))
        except Exception:
            return
        if isinstance(parsed, pendulum.DateTime):
            self._state = parsed
        elif isinstance(parsed, pendulum.Date):
            self._state = pendulum.datetime(
                parsed.year, parsed.month, parsed.day, tz="UTC"
            )

    def _advance_state(self, latest: Optional[pendulum.DateTime]) -> None:
        if latest is None:
            return
        if self._state is None or latest > self._state:
            self._state = latest

    def _record_cursor(
        self, record: Mapping[str, Any]
    ) -> Optional[pendulum.DateTime]:
        """Extract the cursor value from a record (or None if absent)."""
        if self.cursor_field is None:
            return None
        raw = record.get(self.cursor_field)
        if raw is None:
            return None
        if isinstance(raw, pendulum.DateTime):
            return raw
        try:
            parsed = pendulum.parse(str(raw))
        except Exception:
            return None
        if isinstance(parsed, pendulum.DateTime):
            return parsed
        return None

    # ------- Schema ---------------------------------------------------------

    def get_json_schema(self) -> Mapping[str, Any]:
        """Advertise per-stream schema to the destination.

        - Start from describe-generated schema (every hubspotDefined property).
        - Add the envelope fields so ClickHouse creates columns for them.
        - Add ``associations_{to_object_type}`` arrays when applicable.
        - ``custom_fields`` JSON blob is added by inject_envelope_properties.
        """
        # Deep copy so envelope and association-props loop don't mutate the
        # describe cache shared across streams.
        schema = copy.deepcopy(self._hubspot.generate_schema(self._object_type))
        schema = inject_envelope_properties(schema)
        props = schema.setdefault("properties", {})
        for to_type in self._registry.get("associations") or []:
            props[f"associations_{to_type}"] = {
                "type": ["array", "null"],
                "items": {"type": "string"},
            }
        return schema

    # ------- Read pipeline --------------------------------------------------

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[StreamData]:
        """Fetch records, batch-enrich associations, envelope, and yield."""
        # CDK passes incoming state via stream_state on the first call —
        # mirror it onto the instance so subclass logic can read self._state.
        if stream_state and self.cursor_field:
            self.state = stream_state  # type: ignore[assignment]

        custom_names = self._hubspot.custom_property_names(self._object_type)

        latest_cursor: Optional[pendulum.DateTime] = None
        batch: List[MutableMapping[str, Any]] = []
        for record in self._generate_records(sync_mode, stream_slice, stream_state):
            cursor_value = self._record_cursor(record)
            if cursor_value is not None and (
                latest_cursor is None or cursor_value > latest_cursor
            ):
                latest_cursor = cursor_value
            batch.append(dict(record))
            if len(batch) >= SEARCH_PAGE_LIMIT:
                yield from self._finalize_batch(batch, custom_names)
                batch = []
        if batch:
            yield from self._finalize_batch(batch, custom_names)

        self._advance_state(latest_cursor)

    def _finalize_batch(
        self,
        batch: List[MutableMapping[str, Any]],
        custom_names: frozenset,
    ) -> Iterable[MutableMapping[str, Any]]:
        if self._associations is not None:
            self._associations.enrich(batch)
        for record in batch:
            yield envelope(
                record,
                tenant_id=self._tenant_id,
                source_id=self._source_id,
                custom_property_names=custom_names,
                collision_seen=self._envelope_collisions_seen,
            )

    # ------- Subclass contract ----------------------------------------------

    @abstractmethod
    def _generate_records(
        self,
        sync_mode: SyncMode,
        stream_slice: Optional[Mapping[str, Any]],
        stream_state: Optional[Mapping[str, Any]],
    ) -> Iterable[Mapping[str, Any]]:
        """Yield raw (pre-envelope) HubSpot records."""
        ...


# =============================================================================
# CRM Search stream
# =============================================================================


class CrmSearchStream(HubspotStream):
    """Incremental stream via ``/crm/v3/objects/{type}/search``.

    Single window per sync — ``hs_lastmodifieddate`` between
    ``state`` (or the configured start_date on the first sync) and
    ``init_sync`` (the moment the source was constructed). Records are sorted
    by ``hs_object_id ASC`` and paged via ``after``; when HubSpot's hard cap
    of ``after = 10000`` is hit the loop restarts the same window filtered on
    ``hs_object_id > last_seen_id`` keyset, repeating until the page count
    drops below the cap.

    Archived records aren't returned by search — they live in the sibling
    :class:`CrmArchivedListStream`.
    """

    @property
    def _search_cursor_property(self) -> str:
        return self._registry["search_cursor_property"]

    def _search_url(self) -> str:
        return f"{BASE_URL}/crm/v3/objects/{self._object_type}/search"

    def _generate_records(
        self,
        sync_mode: SyncMode,
        stream_slice: Optional[Mapping[str, Any]],
        stream_state: Optional[Mapping[str, Any]],
    ) -> Iterable[Mapping[str, Any]]:
        lower = (self._state or self._start_date).to_iso8601_string()
        upper = self._init_sync.to_iso8601_string()
        property_names = list(self._hubspot.property_names(self._object_type))
        min_object_id: Optional[str] = None
        after: Optional[str] = None

        while True:
            payload = self._search_body(
                lower=lower,
                upper=upper,
                property_names=property_names,
                after=after,
                min_object_id=min_object_id,
            )
            results, next_after = self._post_search(payload)
            if not results:
                return
            for rec in results:
                yield rec

            if next_after is None:
                return
            if _after_exceeds_cap(next_after):
                # 10k cap: restart same window with keyset on hs_object_id.
                last = results[-1]
                last_id = last.get("id")
                if not last_id:
                    logger.warning(
                        "Stream '%s' hit %s records in window [%s..%s] but the "
                        "last record has no id; stopping to avoid an invalid "
                        "keyset filter.",
                        self._stream_name,
                        SEARCH_AFTER_HARD_CAP,
                        lower,
                        upper,
                    )
                    return
                min_object_id = str(last_id)
                logger.info(
                    "Stream '%s' hit %s records in window [%s..%s]; "
                    "restarting from hs_object_id>%s",
                    self._stream_name,
                    SEARCH_AFTER_HARD_CAP,
                    lower,
                    upper,
                    min_object_id,
                )
                after = None
                continue
            after = next_after

    def _post_search(
        self, body: Mapping[str, Any]
    ) -> tuple[List[Mapping[str, Any]], Optional[str]]:
        _, resp = self._http_client.send_request(
            "POST",
            self._search_url(),
            headers={"Content-Type": "application/json"},
            json=body,
            request_kwargs={},
        )
        data = resp.json()
        results = list(data.get("results") or [])
        next_after = None
        paging = data.get("paging") or {}
        if isinstance(paging, Mapping):
            nxt = paging.get("next") or {}
            if isinstance(nxt, Mapping):
                next_after = nxt.get("after")
        return results, next_after

    def _search_body(
        self,
        *,
        lower: str,
        upper: str,
        property_names: List[str],
        after: Optional[str],
        min_object_id: Optional[str],
    ) -> Mapping[str, Any]:
        cursor_prop = self._search_cursor_property
        filters: List[Mapping[str, Any]] = [
            {"propertyName": cursor_prop, "operator": "GTE", "value": lower},
            {"propertyName": cursor_prop, "operator": "LTE", "value": upper},
        ]
        if min_object_id is not None:
            # Keyset restart — same time window, but start from the last-seen id.
            filters.append(
                {"propertyName": "hs_object_id", "operator": "GT", "value": min_object_id}
            )
        body: MutableMapping[str, Any] = {
            "filterGroups": [{"filters": filters}],
            "properties": property_names,
            # Always sort by hs_object_id ASC so the cursor advances
            # monotonically regardless of how many records share the same
            # ``hs_lastmodifieddate`` — and so the keyset restart on the
            # 10k cap can resume cleanly via ``hs_object_id > last_id``.
            "sorts": [{"propertyName": "hs_object_id", "direction": "ASCENDING"}],
            "limit": SEARCH_PAGE_LIMIT,
        }
        if after is not None:
            body["after"] = after
        return body


# =============================================================================
# Owners stream
# =============================================================================


class OwnersStream(HubspotStream):
    """List ``/crm/v3/owners/``, incremental on ``updatedAt``.

    Owners schema is stable across portals (no custom properties) so the
    stream advertises a small hard-coded schema rather than calling
    ``properties/v2``. Owners has no search endpoint and no server-side
    ``updatedAt`` filter, so the stream pages the full owner list every sync
    and filters records client-side; only changed owners are emitted to the
    destination after the first sync.
    """

    def get_json_schema(self) -> Mapping[str, Any]:
        schema = {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": True,
            "properties": {
                "id": {"type": ["string", "null"]},
                "email": {"type": ["string", "null"]},
                "firstName": {"type": ["string", "null"]},
                "lastName": {"type": ["string", "null"]},
                "userId": {"type": ["integer", "null"]},
                "createdAt": {"type": ["string", "null"], "format": "date-time"},
                "updatedAt": {"type": ["string", "null"], "format": "date-time"},
                "archived": {"type": ["boolean", "null"]},
                # Populated only on the archived owners stream; lets dbt
                # Silver dedup rank archive events above prior updates.
                "archivedAt": {"type": ["string", "null"], "format": "date-time"},
            },
        }
        return inject_envelope_properties(schema)

    def _generate_records(
        self,
        sync_mode: SyncMode,
        stream_slice: Optional[Mapping[str, Any]],
        stream_state: Optional[Mapping[str, Any]],
    ) -> Iterable[Mapping[str, Any]]:
        threshold = self._state  # None on first sync → emit all
        for record in self._paginate_owners(archived=False):
            if _record_cursor_passes(record, "updatedAt", threshold):
                yield record

    def _paginate_owners(self, *, archived: bool) -> Iterable[Mapping[str, Any]]:
        url = f"{BASE_URL}/crm/v3/owners/"
        after: Optional[str] = None
        while True:
            params: MutableMapping[str, Any] = {
                "limit": LIST_PAGE_LIMIT,
                "archived": "true" if archived else "false",
            }
            if after:
                params["after"] = after
            _, resp = self._http_client.send_request(
                "GET", url, headers={}, params=params, request_kwargs={}
            )
            data = resp.json()
            for rec in data.get("results") or []:
                yield rec
            paging = data.get("paging") or {}
            nxt = (paging.get("next") or {}) if isinstance(paging, Mapping) else {}
            after = nxt.get("after") if isinstance(nxt, Mapping) else None
            if not after:
                return

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[StreamData]:
        """Envelope owners without touching the CRM properties endpoint.

        Owners have no ``/crm/v3/properties/owners`` endpoint and no
        custom-field surface, so the base :class:`HubspotStream.read_records`
        path (which calls ``self._hubspot.custom_property_names`` and batches
        through :func:`_finalize_batch`) doesn't apply. Stream directly from
        :meth:`_generate_records`, envelope with an empty custom-field set,
        and skip association enrichment (owners have none). State advance is
        applied at the end via the same cursor-tracking pattern.
        """
        if stream_state and self.cursor_field:
            self.state = stream_state  # type: ignore[assignment]

        latest_cursor: Optional[pendulum.DateTime] = None
        for record in self._generate_records(sync_mode, stream_slice, stream_state):
            cursor_value = self._record_cursor(record)
            if cursor_value is not None and (
                latest_cursor is None or cursor_value > latest_cursor
            ):
                latest_cursor = cursor_value
            yield envelope(
                record,
                tenant_id=self._tenant_id,
                source_id=self._source_id,
                custom_property_names=frozenset(),
                collision_seen=self._envelope_collisions_seen,
            )
        self._advance_state(latest_cursor)


class OwnersArchivedStream(OwnersStream):
    """Archived owners — ``/crm/v3/owners/?archived=true``, incremental on ``archivedAt``.

    Inherits the hard-coded schema and the no-properties ``read_records``
    override from :class:`OwnersStream`. The list endpoint has no
    ``archivedAt`` filter, so the stream pages the full archived owner set
    every sync but drops records whose ``archivedAt`` precedes the prior
    cursor state.
    """

    def _generate_records(
        self,
        sync_mode: SyncMode,
        stream_slice: Optional[Mapping[str, Any]],
        stream_state: Optional[Mapping[str, Any]],
    ) -> Iterable[Mapping[str, Any]]:
        threshold = self._state if self._state is not None else self._start_date
        # First sync uses start_date as floor — be inclusive so a record
        # archived exactly at start_date isn't dropped at the boundary.
        inclusive = self._state is None
        for record in self._paginate_owners(archived=True):
            if _record_cursor_passes(record, "archivedAt", threshold, inclusive=inclusive):
                yield record


# =============================================================================
# CRM archived list stream
# =============================================================================


class CrmArchivedListStream(HubspotStream):
    """Archived CRM objects via two-pass list + batch_read.

    Search doesn't return archived records, and the list endpoint has no
    ``archivedAt`` query filter, so the stream pages the whole archived set
    every sync but drops records whose ``archivedAt`` precedes the prior
    cursor state. State advances to ``max(archivedAt seen)``.

    Two-pass property fetch:

    1. **List** — ``GET /crm/v3/objects/{type}?archived=true&limit=100`` with
       NO ``properties=`` query param. Returns each archive's ``id``,
       ``createdAt``, ``updatedAt``, ``archivedAt``, ``archived``. The
       ``archivedAt > threshold`` filter is applied here so we don't pay the
       Pass 2 cost for records we'd just discard. Surviving ids buffered.
    2. **batch_read** — ``POST /crm/v3/objects/{type}/batch/read?archived=true``
       with the full property list (standard + custom) in the JSON body, up
       to 100 ids per call. The body has no URL-length cap, so HTTP 414
       (which would trip a GET listing every property) is sidestepped.
       Returns full records; we overlay the Pass 1 ``archivedAt`` to keep
       filter and state consistent (Pass 1 is the authoritative archive
       timestamp).

    Some object types (e.g. meetings = 0-47) return HTTP 400 on Pass 1
    ("Paging through deleted objects is not yet supported"); the registry
    flag ``archived_supported=False`` excludes them up-front, but the runtime
    swallow remains as a safety net for future regressions.
    """

    def _list_url(self) -> str:
        return f"{BASE_URL}/crm/v3/objects/{self._object_type}"

    def _batch_read_url(self) -> str:
        return f"{BASE_URL}/crm/v3/objects/{self._object_type}/batch/read"

    def _generate_records(
        self,
        sync_mode: SyncMode,
        stream_slice: Optional[Mapping[str, Any]],
        stream_state: Optional[Mapping[str, Any]],
    ) -> Iterable[Mapping[str, Any]]:
        threshold = self._state if self._state is not None else self._start_date
        inclusive = self._state is None
        property_names = list(self._hubspot.property_names(self._object_type))

        # Buffer at most BATCH_READ_LIMIT stubs before flushing Pass 2 so we
        # never accumulate the full archived set in memory (large orgs can have
        # hundreds of thousands of archived records).
        pending: List[Tuple[str, Any]] = []  # (id, archivedAt)
        for stub in self._paginate_archived_ids():
            if not _record_cursor_passes(stub, "archivedAt", threshold, inclusive=inclusive):
                continue
            stub_id = stub.get("id")
            if not stub_id:
                continue
            pending.append((str(stub_id), stub.get("archivedAt")))
            if len(pending) >= BATCH_READ_LIMIT:
                yield from self._batch_read_chunk(pending, property_names)
                pending = []

        if pending:
            yield from self._batch_read_chunk(pending, property_names)

    def _paginate_archived_ids(self) -> Iterable[Mapping[str, Any]]:
        """Page the archived list endpoint with no ``properties=`` param."""
        url = self._list_url()
        after: Optional[str] = None
        while True:
            params: MutableMapping[str, Any] = {
                "limit": LIST_PAGE_LIMIT,
                "archived": "true",
            }
            if after:
                params["after"] = after
            _, resp = self._http_client.send_request(
                "GET", url, headers={}, params=params, request_kwargs={}
            )
            data = resp.json()
            for rec in data.get("results") or []:
                yield rec
            paging = data.get("paging") or {}
            nxt = (paging.get("next") or {}) if isinstance(paging, Mapping) else {}
            after = nxt.get("after") if isinstance(nxt, Mapping) else None
            if not after:
                return

    def _batch_read_chunk(
        self,
        id_ts_pairs: List[Tuple[str, Any]],
        property_names: List[str],
    ) -> Iterable[Mapping[str, Any]]:
        """Fetch full properties for a chunk and overlay the Pass-1 archivedAt."""
        archived_ts = {id_: ts for id_, ts in id_ts_pairs}
        for record in self._batch_read(list(archived_ts.keys()), property_names):
            rec_id = record.get("id")
            if rec_id is not None:
                # Pass 1 timestamp is authoritative — covers the (rare) case
                # where batch_read omits archivedAt for a row that the list
                # endpoint returned it for.
                record["archivedAt"] = archived_ts.get(
                    str(rec_id), record.get("archivedAt")
                )
            yield record

    def _batch_read(
        self, ids: List[str], property_names: List[str]
    ) -> Iterable[Mapping[str, Any]]:
        body: MutableMapping[str, Any] = {
            "properties": property_names,
            "inputs": [{"id": _id} for _id in ids],
        }
        # ``archived=true`` instructs batch_read to look up archived records;
        # without it the endpoint only resolves live ids and returns empties
        # for everything we just listed.
        url = f"{self._batch_read_url()}?archived=true"
        _, resp = self._http_client.send_request(
            "POST",
            url,
            headers={"Content-Type": "application/json"},
            json=body,
            request_kwargs={},
        )
        data = resp.json()
        for rec in data.get("results") or []:
            yield rec


# =============================================================================
# Helpers
# =============================================================================


def _after_exceeds_cap(after: Any) -> bool:
    """True if the ``after`` cursor crosses HubSpot's 10k search-result cap.

    ``after`` is an opaque string from HubSpot but in practice it's numeric
    for search results (row offset). Compare as int; fall back to False on
    non-numeric (shouldn't happen, but guards against API surprise).
    """
    try:
        return int(str(after)) >= SEARCH_AFTER_HARD_CAP
    except (TypeError, ValueError):
        return False


def _parse_datetime(value: Any) -> Optional[pendulum.DateTime]:
    if value is None:
        return None
    if isinstance(value, pendulum.DateTime):
        return value
    try:
        parsed = pendulum.parse(str(value))
    except Exception:
        return None
    if isinstance(parsed, pendulum.DateTime):
        return parsed
    return None


def _record_cursor_passes(
    record: Mapping[str, Any],
    cursor_field: str,
    threshold: Optional[pendulum.DateTime],
    *,
    inclusive: bool = False,
) -> bool:
    """True if record's cursor passes ``threshold`` (or threshold absent).

    ``inclusive=True`` uses ``>=`` instead of ``>`` — used on the first
    archived sync so a record archived exactly at ``start_date`` isn't
    dropped at the boundary.
    """
    value = _parse_datetime(record.get(cursor_field))
    if value is None:
        logger.warning(
            "Record id=%r has no parseable %r value; dropping from sync",
            record.get("id"),
            cursor_field,
        )
        return False
    if threshold is None:
        return True
    return value >= threshold if inclusive else value > threshold
