"""Stream classes: REST API (``/queryAll``), incremental via ``ConcurrentCursor``.

Each stream emits records through :func:`envelope.envelope` so Bronze rows
carry ``tenant_id`` / ``source_id`` / ``unique_key`` / ``custom_fields`` in
addition to the raw SF fields.
"""

import logging
import urllib.parse
from abc import ABC
from typing import (
    Any,
    Callable,
    Iterable,
    List,
    Mapping,
    MutableMapping,
    Optional,
    Tuple,
    Union,
)

import pendulum
import requests  # type: ignore[import]
from pendulum.parsing.exceptions import ParserError
from requests import exceptions

from airbyte_cdk import MessageRepository, StreamSlice
from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.concurrent.cursor import ConcurrentCursor
from airbyte_cdk.sources.streams.concurrent.state_converters.datetime_stream_state_converter import (
    IsoMillisConcurrentStreamStateConverter,
)
from airbyte_cdk.sources.streams.core import CheckpointMixin, StreamData
from airbyte_cdk.sources.streams.http import HttpClient, HttpStream, HttpSubStream
from airbyte_cdk.sources.utils.transform import TransformConfig, TypeTransformer

from source_salesforce.api import Salesforce
from source_salesforce.constants import PARENT_SALESFORCE_OBJECTS, UNSUPPORTED_FILTERING_STREAMS
from source_salesforce.envelope import envelope, inject_envelope_properties
from source_salesforce.rate_limiting import (
    SalesforceErrorHandler,
    default_backoff_handler,
)


logger = logging.getLogger("airbyte")

DEFAULT_LOOKBACK_SECONDS = 600  # based on https://trailhead.salesforce.com/trailblazer-community/feed/0D54V00007T48TASAZ


