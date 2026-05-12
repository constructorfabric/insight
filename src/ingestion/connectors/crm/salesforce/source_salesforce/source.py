"""Salesforce source entry point.

Wires together authentication (OAuth Client Credentials), describe-driven
stream discovery, per-sobject REST-vs-Bulk decision, and ``ConcurrentCursor``
for parallel incremental slicing. Config keys are prefixed ``salesforce_*`` /
``insight_*`` so the K8s Secret can carry multiple connectors without
collision.
"""

import json
import logging
from datetime import datetime, timedelta, timezone
from typing import Any, Iterator, List, Mapping, MutableMapping, Optional, Tuple, Union

import isodate
import pendulum
from dateutil.relativedelta import relativedelta
from pendulum.parsing.exceptions import ParserError
from requests import JSONDecodeError, codes, exceptions  # type: ignore[import]

from airbyte_cdk.logger import AirbyteLogFormatter
from airbyte_cdk.models import (
    AirbyteMessage,
    AirbyteStateMessage,
    ConfiguredAirbyteCatalog,
    ConfiguredAirbyteStream,
    FailureType,
    Level,
    SyncMode,
)
from airbyte_cdk.sources.concurrent_source.concurrent_source import ConcurrentSource
from airbyte_cdk.sources.concurrent_source.concurrent_source_adapter import ConcurrentSourceAdapter
from airbyte_cdk.sources.connector_state_manager import ConnectorStateManager
from airbyte_cdk.sources.declarative.async_job.job_tracker import JobTracker
from airbyte_cdk.sources.message import InMemoryMessageRepository
from airbyte_cdk.sources.source import TState
from airbyte_cdk.sources.streams import Stream
from airbyte_cdk.sources.streams.concurrent.adapters import StreamFacade
from airbyte_cdk.sources.streams.concurrent.cursor import ConcurrentCursor, CursorField, FinalStateCursor
from airbyte_cdk.sources.streams.http.requests_native_auth import TokenAuthenticator
from airbyte_cdk.sources.utils.schema_helpers import InternalConfig
from airbyte_cdk.utils.traced_exception import AirbyteTracedException

from pathlib import Path
from airbyte_cdk.models import ConnectorSpecification

from source_salesforce.api import Salesforce
from source_salesforce.constants import (
    PARENT_SALESFORCE_OBJECTS,
    UNSUPPORTED_BULK_API_SALESFORCE_OBJECTS,
    UNSUPPORTED_FILTERING_STREAMS,
)
from source_salesforce.streams import (
    DEFAULT_LOOKBACK_SECONDS,
    BulkIncrementalSalesforceStream,
    BulkSalesforceStream,
    BulkSalesforceSubStream,
    IncrementalRestSalesforceStream,
    RestSalesforceStream,
    RestSalesforceSubStream,
)


_DEFAULT_CONCURRENCY = 20
_MAX_CONCURRENCY = 50
logger = logging.getLogger("airbyte")


