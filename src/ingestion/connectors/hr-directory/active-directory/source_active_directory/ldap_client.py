"""LDAP transport helpers for the Active Directory connector.

Active Directory is an LDAP directory, not an HTTP API — so this connector cannot
use the declarative manifest framework or the CDK's HttpStream. It speaks LDAP
directly via `ldap3` and exposes plain `Stream` subclasses (see streams/users.py).

This module centralises everything transport-specific:
  * `connect(config)`        — build a bound `ldap3.Connection` from connector config
  * `USER_ATTRIBUTES`        — the explicit attribute allowlist (privacy by default)
  * `guid_to_str`            — AD `objectGUID` (binary) → canonical GUID string
  * `generalized_time_to_iso`— AD Generalized-Time → ISO-8601
  * `account_enabled`        — decode the ACCOUNTDISABLE bit of `userAccountControl`
"""

import logging
import ssl
import uuid
from datetime import UTC, datetime
from typing import Any

from ldap3 import ALL, Connection, Server, Tls

from source_active_directory.config import ActiveDirectoryConfig

logger = logging.getLogger("airbyte")

# Explicit attribute allowlist — fetch ONLY identity-resolution fields, never the
# whole AD object (which can carry photos, addresses, phone numbers, etc.).
# Mirrors the ms-entra `$select` allowlist so the two directory connectors collect
# the same identity surface. Privacy by default (Connector Spec NFR privacy).
USER_ATTRIBUTES: list[str] = [
    "objectGUID",  # stable binary GUID — analog of Entra `id`/oid
    "sAMAccountName",  # legacy pre-Windows-2000 login — analog of onPremisesSamAccountName
    "userPrincipalName",  # UPN (user@domain)
    "mail",  # primary SMTP address
    "proxyAddresses",  # alternate SMTP addresses (multi-valued)
    "displayName",
    "givenName",
    "sn",  # surname
    "employeeID",
    "department",
    "title",  # job title
    "userAccountControl",  # bitmask — ACCOUNTDISABLE (0x2) drives enabled/disabled
    "distinguishedName",
    "manager",  # manager DN (resolved to person downstream, not in v1)
    "whenCreated",  # Generalized-Time
    "whenChanged",  # Generalized-Time
]

# ACCOUNTDISABLE flag in userAccountControl (Microsoft AD schema).
_UAC_ACCOUNTDISABLE = 0x2


def build_server(config: ActiveDirectoryConfig) -> Server:
    """Construct an `ldap3.Server` from connector config."""
    # Require server certificate validation. Private AD CAs are supported via
    # `ad_ca_cert_path` (a CA bundle mounted into the connector pod) — never fall
    # back to CERT_NONE, which skips hostname/certificate checking entirely.
    tls = Tls(validate=ssl.CERT_REQUIRED, ca_certs_file=config.ad_ca_cert_path or None) if config.ad_use_ssl else None
    return Server(
        host=config.ad_server_host,
        port=config.port,
        use_ssl=config.ad_use_ssl,
        tls=tls,
        get_info=ALL,
        connect_timeout=15,
    )


def connect(config: ActiveDirectoryConfig) -> Connection:
    """Open and bind an LDAP connection. Raises `ldap3.core.exceptions.LDAPException`
    (or a subclass) on failure — callers translate that into a check/read error."""
    server = build_server(config)
    conn = Connection(
        server,
        user=config.ad_bind_dn,
        password=config.ad_bind_password,
        auto_bind=True,  # bind immediately; raises LDAPBindError on bad creds
        raise_exceptions=True,
        receive_timeout=60,
    )
    return conn


def guid_to_str(raw: Any) -> str | None:
    """Convert an AD `objectGUID` raw value to the canonical GUID string.

    AD stores objectGUID as a 16-byte little-endian blob. `uuid.UUID(bytes_le=...)`
    yields the same GUID string that Microsoft tooling (ADUC, PowerShell) displays.
    """
    if raw is None:
        return None
    if isinstance(raw, str):
        # ldap3 sometimes pre-formats it as '{xxxxxxxx-...}' — normalise.
        return raw.strip("{}").lower()
    if isinstance(raw, (bytes, bytearray)) and len(raw) == 16:
        return str(uuid.UUID(bytes_le=bytes(raw)))
    return str(raw)


def generalized_time_to_iso(value: Any) -> str | None:
    """Convert AD Generalized-Time (`YYYYMMDDHHMMSS.0Z`) to ISO-8601.

    ldap3 often parses these into `datetime` objects already; handle both that and
    the raw string form. Returns None on anything unparseable (downstream dbt uses
    parseDateTimeBestEffortOrNull, so a passthrough string is also acceptable, but
    ISO keeps Bronze tidy)."""
    if value is None:
        return None
    if isinstance(value, datetime):
        return value.astimezone(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    text = value.decode() if isinstance(value, (bytes, bytearray)) else str(value)
    text = text.strip()
    if len(text) >= 14 and text[:14].isdigit():
        try:
            dt = datetime.strptime(text[:14], "%Y%m%d%H%M%S").replace(tzinfo=UTC)
            return dt.strftime("%Y-%m-%dT%H:%M:%SZ")
        except ValueError:
            return text
    return text


def account_enabled(uac: Any) -> bool | None:
    """Decode the ACCOUNTDISABLE bit of `userAccountControl`. None when absent."""
    if uac is None or uac == "":
        return None
    try:
        return not (int(uac) & _UAC_ACCOUNTDISABLE)
    except (TypeError, ValueError):
        return None
