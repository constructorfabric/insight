from __future__ import annotations

import json
import logging
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from airbyte_cdk.models import ConnectorSpecification
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_bamboohr.client import BambooClient, BambooHrApiError, BambooHrDomainError
from source_bamboohr.streams.employees import EmployeesStream
from source_bamboohr.streams.leave_requests import DEFAULT_START_DATE, LeaveRequestsStream
from source_bamboohr.streams.meta_fields import MetaFieldsStream

logger = logging.getLogger("airbyte")

REQUIRED_CONFIG_FIELDS = {
    "insight_tenant_id": (
        "insight_tenant_id MUST be a non-empty tenant UUID; an empty value would "
        "stamp every record with an unscoped tenant, breaking tenant isolation."
    ),
    "insight_source_id": (
        "insight_source_id MUST be set via the `insight.cyberfabric.com/source-id` "
        "annotation; an empty value would cause silent dedup collisions."
    ),
    "bamboohr_api_key": "bamboohr_api_key MUST be set — generate one under Account > API Keys.",
    "bamboohr_domain": (
        "bamboohr_domain MUST be the BambooHR subdomain (the 'acme' of acme.bamboohr.com)."
    ),
}


class SourceBamboohr(AbstractSource):
    def spec(self, logger: logging.Logger) -> ConnectorSpecification:

        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self, logger: logging.Logger, config: Mapping[str, Any]
    ) -> tuple[bool, str | None]:
        for field, message in REQUIRED_CONFIG_FIELDS.items():
            if not str(config.get(field) or "").strip():
                return False, message

        try:
            _client(config).get("meta/fields")
        except BambooHrDomainError as exc:
            return False, str(exc)
        except BambooHrApiError as exc:
            return False, str(exc)
        except Exception as exc:  # noqa: BLE001 — any transport failure is a check failure
            return False, (
                f"Could not reach BambooHR for domain '{config['bamboohr_domain']}': {exc}"
            )

        return True, None

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        client = _client(config)
        tenant_id = config["insight_tenant_id"]
        source_id = config["insight_source_id"]

        return [
            EmployeesStream(client=client, tenant_id=tenant_id, source_id=source_id),
            LeaveRequestsStream(
                client=client,
                tenant_id=tenant_id,
                source_id=source_id,
                start_date=config.get("bamboohr_start_date") or DEFAULT_START_DATE,
            ),
            MetaFieldsStream(client=client, tenant_id=tenant_id, source_id=source_id),
        ]


def _client(config: Mapping[str, Any]) -> BambooClient:
    return BambooClient(domain=config["bamboohr_domain"], api_key=config["bamboohr_api_key"])


def main() -> None:
    from airbyte_cdk.entrypoint import launch

    launch(SourceBamboohr(), sys.argv[1:])


if __name__ == "__main__":
    main()
