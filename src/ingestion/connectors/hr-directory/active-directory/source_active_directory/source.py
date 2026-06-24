"""Active Directory (LDAP) Airbyte source connector — entry point.

One stream:
  users   full-refresh, LDAP paged search over the configured search base

ADR (Python CDK over declarative manifest): Active Directory is queried over LDAP,
not HTTP. The declarative manifest framework and the CDK's HttpStream both assume an
HTTP transport, so neither can express an LDAP bind + paged search. This connector
therefore implements a plain `Stream` whose `read_records` drives `ldap3` directly.

Sibling of the `ms-entra` connector (cloud Entra ID via Microsoft Graph): both emit
the same `class_people` / `identity_inputs` Silver contract from a user directory;
only the transport differs.
"""

import json
import logging
import sys
from pathlib import Path
from typing import Any, List, Mapping, Optional, Tuple

from airbyte_cdk.sources import AbstractSource
from airbyte_cdk.sources.streams import Stream

from source_active_directory import ldap_client

logger = logging.getLogger("airbyte")


class SourceActiveDirectory(AbstractSource):
    """Entry point for the on-prem Active Directory connector."""

    def spec(self, logger: Any) -> Mapping[str, Any]:
        from airbyte_cdk.models import ConnectorSpecification

        spec_path = Path(__file__).parent / "spec.json"
        return ConnectorSpecification(**json.loads(spec_path.read_text()))

    def check_connection(
        self,
        logger: Any,
        config: Mapping[str, Any],
    ) -> Tuple[bool, Optional[Any]]:
        """Validate config end-to-end:

          1. insight_source_id is non-empty (composite unique_key collisions otherwise).
          2. LDAP bind succeeds against the configured DC with the bind credentials.
          3. The search base is readable (size-limited probe returns without error).
        """
        insight_source_id = (config.get("insight_source_id") or "").strip()
        if not insight_source_id:
            return False, (
                "insight_source_id MUST be set via the "
                "`insight.cyberfabric.com/source-id` annotation; an empty value "
                "would cause silent dedup collisions in the users stream."
            )

        # Import here so a missing ldap3 surfaces as a clean check error, not an
        # import-time crash of the whole connector.
        from ldap3.core.exceptions import LDAPException

        conn = None
        try:
            conn = ldap_client.connect(config)
        except LDAPException as exc:
            return False, (
                f"LDAP bind failed against '{config.get('ad_server_host')}': {exc}. "
                "Check ad_server_host/ad_port/ad_use_ssl, and that ad_bind_dn + "
                "ad_bind_password are correct for a read-enabled account."
            )

        try:
            ok = conn.search(
                search_base=config["ad_search_base"],
                search_filter=config.get(
                    "ad_user_filter",
                    "(&(objectClass=user)(objectCategory=person)(!(sAMAccountName=krbtgt)))",
                ),
                search_scope="SUBTREE",
                attributes=["sAMAccountName"],
                size_limit=1,
            )
            if not ok and conn.result and conn.result.get("result") not in (0, 4):
                # result 0 = success, 4 = sizeLimitExceeded (expected with size_limit=1)
                return False, (
                    f"Search base '{config['ad_search_base']}' not readable: "
                    f"{conn.result.get('description')} ({conn.result.get('message')})."
                )
            return True, None
        except LDAPException as exc:
            return False, f"LDAP search against '{config['ad_search_base']}' failed: {exc}"
        finally:
            if conn is not None:
                try:
                    conn.unbind()
                except Exception:  # noqa: BLE001 — best-effort cleanup
                    pass

    def streams(self, config: Mapping[str, Any]) -> List[Stream]:
        from source_active_directory.streams.users import ActiveDirectoryUsersStream

        return [
            ActiveDirectoryUsersStream(
                config=config,
                tenant_id=config["insight_tenant_id"],
                source_id=config["insight_source_id"],
            )
        ]


def main():
    """Airbyte runner entry point — invoked from the Dockerfile ENTRYPOINT."""
    from airbyte_cdk.entrypoint import launch

    source = SourceActiveDirectory()
    launch(source, sys.argv[1:])


if __name__ == "__main__":
    main()