class SourceSalesforce(ConcurrentSourceAdapter):
    DATETIME_FORMAT = "%Y-%m-%dT%H:%M:%SZ"
    START_DATE_OFFSET_IN_YEARS = 2
    stop_sync_on_stream_failure = True
    message_repository = InMemoryMessageRepository(Level(AirbyteLogFormatter.level_mapping[logger.level]))

    def __init__(self, catalog: Optional[ConfiguredAirbyteCatalog], config: Optional[Mapping[str, Any]], state: Optional[TState], **kwargs):
        if config:
            raw = config.get("salesforce_num_workers", _DEFAULT_CONCURRENCY)
            try:
                parsed = int(raw)
            except (TypeError, ValueError):
                parsed = _DEFAULT_CONCURRENCY
            # K8s Secret values arrive as strings; parse + clamp to spec bounds [1, _MAX].
            concurrency_level = max(1, min(parsed, _MAX_CONCURRENCY))
        else:
            concurrency_level = _DEFAULT_CONCURRENCY
        logger.info(f"Using concurrent cdk with concurrency level {concurrency_level}")
        concurrent_source = ConcurrentSource.create(
            concurrency_level, concurrency_level // 2, logger, self._slice_logger, self.message_repository
        )
        super().__init__(concurrent_source)
        self.catalog = catalog
        self.state = state
        self._job_tracker = JobTracker(limit=100)

    def spec(self, logger_: logging.Logger) -> ConnectorSpecification:
        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    @staticmethod
    def _get_sf_object(config: Mapping[str, Any]) -> Salesforce:
        """Instantiate the Salesforce client and authenticate.

        Config keys are prefixed to avoid collisions in shared K8s Secrets.
        Only the ``salesforce_*`` keys are passed to the client.
        """
        sf = Salesforce(
            instance_url=config["salesforce_instance_url"],
            client_id=config["salesforce_client_id"],
            client_secret=config["salesforce_client_secret"],
            start_date=config.get("salesforce_start_date"),
        )
        sf.login()
        return sf

    @staticmethod
    def _to_timedelta(duration: Any) -> Optional[timedelta]:
        # isodate.parse_duration returns either a datetime.timedelta or an
        # isodate.Duration (for year/month-bearing durations). Normalize to
        # timedelta so comparisons are well-defined; return None for Duration
        # shapes we can't compare (year/month can't be reduced to a fixed delta).
        if isinstance(duration, timedelta):
            return duration
        to_td = getattr(duration, "totimedelta", None)
        if callable(to_td):
            try:
                return to_td(start=datetime.now(timezone.utc))
            except Exception:
                return None
        return None

    @staticmethod
    def _validate_stream_slice_step(stream_slice_step: str):
        if stream_slice_step:
            try:
                duration = isodate.parse_duration(stream_slice_step)
                td = SourceSalesforce._to_timedelta(duration)
                if td is None:
                    message = "Stream slice step Interval should be provided in ISO 8601 format."
                elif td < timedelta(seconds=1):
                    message = "Stream slice step Interval is too small. It should be no less than 1 second. Please set higher value and try again."
                else:
                    return
                raise ParserError(message)
            except (ParserError, isodate.ISO8601Error) as e:
                internal_message = "Incorrect stream slice step"
                raise AirbyteTracedException(failure_type=FailureType.config_error, internal_message=internal_message, message=e.args[0] if e.args else internal_message)

    @staticmethod
    def _validate_lookback_window(lookback_window: str):
        if lookback_window:
            try:
                duration = isodate.parse_duration(lookback_window)
                td = SourceSalesforce._to_timedelta(duration)
                if td is None:
                    message = "Lookback window should be provided in ISO 8601 duration format."
                elif td < timedelta(seconds=0):
                    message = "Lookback window must not be negative."
                else:
                    return
                raise ParserError(message)
            except (ParserError, isodate.ISO8601Error) as e:
                internal_message = str(e) if e.args else "Lookback window parsing failed"
                raise AirbyteTracedException(
                    failure_type=FailureType.config_error,
                    internal_message=internal_message,
                    message=f"The lookback_window value is invalid: {internal_message.rstrip('.')}. Please provide a valid ISO 8601 duration (e.g., 'PT10M' for 10 minutes, 'PT1H' for 1 hour). See https://docs.airbyte.com/integrations/sources/salesforce#limitations--troubleshooting for more details.",
                )

    def check_connection(self, logger: logging.Logger, config: Mapping[str, Any]) -> Tuple[bool, Optional[str]]:
        self._validate_stream_slice_step(config.get("salesforce_stream_slice_step"))
        self._validate_lookback_window(config.get("salesforce_lookback_window"))
        salesforce = self._get_sf_object(config)
        salesforce.describe()
        return True, None

    @classmethod
    def _get_api_type(
        cls, stream_name: str, json_schema: Mapping[str, Any], force_use_bulk_api: bool
    ) -> str:
        """Return ``"rest"`` or ``"bulk"`` for the given stream.

        Bulk API cannot emit base64 or compound (object) fields as CSV columns,
        so any sobject carrying those drops to REST unless the operator sets
        ``force_use_bulk_api`` (accepting data loss for those fields).
        """
        properties = json_schema.get("properties") or {}
        not_bulk_safe = {
            key: value
            for key, value in properties.items()
            if isinstance(value, Mapping)
            and (
                value.get("format") == "base64"
                or "object" in (value.get("type") or [])
            )
        }
        if stream_name in UNSUPPORTED_BULK_API_SALESFORCE_OBJECTS:
            logger.warning("Bulk API not supported for stream '%s'; using REST", stream_name)
            return "rest"
        if force_use_bulk_api and not_bulk_safe:
            logger.warning(
                "Excluding non-Bulk fields from stream '%s': %s",
                stream_name,
                list(not_bulk_safe),
            )
            return "bulk"
        if not_bulk_safe:
            return "rest"
        return "bulk"

    @classmethod
    def _get_stream_type(cls, stream_name: str, api_type: str):
        """Get proper stream class: full_refresh, incremental or substream

        SubStreams (like ContentDocumentLink) do not support incremental sync because of query restrictions, look here:
        https://developer.salesforce.com/docs/atlas.en-us.object_reference.meta/object_reference/sforce_api_objects_contentdocumentlink.htm
        """
        parent_name = PARENT_SALESFORCE_OBJECTS.get(stream_name, {}).get("parent_name")
        if api_type == "rest":
            full_refresh = RestSalesforceSubStream if parent_name else RestSalesforceStream
            incremental = IncrementalRestSalesforceStream
        elif api_type == "bulk":
            full_refresh = BulkSalesforceSubStream if parent_name else BulkSalesforceStream
            incremental = BulkIncrementalSalesforceStream
        else:
            raise Exception(f"Stream {stream_name} cannot be processed by REST or BULK API.")
        return full_refresh, incremental

    def prepare_stream(self, stream_name: str, json_schema, sobject_options, sf_object, authenticator, config):
        """Choose proper stream class: syncMode(full_refresh/incremental), API type(Rest/Bulk), SubStream"""
        pk, replication_key = sf_object.get_pk_and_replication_key(json_schema)
        stream_kwargs = {
            "stream_name": stream_name,
            "schema": json_schema,
            "pk": pk,
            "sobject_options": sobject_options,
            "sf_api": sf_object,
            "authenticator": authenticator,
            "start_date": config.get("salesforce_start_date"),
            "job_tracker": self._job_tracker,
            "message_repository": self.message_repository,
            # Envelope context — tenant_id / source_id / custom_fields split.
            "tenant_id": config["insight_tenant_id"],
            "source_id": config["insight_source_id"],
            "custom_field_names": sf_object.get_custom_field_names(stream_name),
        }

        api_type = self._get_api_type(
            stream_name, json_schema, config.get("salesforce_force_use_bulk_api", False)
        )
        full_refresh, incremental = self._get_stream_type(stream_name, api_type)
        if replication_key and stream_name not in UNSUPPORTED_FILTERING_STREAMS:
            stream_class = incremental
            stream_kwargs["replication_key"] = replication_key
            stream_kwargs["stream_slice_step"] = config.get(
                "salesforce_stream_slice_step", "P30D"
            )
        else:
            stream_class = full_refresh

        return stream_class, stream_kwargs

    def generate_streams(
        self,
        config: Mapping[str, Any],
        stream_objects: Mapping[str, Any],
        sf_object: Salesforce,
    ) -> List[Stream]:
        """Generates a list of stream by their names. It can be used for different tests too"""
        authenticator = TokenAuthenticator(sf_object.access_token)
        schemas = sf_object.generate_schemas(stream_objects)
        default_args = [sf_object, authenticator, config]
        streams = []
        state_manager = ConnectorStateManager(state=self.state)
        for stream_name, sobject_options in stream_objects.items():
            json_schema = schemas.get(stream_name, {})

            stream_class, kwargs = self.prepare_stream(stream_name, json_schema, sobject_options, *default_args)

            parent_name = PARENT_SALESFORCE_OBJECTS.get(stream_name, {}).get("parent_name")
            if parent_name:
                # Minimal schema + sobject_options specific to the parent (not
                # the child's). Child-specific permission flags should not
                # shape the parent stream.
                parent_schema = PARENT_SALESFORCE_OBJECTS.get(stream_name, {}).get("schema_minimal")
                parent_sobject_options = stream_objects.get(parent_name) or {}
                parent_class, parent_kwargs = self.prepare_stream(
                    parent_name, parent_schema, parent_sobject_options, *default_args
                )
                kwargs["parent"] = parent_class(**parent_kwargs)

            stream = stream_class(**kwargs)
            streams.append(self._wrap_for_concurrency(config, stream, state_manager))
        # The Describe meta-stream from upstream is intentionally omitted —
        # Bronze already captures per-field metadata via the generated schema,
        # and the meta-stream would add a large sobjects table that nothing in
        # the downstream dbt pipeline consumes.
        return streams

    def _wrap_for_concurrency(self, config, stream, state_manager):
        stream_slicer_cursor = None
        is_full_refresh = self._get_sync_mode_from_catalog(stream) == SyncMode.full_refresh
        if stream.cursor_field:
            stream_slicer_cursor = self._create_stream_slicer_cursor(config, state_manager, stream)
            if hasattr(stream, "set_cursor") and not is_full_refresh:
                stream.set_cursor(stream_slicer_cursor)
        if hasattr(stream, "parent") and hasattr(stream.parent, "set_cursor"):
            # Build the cursor from the parent's own state, not the child's —
            # they sync independent sobjects.
            parent_cursor = self._create_stream_slicer_cursor(config, state_manager, stream.parent)
            stream.parent.set_cursor(parent_cursor)

        if not stream_slicer_cursor or is_full_refresh:
            cursor = FinalStateCursor(
                stream_name=stream.name, stream_namespace=stream.namespace, message_repository=self.message_repository
            )
            state = None
        else:
            cursor = stream_slicer_cursor
            state = cursor.state
        return StreamFacade.create_from_stream(stream, self, logger, state, cursor)

    def streams(self, config: Mapping[str, Any]) -> List[Stream]:
        if not config.get("salesforce_start_date"):
            config = dict(config)
            config["salesforce_start_date"] = (
                datetime.now() - relativedelta(years=self.START_DATE_OFFSET_IN_YEARS)
            ).strftime(self.DATETIME_FORMAT)
        sf = self._get_sf_object(config)
        stream_objects = sf.get_validated_streams(config=config, catalog=self.catalog)
        streams = self.generate_streams(config, stream_objects, sf)
        return streams

    def _create_stream_slicer_cursor(
        self, config: Mapping[str, Any], state_manager: ConnectorStateManager, stream: Stream
    ) -> ConcurrentCursor:
        """
        We have moved the generation of stream slices to the concurrent CDK cursor
        """
        cursor_field_key = stream.cursor_field or ""
        if not isinstance(cursor_field_key, str):
            raise AssertionError(f"Nested cursor field are not supported hence type str is expected but got {cursor_field_key}.")
        cursor_field = CursorField(cursor_field_key)
        stream_state = state_manager.get_stream_state(stream.name, stream.namespace)
        return ConcurrentCursor(
            stream.name,
            stream.namespace,
            stream_state,
            self.message_repository,
            state_manager,
            stream.state_converter,
            cursor_field,
            self._get_slice_boundary_fields(stream, state_manager),
            datetime.fromtimestamp(pendulum.parse(config["salesforce_start_date"]).timestamp(), timezone.utc),
            stream.state_converter.get_end_provider(),
            isodate.parse_duration(config["salesforce_lookback_window"])
            if "salesforce_lookback_window" in config
            else timedelta(seconds=DEFAULT_LOOKBACK_SECONDS),
            isodate.parse_duration(config["salesforce_stream_slice_step"])
            if "salesforce_stream_slice_step" in config
            else timedelta(days=30),
        )

    def _get_slice_boundary_fields(self, stream: Stream, state_manager: ConnectorStateManager) -> Optional[Tuple[str, str]]:
        return ("start_date", "end_date")

    def _get_sync_mode_from_catalog(self, stream: Stream) -> Optional[SyncMode]:
        if self.catalog:
            for catalog_stream in self.catalog.streams:
                if stream.name == catalog_stream.stream.name:
                    return catalog_stream.sync_mode
        return None

    def read(
        self,
        logger: logging.Logger,
        config: Mapping[str, Any],
        catalog: ConfiguredAirbyteCatalog,
        state: Union[List[AirbyteStateMessage], MutableMapping[str, Any]] = None,
    ) -> Iterator[AirbyteMessage]:
        # save for use inside streams method
        self.catalog = catalog
        yield from super().read(logger, config, catalog, state)

    def _read_stream(
        self,
        logger: logging.Logger,
        stream_instance: Stream,
        configured_stream: ConfiguredAirbyteStream,
        state_manager: ConnectorStateManager,
        internal_config: InternalConfig,
    ) -> Iterator[AirbyteMessage]:
        try:
            yield from super()._read_stream(
                logger, stream_instance, configured_stream, state_manager, internal_config
            )
        except exceptions.HTTPError as error:
            response = error.response
            error_code = None
            message = None
            try:
                payload = response.json()
            except (JSONDecodeError, ValueError):
                payload = None
            if isinstance(payload, list) and payload and isinstance(payload[0], dict):
                error_code = payload[0].get("errorCode")
                message = payload[0].get("message")
            elif isinstance(payload, dict):
                error_code = payload.get("errorCode") or payload.get("error")
                message = payload.get("message") or payload.get("error_description")

            if (
                response.status_code == codes.FORBIDDEN
                and error_code == "REQUEST_LIMIT_EXCEEDED"
            ):
                # 24h rolling quota reached — surface as a hard HTTPError so
                # orchestration treats the sync as failed and alerts/retries.
                logger.warning(
                    "API call %s hit rate limit: %r", response.url, message
                )
            raise


def main() -> None:
    """CLI entry-point used by the Docker ENTRYPOINT and pyproject console script."""
    import sys
    from airbyte_cdk.entrypoint import AirbyteEntrypoint, launch

    args = sys.argv[1:]
    catalog_path = AirbyteEntrypoint.extract_catalog(args)
    config_path = AirbyteEntrypoint.extract_config(args)
    state_path = AirbyteEntrypoint.extract_state(args)
    source = SourceSalesforce(
        SourceSalesforce.read_catalog(catalog_path) if catalog_path else None,
        SourceSalesforce.read_config(config_path) if config_path else None,
        SourceSalesforce.read_state(state_path) if state_path else None,
    )
    launch(source, args)


if __name__ == "__main__":
    main()
