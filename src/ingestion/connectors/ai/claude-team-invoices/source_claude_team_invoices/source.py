"""Claude Team invoices — Airbyte source connector entry point.

One stream, `claude_team_invoice_lines`: the invoiced layer for Claude Team.

Python CDK rather than a declarative manifest, for the same reason as
github-copilot ADR-0001. The invoice list is trivially declarative, but the
line items behind it are reached through a three-hop chain across two hosts in
which each hop's credential comes out of the previous hop's response body. The
manifest framework has no way to express that, and the credential must not be
persisted into a record or state — which a parent stream would do.
"""

import json
import logging
import sys
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import requests
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_claude_team_invoices.streams import InvoiceLines

logger = logging.getLogger("airbyte")

_SCHEMAS = Path(__file__).parent / "schemas"


def load_stream_schema(name: str) -> Mapping[str, Any]:
    """Read a stream's JSON schema from the packaged `schemas/` directory."""
    return json.loads((_SCHEMAS / f"{name}.json").read_text(encoding="utf-8"))


class SourceClaudeTeamInvoices(AbstractSource):
    """Entry point for the Claude Team invoices connector."""

    def spec(self, logger: Any) -> Mapping[str, Any]:
        from airbyte_cdk.models import ConnectorSpecification

        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text(encoding="utf-8")))

    def check_connection(self, logger: Any, config: Mapping[str, Any]) -> tuple[bool, Any | None]:
        """Validate the config against the two things that can independently fail.

        The proxy and the session key behind it are one failure domain; egress
        to Stripe is another, and a green proxy with blocked egress would
        produce invoices with no prices — the exact silent-emptiness this
        connector exists to avoid. Both are checked before a sync is allowed.
        """
        source_id = (config.get("insight_source_id") or "").strip()
        if not source_id:
            return False, (
                "insight_source_id MUST be set via the "
                "`insight.cyberfabric.com/source-id` annotation; an empty value "
                "would collide unique_keys across connector instances."
            )

        proxy_url = str(config["proxy_url"]).rstrip("/")
        org_id = config["claude_org_id"]
        try:
            response = requests.get(
                f"{proxy_url}/api/stripe/{org_id}/invoices?limit=1&page=",
                headers={"Authorization": f"Bearer {config['proxy_auth_token']}"},
                timeout=30,
            )
        except requests.RequestException as error:
            return False, f"proxy unreachable at {proxy_url}: {error}"

        if response.status_code == 403:
            return False, (
                "the proxy session key is not permitted to read invoices "
                "(HTTP 403). Rotate a billing-capable cookie in via the proxy's "
                "POST /admin/session-key."
            )
        if response.status_code != 200:
            return False, f"invoice list returned HTTP {response.status_code}"
        try:
            listing = response.json()
        except ValueError:
            return False, "invoice list answered 200 with a body that is not JSON"
        if not isinstance(listing, dict):
            return False, f"invoice list answered a {type(listing).__name__}, not an object"
        if not isinstance(listing.get("invoices"), list):
            # A sync extends this value; anything else fails mid-run, after
            # this check has already reported the source healthy.
            return False, "invoice list did not carry an `invoices` array"

        try:
            # Unauthenticated and harmless: it proves the host resolves and TLS
            # completes. A credentialled call cannot be made here — the key it
            # would need only exists mid-chain.
            requests.get("https://api.stripe.com/v1", timeout=15)
        except requests.RequestException as error:
            return False, (
                f"api.stripe.com is not reachable from this network ({error}); "
                "invoice line items and therefore every seat price would be missing"
            )

        return True, None

    def streams(self, config: Mapping[str, Any]) -> list[Stream]:
        return [InvoiceLines(config)]


def main() -> None:
    from airbyte_cdk.entrypoint import launch

    launch(SourceClaudeTeamInvoices(), sys.argv[1:])
