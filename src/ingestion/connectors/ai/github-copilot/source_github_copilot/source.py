"""GitHub Copilot Airbyte source connector — entry point.

Three streams:
  copilot_seats           full-refresh, paginated
  copilot_user_metrics    incremental, P1D, NDJSON via signed URL
  copilot_org_metrics     incremental, P1D, NDJSON via signed URL

ADR-0001 (Python CDK over declarative manifest): the two-step signed URL pattern
plus per-request Authorization header suppression on copilot-reports.github.com
cannot be expressed in the declarative manifest framework. See
docs/components/connectors/ai/github-copilot/specs/ADR/0001-python-cdk-over-declarative-manifest.md
"""

import json
import logging
import sys
from pathlib import Path
from typing import Any, List, Mapping, Optional, Tuple

import requests
from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_github_copilot.auth import rest_headers

logger = logging.getLogger("airbyte")


class SourceGitHubCopilot(AbstractSource):
    """Entry point for the GitHub Copilot connector."""

    def spec(self, logger: Any) -> Mapping[str, Any]:
        from airbyte_cdk.models import ConnectorSpecification

        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self,
        logger: Any,
        config: Mapping[str, Any],
    ) -> Tuple[bool, Optional[Any]]:
        """Validate connector config end-to-end.

        Four checks (in order):
          1. insight_source_id is non-empty (per DESIGN §3.3 — composite unique_key
             collisions otherwise).
          2. PAT is valid against GitHub API (`/rate_limit` returns 200).
          3. PAT can read the seats endpoint for the configured org (validates org
             existence and that the token has `manage_billing:copilot` scope).
          4. Metrics reports endpoint is accessible (validates that the "Copilot usage
             metrics" policy is enabled in org settings — 403 here means the policy is
             off, not a missing scope).
        """
        insight_source_id = (config.get("insight_source_id") or "").strip()
        if not insight_source_id:
            return False, (
                "insight_source_id MUST be set via the "
                "`insight.cyberfabric.com/source-id` annotation; an empty value "
                "would cause silent dedup collisions in copilot_org_metrics."
            )

        token = config["github_token"]
        org = config["github_org"]
        headers = rest_headers(token)

        try:
            # 1. Token validity
            resp = requests.get(
                "https://api.github.com/rate_limit",
                headers=headers,
                timeout=10,
            )
            if resp.status_code == 401:
                return False, (
                    "GitHub PAT is invalid or expired (HTTP 401). "
                    "Generate a new classic PAT with `manage_billing:copilot` scope."
                )
            if resp.status_code != 200:
                return False, (
                    f"Token validation failed (HTTP {resp.status_code}): "
                    f"{resp.text[:200]}"
                )

            # 2. Seats endpoint access — validates org + manage_billing:copilot scope
            resp = requests.get(
                f"https://api.github.com/orgs/{org}/copilot/billing/seats?per_page=1",
                headers=headers,
                timeout=15,
            )
            if resp.status_code == 404:
                return False, (
                    f"Organization '{org}' not found, or Copilot is not enabled "
                    "for this org, or the PAT was not created by an Organization Owner."
                )
            if resp.status_code == 403:
                return False, (
                    f"Token lacks `manage_billing:copilot` scope (HTTP 403 on /copilot/billing/seats). "
                    "Recreate the PAT with `manage_billing:copilot` scope and retry."
                )
            if resp.status_code != 200:
                return False, (
                    f"Failed to access seats endpoint for org '{org}' (HTTP "
                    f"{resp.status_code}): {resp.text[:200]}"
                )

            # 3. Metrics reports endpoint access — validates that the "Copilot usage
            # metrics" org policy is enabled. The endpoint rejects dates outside
            # `[today-365d, today)` with HTTP 400, so probe with yesterday UTC —
            # that's both inside the allowed window and guaranteed to be a fully
            # closed reporting day. Possible outcomes:
            #   200/204 — endpoint reachable and policy enabled (data may or
            #             may not exist for the day; both are fine for a probe)
            #   403     — Copilot usage metrics policy disabled (or token lacks
            #             a sufficient scope; the seats probe above already
            #             eliminates the scope path)
            #   400     — should not happen for yesterday; treat as FAIL
            from datetime import datetime, timedelta, timezone
            probe_date = (datetime.now(timezone.utc) - timedelta(days=1)).strftime("%Y-%m-%d")
            resp = requests.get(
                f"https://api.github.com/orgs/{org}/copilot/metrics/reports/users-1-day",
                headers=headers,
                params={"day": probe_date},
                timeout=15,
            )
            if resp.status_code == 403:
                return False, (
                    f"Metrics reports endpoint returned HTTP 403 for org '{org}'. "
                    "Most likely cause: the 'Copilot usage metrics' policy is disabled. "
                    "Enable it under GitHub Organization Settings → Copilot → Policies → "
                    "'Copilot usage metrics'. "
                    f"API response: {resp.text[:200]}"
                )
            if resp.status_code == 404:
                return False, (
                    f"Metrics reports endpoint returned HTTP 404 for org '{org}'. "
                    "The org may not have the Copilot Metrics API enabled, or does not "
                    "have an active Copilot subscription. "
                    f"API response: {resp.text[:200]}"
                )
            if resp.status_code not in (200, 204):
                # 200 = data exists; 204 = no data for the day (still a valid OK).
                return False, (
                    f"Failed to probe metrics endpoint for org '{org}' (HTTP "
                    f"{resp.status_code}): {resp.text[:200]}"
                )

            return True, None
        except requests.RequestException as exc:
            return False, f"GitHub API request failed: {exc}"

    def streams(self, config: Mapping[str, Any]) -> List[Stream]:
        """Instantiate the three streams shipped by this connector."""
        # Lazy imports to keep module import cheap and avoid cycles
        from source_github_copilot.streams.org_metrics import CopilotOrgMetricsStream
        from source_github_copilot.streams.seats import CopilotSeatsStream
        from source_github_copilot.streams.user_metrics import CopilotUserMetricsStream

        shared = {
            "token": config["github_token"],
            "tenant_id": config["insight_tenant_id"],
            "source_id": config["insight_source_id"],
            "org": config["github_org"],
        }
        start_date = config.get("github_start_date")
        # Trailing re-fetch window for the daily report streams. Copilot reports
        # lag (often >24-48h; weekends later) and can be restated, so each run
        # re-queries the last N days to self-heal late/adjusted days that a plain
        # cursor+1 would permanently skip. RMT dedup on unique_key makes it idempotent.
        try:
            lookback_days = int(config.get("metrics_lookback_days", 7))
        except (TypeError, ValueError):
            lookback_days = 7

        return [
            CopilotSeatsStream(**shared),
            CopilotUserMetricsStream(start_date=start_date, lookback_days=lookback_days, **shared),
            CopilotOrgMetricsStream(start_date=start_date, lookback_days=lookback_days, **shared),
        ]


def main():
    """Airbyte runner entry point — invoked from Dockerfile ENTRYPOINT."""
    from airbyte_cdk.entrypoint import launch

    source = SourceGitHubCopilot()
    launch(source, sys.argv[1:])


if __name__ == "__main__":
    main()