class SalesforceStream(HttpStream, ABC):
    state_converter = IsoMillisConcurrentStreamStateConverter(is_sequential_state=False)
    transformer = TypeTransformer(TransformConfig.DefaultSchemaNormalization)

    def __init__(
        self,
        sf_api: Salesforce,
        pk: str,
        stream_name: str,
        message_repository: MessageRepository,
        sobject_options: Mapping[str, Any] = None,
        schema: dict = None,
        start_date=None,
        tenant_id: str = "",
        source_id: str = "",
        custom_field_names: Optional[frozenset] = None,
        **kwargs,
    ):
        self.stream_name = stream_name
        self.pk = pk
        self.sf_api = sf_api
        super().__init__(**kwargs)
        self.schema: Mapping[str, Any] = schema  # type: ignore[assignment]
        self.sobject_options = sobject_options
        self.start_date = self.format_start_date(start_date)
        self._message_repository = message_repository
        # Insight envelope context — used in read_records() to inject tenant /
        # source / unique_key / custom_fields onto every emitted record.
        self._tenant_id = tenant_id
        self._source_id = source_id
        self._custom_field_names: frozenset = custom_field_names or frozenset()
        # Tracks envelope-key collisions so we only warn once per offender
        # per stream instead of every record.
        self._envelope_collisions_seen: set = set()
        self._http_client = HttpClient(
            self.stream_name,
            self.logger,
            session=self._http_client._session,  # no need to specific api_budget and authenticator as HttpStream sets them in self._session
            error_handler=SalesforceErrorHandler(
                stream_name=self.stream_name,
                sobject_options=self.sobject_options,
                token_provider=self.sf_api._token_provider,
            ),
        )

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[StreamData]:
        """Bypass RFR (which uses `_read_single_page`) and inject envelope.

        Every record yielded by the upstream reader is passed through
        :func:`envelope.envelope` so Bronze gets tenant_id / source_id /
        unique_key / data_source / collected_at / custom_fields.
        """
        for record in super().read_records(sync_mode, cursor_field, stream_slice, stream_state):
            if isinstance(record, Mapping):
                yield envelope(
                    record,
                    tenant_id=self._tenant_id,
                    source_id=self._source_id,
                    custom_field_names=self._custom_field_names,
                    collision_seen=self._envelope_collisions_seen,
                )
            else:
                # State / log / trace messages pass through untouched.
                yield record

    def _sf_properties(self) -> Mapping[str, Any]:
        """All describe-reported fields (standard + custom). Used for SOQL.

        SOQL needs every field present on the sobject so custom values can
        reach :func:`envelope.envelope`, which routes them into the
        ``custom_fields`` blob.
        """
        if not self.schema:
            self.schema = self.sf_api.generate_schema(self.name)
        return self.schema.get("properties", {})

    def get_json_schema(self) -> Mapping[str, Any]:
        """Advertise schema to the destination.

        - Start from describe-generated properties.
        - Strip ``__c`` custom fields — their values are routed into the
          ``custom_fields`` JSON blob by :func:`envelope.envelope`, so top-level
          columns would always be NULL and create per-org schema drift in
          Bronze. This is the main reason our Bronze stays stable across orgs.
        - Add the Insight envelope fields (``tenant_id`` / ``source_id`` /
          ``unique_key`` / ``data_source`` / ``collected_at`` / ``custom_fields``).
        """
        if not self.schema:
            self.schema = self.sf_api.generate_schema(self.name)
        schema = {
            "$schema": self.schema.get("$schema", "http://json-schema.org/draft-07/schema#"),
            "type": self.schema.get("type", "object"),
            "additionalProperties": self.schema.get("additionalProperties", True),
            "properties": {
                k: v
                for k, v in self.schema.get("properties", {}).items()
                if k not in self._custom_field_names
            },
        }
        inject_envelope_properties(schema)
        return schema

    @staticmethod
    def format_start_date(start_date: Optional[str]) -> Optional[str]:
        """Transform the format `2021-07-25` into the format `2021-07-25T00:00:00Z`"""
        if start_date:
            return pendulum.parse(start_date).strftime("%Y-%m-%dT%H:%M:%SZ")  # type: ignore[attr-defined,no-any-return]
        return None

    @property
    def max_properties_length(self) -> int:
        return Salesforce.REQUEST_SIZE_LIMITS - len(self.url_base) - 2000

    @property
    def name(self) -> str:
        return self.stream_name

    @property
    def primary_key(self) -> Optional[Union[str, List[str], List[List[str]]]]:
        # Destination dedup must key on the tenant-scoped unique_key, not the
        # bare SF Id. Two customer orgs can produce the same Record ID (e.g.
        # ``001xxx`` for Account), and a plain-Id PK would let a later tenant's
        # row overwrite an earlier tenant's in a shared Bronze table.
        return "unique_key"

    @property
    def url_base(self) -> str:
        return self.sf_api.instance_url

    @property
    def too_many_properties(self):
        # Size check uses the full SF field list (what actually goes into SOQL).
        selected_properties = self._sf_properties()
        properties_length = len(urllib.parse.quote(",".join(p for p in selected_properties)))
        return properties_length > self.max_properties_length

    def parse_response(self, response: requests.Response, **kwargs) -> Iterable[Mapping]:
        yield from response.json()["records"]

    # get_json_schema() is overridden above to inject envelope fields.

    def get_error_display_message(self, exception: BaseException) -> Optional[str]:
        if isinstance(exception, exceptions.ConnectionError):
            return f"After {self.max_retries} retries the connector has failed with a network error. It looks like Salesforce API experienced temporary instability, please try again later."
        return super().get_error_display_message(exception)

class PropertyChunk:
    """
    Object that is used to keep track of the current state of a chunk of properties for the stream of records being synced.
    """

    properties: Mapping[str, Any]
    first_time: bool
    record_counter: int
    next_page: Optional[Mapping[str, Any]]

    def __init__(self, properties: Mapping[str, Any]):
        self.properties = properties
        self.first_time = True
        self.record_counter = 0
        self.next_page = None


