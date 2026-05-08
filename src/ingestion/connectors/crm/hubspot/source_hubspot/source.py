"""HubSpot source entry point.

Wires Private App auth and stream construction from the curated registry.
Streams sync sequentially via the classic ``AbstractSource`` path: HubSpot's
search endpoint is rate-limited to 4 rps portal-wide, so concurrency adds
429 retries rather than throughput. Each stream manages its own cursor
state via the CDK ``Stream.state`` property.
"""

from __future__ import annotations

import json
import logging
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, List, Mapping, Optional, Tuple

import pendulum
from dateutil.relativedelta import relativedelta

from airbyte_cdk.models import (
    ConnectorSpecification,
    FailureType,
)
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream
from airbyte_cdk.utils.traced_exception import AirbyteTracedException

from source_hubspot.api import Hubspot
from source_hubspot.constants import (
    ARCHIVED_STREAM_SUFFIX,
    CURATED_STREAMS,
    STREAM_REGISTRY,
)
from source_hubspot.streams import (
    CrmArchivedListStream,
    CrmSearchStream,
    HubspotStream,
    OwnersArchivedStream,
    OwnersStream,
)


logger = logging.getLogger("airbyte")

_START_DATE_FALLBACK_YEARS = 2


class SourceHubspot(AbstractSource):
    DATETIME_FORMAT = "%Y-%m-%dT%H:%M:%SZ"

    # ------- Spec / check ---------------------------------------------------

    def spec(self, logger_: logging.Logger) -> ConnectorSpecification:
        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self, logger: logging.Logger, config: Mapping[str, Any]
    ) -> Tuple[bool, Optional[str]]:
        hubspot = Hubspot(access_token=config["hubspot_access_token"])
        reason = hubspot.check_connection()
        if reason is not None:
            return False, reason
        # Probe property discovery on a stream the operator actually enabled
        # — hard-coding "contacts" would false-fail tokens scoped to, say,
        # deals-only portals. Owners skipped because it has no properties
        # endpoint.
        probe_object = self._pick_properties_probe_object(config)
        if probe_object is not None:
            try:
                hubspot.properties_for(probe_object)
            except AirbyteTracedException as exc:
                return False, exc.message
        # Verify the token has association read scope only when at least one
        # enabled stream actually uses associations. Owners-only or
        # leads/tickets-only configs (no association fetches) shouldn't be
        # rejected for missing a scope they'll never call.
        if self._has_association_streams(config):
            reason = hubspot.probe_association_scope()
            if reason is not None:
                return False, reason
        return True, None

    def _has_association_streams(self, config: Mapping[str, Any]) -> bool:
        for name in self._resolve_stream_list(config):
            entry = STREAM_REGISTRY.get(name)
            if entry and entry.get("associations"):
                return True
        return False

    def _pick_properties_probe_object(
        self, config: Mapping[str, Any]
    ) -> Optional[str]:
        """First requested stream that has a CRM properties endpoint."""
        for name in self._resolve_stream_list(config):
            entry = STREAM_REGISTRY.get(name)
            if entry is None:
                continue
            obj = entry.get("object_type")
            if obj and obj != "owners":
                return obj
        return None

    # ------- Stream discovery ----------------------------------------------

    def streams(self, config: Mapping[str, Any]) -> List[Stream]:
        start_date = self._resolve_start_date(config)
        hubspot = Hubspot(access_token=config["hubspot_access_token"])
        return [
            self._build_stream(name, config, hubspot, start_date)
            for name in self._resolve_stream_list(config)
        ]

    def _build_stream(
        self,
        stream_name: str,
        config: Mapping[str, Any],
        hubspot: Hubspot,
        start_date: pendulum.DateTime,
    ) -> HubspotStream:
        kwargs = dict(
            stream_name=stream_name,
            hubspot_api=hubspot,
            access_token=config["hubspot_access_token"],
            tenant_id=config["insight_tenant_id"],
            source_id=config["insight_source_id"],
            start_date=start_date,
        )
        registry = STREAM_REGISTRY[stream_name]
        is_archived = bool(registry.get("is_archived"))
        is_owners = registry.get("object_type") == "owners"
        if is_archived and is_owners:
            return OwnersArchivedStream(**kwargs)
        if is_archived:
            return CrmArchivedListStream(**kwargs)
        if is_owners:
            return OwnersStream(**kwargs)
        return CrmSearchStream(**kwargs)

    # ------- Config helpers ------------------------------------------------

    def _resolve_start_date(self, config: Mapping[str, Any]) -> pendulum.DateTime:
        raw = config.get("hubspot_start_date")
        if raw:
            try:
                parsed = pendulum.parse(raw)
            except Exception as exc:
                raise AirbyteTracedException(
                    message=(
                        f"Invalid hubspot_start_date {raw!r}. "
                        "Expected YYYY-MM-DD or YYYY-MM-DDTHH:MM:SSZ."
                    ),
                    internal_message=str(exc),
                    failure_type=FailureType.config_error,
                ) from exc
            # pendulum.parse can return Date or Time for partial inputs.
            # Always normalize to UTC so cursor math doesn't inherit a
            # locale-dependent offset from the input string.
            if isinstance(parsed, pendulum.DateTime):
                return parsed.in_timezone("UTC")
            if isinstance(parsed, pendulum.Date):
                return pendulum.datetime(
                    parsed.year, parsed.month, parsed.day, tz="UTC"
                )
            raise AirbyteTracedException(
                message=(
                    f"hubspot_start_date {raw!r} parsed as {type(parsed).__name__}; "
                    "expected a date or datetime."
                ),
                internal_message=f"type={type(parsed).__name__}",
                failure_type=FailureType.config_error,
            )
        fallback = datetime.now(timezone.utc) - relativedelta(years=_START_DATE_FALLBACK_YEARS)
        return pendulum.instance(fallback)

    def _resolve_stream_list(self, config: Mapping[str, Any]) -> List[str]:
        names = list(CURATED_STREAMS)
        for live_name in CURATED_STREAMS:
            archived_name = f"{live_name}{ARCHIVED_STREAM_SUFFIX}"
            if archived_name in STREAM_REGISTRY:
                names.append(archived_name)
        return names


def main() -> None:
    """CLI entry-point used by the Docker ENTRYPOINT and pyproject console script."""
    import sys
    from airbyte_cdk.entrypoint import launch

    source = SourceHubspot()
    launch(source, sys.argv[1:])


if __name__ == "__main__":
    main()
