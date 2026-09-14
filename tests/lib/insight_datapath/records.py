"""Bronze records built in code, composed from the same templates a fixture uses.

A test that needs a run-unique person or a shape no fixture states builds its rows
here. They still come from `metrics/templates`, so a column the connector adds reaches
them the moment it reaches the template.
"""

from __future__ import annotations

import uuid
from collections.abc import Mapping
from pathlib import Path
from typing import Any

from insight_datapath.fixture_loader import resolve_record

_REPO_ROOT = Path(__file__).resolve().parents[3]
METRICS_DIR = _REPO_ROOT / "tests" / "datapath" / "metrics"
SCHEMAS_DIR = METRICS_DIR / "schemas"

#: A path inside `metrics/`, so a `templates/...` ref resolves as it does from a spec.
_ANCHOR = METRICS_DIR / "records.py"

EMPLOYEE_TEMPLATE = "people.yaml#/templates/bamboohr_employee"

EXTRACTED_AT = "2026-01-05T00:00:00"
OBSERVED_AT = "2026-01-05T00:00:00Z"
GITHUB_ORG = "acme"


def from_template(ref: str, *, substitutions: Mapping[str, str], **fields: Any) -> dict[str, Any]:
    """`from_template("people.yaml#/templates/bamboohr_employee", ..., id="e001")`.

    `fields` override the template; a `None` clears a column the template fills.
    """
    return resolve_record(
        {"$ref": f"templates/{ref}", **fields}, anchor=_ANCHOR, substitutions=substitutions
    )


def employee(
    *,
    substitutions: Mapping[str, str],
    key: str,
    email: str,
    display_name: str,
    supervisor_email: str | None = None,
) -> dict[str, Any]:
    """One HR record. Without `supervisor_email` it reports to the lane's caller."""
    first, _, last = display_name.partition(" ")
    overrides: dict[str, Any] = {
        "id": key,
        "unique_key": f"bamboohr-test-{key}",
        "firstName": first,
        "lastName": last or None,
        "displayName": display_name,
        "workEmail": email,
    }
    if supervisor_email is not None:
        overrides["supervisorEmail"] = supervisor_email
        overrides["supervisor"] = None
    return from_template(EMPLOYEE_TEMPLATE, substitutions=substitutions, **overrides)


def framing() -> dict[str, Any]:
    """The four CDK columns every bronze row carries."""
    return {
        "_airbyte_raw_id": str(uuid.uuid4()),
        "_airbyte_extracted_at": EXTRACTED_AT,
        "_airbyte_meta": "{}",
        "_airbyte_generation_id": 0,
    }


def org_member(
    *, tenant: str, source_id: str, login: str, member_id: int, email: str, name: str
) -> dict[str, Any]:
    """One GitHub organisation member (bronze_github_directory.org_members)."""
    return {
        **framing(),
        "tenant_id": tenant,
        "source_id": source_id,
        "unique_key": f"{tenant}:{source_id}:{GITHUB_ORG}:{member_id}",
        "collected_at": OBSERVED_AT,
        "data_source": "insight_github",
        "org": GITHUB_ORG,
        "login": login,
        "member_id": member_id,
        "name": name,
        "email": email,
        "company": "Example Corp",
        "role": "MEMBER",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-04T00:00:00Z",
    }