class RestSalesforceStream(SalesforceStream):
    state_converter = IsoMillisConcurrentStreamStateConverter(is_sequential_state=False)

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        # Property chunking needs a natural key (self.pk, i.e. SF Id) to
        # reassemble split records before envelope runs. Raise rather than
        # assert so the failure survives `python -O` and gives operators a
        # clear message instead of a bare AssertionError.
        if self.too_many_properties and not self.pk:
            raise RuntimeError(
                f"Stream '{self.name}' has too many properties for REST "
                "property chunking but no primary key; records cannot be "
                "reassembled. The sobject must expose a primary key (Id)."
            )

    def path(self, next_page_token: Mapping[str, Any] = None, **kwargs: Any) -> str:
        if next_page_token:
            """
            If `next_page_token` is set, subsequent requests use `nextRecordsUrl`.
            """
            next_token: str = next_page_token["next_token"]
            return next_token
        return f"/services/data/{self.sf_api.version}/queryAll"

    def next_page_token(self, response: requests.Response) -> Optional[Mapping[str, Any]]:
        response_data = response.json()
        next_token = response_data.get("nextRecordsUrl")
        return {"next_token": next_token} if next_token else None

    def request_params(
        self,
        stream_state: Mapping[str, Any],
        stream_slice: Mapping[str, Any] = None,
        next_page_token: Mapping[str, Any] = None,
        property_chunk: Mapping[str, Any] = None,
    ) -> MutableMapping[str, Any]:
        """
        Salesforce SOQL Query: https://developer.salesforce.com/docs/atlas.en-us.232.0.api_rest.meta/api_rest/dome_queryall.htm
        """
        if next_page_token:
            # If `next_page_token` is set, subsequent requests use `nextRecordsUrl`, and do not include any parameters.
            return {}

        property_chunk = property_chunk or {}
        query = f"SELECT {','.join(property_chunk.keys())} FROM {self.name} "

        if self.name in PARENT_SALESFORCE_OBJECTS:
            # add where clause: " WHERE ContentDocumentId IN ('06905000000NMXXXXX', ...)"
            parent_field = PARENT_SALESFORCE_OBJECTS[self.name]["field"]
            parent_ids = [f"'{parent_record[parent_field]}'" for parent_record in stream_slice["parents"]]
            query += f" WHERE ContentDocumentId IN ({','.join(parent_ids)})"

        if self.pk and self.name not in UNSUPPORTED_FILTERING_STREAMS:
            # ORDER BY the SF natural key (Id), not the Insight unique_key —
            # SOQL only knows SF fields. Leading space matters: when a WHERE
            # clause was appended above it ends without trailing whitespace.
            query += f" ORDER BY {self.pk} ASC"

        return {"q": query}

    def chunk_properties(self) -> Iterable[Mapping[str, Any]]:
        # Use the full describe-derived field list (standard + custom). Custom
        # fields are NOT in the destination schema but we still need them in
        # SOQL so envelope() can route them into the ``custom_fields`` blob.
        selected_properties = dict(self._sf_properties())

        def empty_props_with_pk_if_present():
            # Chunk reassembly keys by SF Id (self.pk), not the Insight
            # unique_key which doesn't exist on the SF response.
            return {self.pk: selected_properties[self.pk]} if self.pk else {}

        summary_length = 0
        local_properties = empty_props_with_pk_if_present()
        for property_name, value in selected_properties.items():
            current_property_length = len(urllib.parse.quote(f"{property_name},"))
            if current_property_length + summary_length >= self.max_properties_length:
                yield local_properties
                local_properties = empty_props_with_pk_if_present()
                summary_length = 0

            local_properties[property_name] = value
            summary_length += current_property_length

        if local_properties:
            yield local_properties

    @staticmethod
    def _next_chunk_id(property_chunks: Mapping[int, PropertyChunk]) -> Optional[int]:
        """
        Figure out which chunk is going to be read next.
        It should be the one with the least number of records read by the moment.
        """
        non_exhausted_chunks = {
            # We skip chunks that have already attempted a sync before and do not have a next page
            chunk_id: property_chunk.record_counter
            for chunk_id, property_chunk in property_chunks.items()
            if property_chunk.first_time or property_chunk.next_page
        }
        if not non_exhausted_chunks:
            return None
        return min(non_exhausted_chunks, key=non_exhausted_chunks.get)

    def _read_pages(
        self,
        records_generator_fn: Callable[
            [
                requests.PreparedRequest,
                requests.Response,
                Mapping[str, Any],
                Mapping[str, Any],
            ],
            Iterable[StreamData],
        ],
        stream_slice: Mapping[str, Any] = None,
        stream_state: Mapping[str, Any] = None,
    ) -> Iterable[StreamData]:
        stream_state = stream_state or {}
        records_by_primary_key = {}
        property_chunks: Mapping[int, PropertyChunk] = {
            index: PropertyChunk(properties=properties) for index, properties in enumerate(self.chunk_properties())
        }
        while True:
            chunk_id = self._next_chunk_id(property_chunks)
            if chunk_id is None:
                # pagination complete
                break

            property_chunk = property_chunks[chunk_id]
            request, response = self._fetch_next_page_for_chunk(
                stream_slice,
                stream_state,
                property_chunk.next_page,
                property_chunk.properties,
            )

            # When this is the first time we're getting a chunk's records, we set this to False to be used when deciding the next chunk
            if property_chunk.first_time:
                property_chunk.first_time = False
            property_chunk.next_page = self.next_page_token(response)
            chunk_page_records = records_generator_fn(request, response, stream_state, stream_slice)
            if not self.too_many_properties:
                # this is the case when a stream has no primary key
                # (it is allowed when properties length does not exceed the maximum value)
                # so there would be a single chunk, therefore we may and should yield records immediately
                for record in chunk_page_records:
                    property_chunk.record_counter += 1
                    yield record
                continue

            # Stitch split records by SF natural key. Envelope hasn't fired
            # yet at this point, so records have ``Id`` (self.pk) but no
            # ``unique_key``.
            for record in chunk_page_records:
                property_chunk.record_counter += 1
                record_id = record[self.pk]
                if record_id not in records_by_primary_key:
                    records_by_primary_key[record_id] = (record, 1)
                    continue
                partial_record, counter = records_by_primary_key[record_id]
                partial_record.update(record)
                counter += 1
                if counter == len(property_chunks):
                    yield partial_record  # now it's complete
                    records_by_primary_key.pop(record_id)
                else:
                    records_by_primary_key[record_id] = (partial_record, counter)

        # Process what's left.
        # Because we make multiple calls to query N records (each call to fetch X properties of all the N records),
        # there's a chance that the number of records corresponding to the query may change between the calls.
        # Select 'a', 'b' from table order by pk -> returns records with ids `1`, `2`
        #   <insert smth.>
        # Select 'c', 'd' from table order by pk -> returns records with ids `1`, `3`
        # Then records `2` and `3` would be incomplete.
        # This may result in data inconsistency. We skip such records for now and log a warning message.
        incomplete_record_ids = ",".join([str(key) for key in records_by_primary_key])
        if incomplete_record_ids:
            self.logger.warning(f"Inconsistent record(s) with primary keys {incomplete_record_ids} found. Skipping them.")

        # Always return an empty generator just in case no records were ever yielded
        yield from []

    @default_backoff_handler(max_tries=5)  # FIXME remove once HttpStream relies on the HttpClient
    def _fetch_next_page_for_chunk(
        self,
        stream_slice: Mapping[str, Any] = None,
        stream_state: Mapping[str, Any] = None,
        next_page_token: Mapping[str, Any] = None,
        property_chunk: Mapping[str, Any] = None,
    ) -> Tuple[requests.PreparedRequest, requests.Response]:
        request_headers = self.request_headers(
            stream_state=stream_state,
            stream_slice=stream_slice,
            next_page_token=next_page_token,
        )
        return self._http_client.send_request(
            http_method=self.http_method,
            url=self._join_url(
                self.url_base,
                self.path(
                    stream_state=stream_state,
                    stream_slice=stream_slice,
                    next_page_token=next_page_token,
                ),
            ),
            headers=dict(request_headers),
            params=self.request_params(
                stream_state=stream_state,
                stream_slice=stream_slice,
                next_page_token=next_page_token,
                property_chunk=property_chunk,
            ),
            json=self.request_body_json(
                stream_state=stream_state,
                stream_slice=stream_slice,
                next_page_token=next_page_token,
            ),
            data=self.request_body_data(
                stream_state=stream_state,
                stream_slice=stream_slice,
                next_page_token=next_page_token,
            ),
            request_kwargs={},
        )


