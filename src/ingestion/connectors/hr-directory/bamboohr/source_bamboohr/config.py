from __future__ import annotations

from pydantic import BaseModel, ConfigDict, ValidationError

from source_bamboohr.parsing import Id, describe

REMEDIES = {
    "insight_tenant_id": (
        "insight_tenant_id MUST be a non-empty tenant UUID; an empty value would "
        "stamp every record with an unscoped tenant, breaking tenant isolation."
    ),
    "insight_source_id": (
        "insight_source_id MUST be set via the `insight.cyberfabric.com/source-id` "
        "annotation; an empty value would cause silent dedup collisions."
    ),
    "bamboohr_api_key": "bamboohr_api_key MUST be set — generate one under Account > API Keys.",
    "bamboohr_domain": "bamboohr_domain MUST be the BambooHR subdomain (the 'acme' of acme.bamboohr.com).",
}


class BambooConfig(BaseModel):
    model_config = ConfigDict(frozen=True)

    insight_tenant_id: Id
    insight_source_id: Id
    bamboohr_api_key: Id
    bamboohr_domain: Id
    bamboohr_start_date: str | None = None


def config_problem(exc: ValidationError) -> str:
    """The remedy for the first field the config got wrong; a field without one
    is named by path and error type."""
    for error in exc.errors(include_input=False):
        remedy = REMEDIES.get(str(error["loc"][0])) if error["loc"] else None
        if remedy is not None:
            return remedy

    return f"Invalid config: {describe(exc)}"