class BatchedSubStream(HttpSubStream):
    state_converter = IsoMillisConcurrentStreamStateConverter(is_sequential_state=False)
    SLICE_BATCH_SIZE = 200

    def stream_slices(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        """Instead of yielding one parent record at a time, make stream slice contain a batch of parent records.

        It allows to get <SLICE_BATCH_SIZE> records by one requests (instead of only one).
        """
        batched_slice = []
        for stream_slice in super().stream_slices(sync_mode, cursor_field, stream_state):
            if len(batched_slice) == self.SLICE_BATCH_SIZE:
                yield {"parents": batched_slice}
                batched_slice = []
            batched_slice.append(stream_slice["parent"])
        if batched_slice:
            yield {"parents": batched_slice}


class RestSalesforceSubStream(BatchedSubStream, RestSalesforceStream):
    pass


class IncrementalRestSalesforceStream(RestSalesforceStream, CheckpointMixin, ABC):
    def __init__(self, replication_key: str, stream_slice_step: str = "P30D", **kwargs):
        self.replication_key = replication_key
        super().__init__(**kwargs)
        self._stream_slice_step = stream_slice_step
        self._stream_slicer_cursor = None
        self._state = {}

    def set_cursor(self, cursor: ConcurrentCursor) -> None:
        self._stream_slicer_cursor = cursor

    def stream_slices(
        self,
        *,
        sync_mode: SyncMode,
        cursor_field: List[str] = None,
        stream_state: Mapping[str, Any] = None,
    ) -> Iterable[Optional[Mapping[str, Any]]]:
        if not self._stream_slicer_cursor:
            yield from [StreamSlice(partition={}, cursor_slice={})]
            return

        for stream_slice in self._stream_slicer_cursor.stream_slices():
            yield StreamSlice(
                partition={},
                cursor_slice={
                    "start_date": stream_slice["start_date"].replace("Z", "+00:00"),
                    "end_date": stream_slice["end_date"].replace("Z", "+00:00"),
                },
            )

    @property
    def stream_slice_step(self) -> pendulum.Duration:
        return pendulum.parse(self._stream_slice_step)

    def request_params(
        self,
        stream_state: Mapping[str, Any],
        stream_slice: Mapping[str, Any] = None,
        next_page_token: Mapping[str, Any] = None,
        property_chunk: Mapping[str, Any] = None,
    ) -> MutableMapping[str, Any]:
        if next_page_token:
            """
            If `next_page_token` is set, subsequent requests use `nextRecordsUrl`, and do not include any parameters.
            """
            return {}

        property_chunk = property_chunk or {}
        select_fields = ",".join(property_chunk.keys())
        table_name = self.name

        if not self._stream_slicer_cursor:
            query = f"SELECT {select_fields} FROM {table_name}"
            return {"q": query}

        # Normalize cursor boundaries to canonical SOQL datetime literals
        # (YYYY-MM-DDTHH:MM:SS.sss+00:00) before interpolation. Inputs can
        # arrive as ISO variants from state / slices / page tokens; unparsable
        # values drop to "" so the filter is simply omitted rather than
        # producing a malformed SOQL predicate.
        def _soql_dt(value: Any) -> str:
            if not value:
                return ""
            try:
                return pendulum.parse(str(value)).in_timezone("UTC").isoformat(timespec="milliseconds")
            except (ParserError, ValueError):
                return ""

        candidates = [
            _soql_dt((stream_state or {}).get(self.cursor_field, self.start_date)),
            _soql_dt((stream_slice or {}).get("start_date", "")),
            _soql_dt((next_page_token or {}).get("start_date", "")),
        ]
        start_date = max(c for c in candidates if c) if any(candidates) else ""
        end_date = _soql_dt(
            (stream_slice or {}).get(
                "end_date",
                pendulum.now(tz="UTC").isoformat(timespec="milliseconds"),
            )
        )

        where_conditions = []

        if start_date:
            where_conditions.append(f"{self.cursor_field} >= {start_date}")
        if end_date:
            where_conditions.append(f"{self.cursor_field} < {end_date}")

        where_clause = f"WHERE {' AND '.join(where_conditions)}"
        query = f"SELECT {select_fields} FROM {table_name} {where_clause}"

        return {"q": query}

    @property
    def cursor_field(self) -> str:
        return self.replication_key

    @property
    def state(self):
        return self._state

    @state.setter
    def state(self, value):
        self._state = value
