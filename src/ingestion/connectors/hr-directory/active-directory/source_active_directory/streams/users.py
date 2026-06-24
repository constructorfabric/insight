"""Active Directory `users` stream.

Full-refresh LDAP paged search over the configured search base. Emits one Bronze
record per user object, with field names deliberately aligned to the `ms-entra`
connector's Bronze schema so the two directory connectors share an (almost) identical
dbt surface:

    AD (LDAP)                 ms-entra (Graph)            Bronze column
    ---------------------     ------------------------    -----------------------
    objectGUID                id (oid)                    id
    sAMAccountName            onPremisesSamAccountName    sAMAccountName
    userPrincipalName         userPrincipalName           userPrincipalName
    mail                      mail                         mail
    proxyAddresses            proxyAddresses               proxyAddresses
    displayName               displayName                  displayName
    givenName                 givenName                    givenName
    sn                        surname                      surname
    employeeID                employeeId                   employeeId
    department                department                   department
    title                     jobTitle                     jobTitle
    userAccountControl→bool   accountEnabled               accountEnabled
    whenCreated               createdDateTime              whenCreated
"""

import logging
from typing import Any, Iterable, List, Mapping, MutableMapping, Optional

from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams import Stream

from source_active_directory import ldap_client

logger = logging.getLogger("airbyte")


class ActiveDirectoryUsersStream(Stream):
    """Full-refresh roster of AD user accounts."""

    name = "users"
    primary_key = "unique_key"

    def __init__(self, config: Mapping[str, Any], tenant_id: str, source_id: str):
        self._config = config
        self._tenant_id = tenant_id
        self._source_id = source_id

    # ------------------------------------------------------------------ helpers

    @staticmethod
    def _scalar(attrs: Mapping[str, Any], key: str) -> Optional[Any]:
        """Read a single-valued attribute, tolerating ldap3 returning a 1-element list."""
        val = attrs.get(key)
        if isinstance(val, list):
            return val[0] if val else None
        if val == "":
            return None
        return val

    @staticmethod
    def _multi(attrs: Mapping[str, Any], key: str) -> Optional[List[str]]:
        """Read a multi-valued attribute as a list of strings (or None when empty)."""
        val = attrs.get(key)
        if val is None or val == "":
            return None
        if isinstance(val, list):
            out = [str(v) for v in val if v not in (None, "")]
            return out or None
        return [str(val)]

    def _make_unique_key(self, object_guid: str) -> str:
        """Composite key per Connector Spec §4.6: `{tenant}-{source}-{natural_key}`."""
        return f"{self._tenant_id}-{self._source_id}-{object_guid}"

    def _to_record(self, entry: Mapping[str, Any]) -> Optional[Mapping[str, Any]]:
        attrs = entry.get("attributes") or {}
        raw = entry.get("raw_attributes") or {}

        # objectGUID is binary — read from raw_attributes and convert.
        guid_raw = raw.get("objectGUID")
        if isinstance(guid_raw, list):
            guid_raw = guid_raw[0] if guid_raw else None
        object_guid = ldap_client.guid_to_str(guid_raw) or ldap_client.guid_to_str(
            self._scalar(attrs, "objectGUID")
        )
        if not object_guid:
            logger.warning("Skipping AD entry without objectGUID: dn=%s", entry.get("dn"))
            return None

        enabled = ldap_client.account_enabled(self._scalar(attrs, "userAccountControl"))

        record: MutableMapping[str, Any] = {
            "id": object_guid,
            "sAMAccountName": self._scalar(attrs, "sAMAccountName"),
            "userPrincipalName": self._scalar(attrs, "userPrincipalName"),
            "mail": self._scalar(attrs, "mail"),
            "proxyAddresses": self._multi(attrs, "proxyAddresses"),
            "displayName": self._scalar(attrs, "displayName"),
            "givenName": self._scalar(attrs, "givenName"),
            "surname": self._scalar(attrs, "sn"),
            "employeeId": self._scalar(attrs, "employeeID"),
            "department": self._scalar(attrs, "department"),
            "jobTitle": self._scalar(attrs, "title"),
            "accountEnabled": enabled,
            # Normalised HR status — drives the Identity Manager active-interval
            # logic (value_type='status'; 'Terminated' is in its inactive set).
            # Disabled AD accounts are retained for years, so without this every
            # long-offboarded user would be treated as always-active in org_chart.
            "status": ("Active" if enabled else "Terminated") if enabled is not None else None,
            "distinguishedName": self._scalar(attrs, "distinguishedName") or entry.get("dn"),
            "managerDn": self._scalar(attrs, "manager"),
            "whenCreated": ldap_client.generalized_time_to_iso(self._scalar(attrs, "whenCreated")),
            "whenChanged": ldap_client.generalized_time_to_iso(self._scalar(attrs, "whenChanged")),
        }

        # Framework fields (Connector Spec principle: mandatory tenant_id/source_id/unique_key).
        record["tenant_id"] = self._tenant_id
        record["source_id"] = self._source_id
        record["unique_key"] = self._make_unique_key(object_guid)
        return record

    # ------------------------------------------------------------------ CDK API

    def read_records(
        self,
        sync_mode: SyncMode,
        cursor_field: Optional[List[str]] = None,
        stream_slice: Optional[Mapping[str, Any]] = None,
        stream_state: Optional[Mapping[str, Any]] = None,
    ) -> Iterable[Mapping[str, Any]]:
        from ldap3.core.exceptions import LDAPException

        conn = ldap_client.connect(self._config)
        try:
            entries = conn.extend.standard.paged_search(
                search_base=self._config["ad_search_base"],
                search_filter=self._config.get(
                    "ad_user_filter",
                    "(&(objectClass=user)(objectCategory=person)(!(sAMAccountName=krbtgt)))",
                ),
                search_scope="SUBTREE",
                attributes=ldap_client.USER_ATTRIBUTES,
                paged_size=int(self._config.get("ad_page_size", 500)),
                generator=True,
            )
            count = 0
            for entry in entries:
                if entry.get("type") != "searchResEntry":
                    continue  # referrals / other control messages
                record = self._to_record(entry)
                if record is not None:
                    count += 1
                    yield record
            logger.info("AD users stream emitted %d records", count)
        except LDAPException:
            logger.exception("LDAP paged search failed")
            raise
        finally:
            try:
                conn.unbind()
            except Exception:  # noqa: BLE001 — best-effort cleanup
                pass

    def get_json_schema(self) -> Mapping[str, Any]:
        """JSON Schema for `bronze_active_directory.users`. additionalProperties=true so
        new attributes surface as nullable columns without a schema migration. Field
        set and nullability mirror `ms-entra`'s `users` stream."""
        nullable_str = {"type": ["null", "string"]}
        return {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": True,
            "required": ["unique_key", "id"],
            "properties": {
                "id": {"type": "string", "description": "objectGUID — stable AD GUID; identity join key."},
                "sAMAccountName": nullable_str,
                "userPrincipalName": nullable_str,
                "mail": nullable_str,
                "proxyAddresses": {"type": ["null", "array"], "items": nullable_str},
                "displayName": nullable_str,
                "givenName": nullable_str,
                "surname": nullable_str,
                "employeeId": nullable_str,
                "department": nullable_str,
                "jobTitle": nullable_str,
                "accountEnabled": {"type": ["null", "boolean"]},
                "status": nullable_str,
                "distinguishedName": nullable_str,
                "managerDn": nullable_str,
                "whenCreated": nullable_str,
                "whenChanged": nullable_str,
                "tenant_id": {"type": "string"},
                "source_id": {"type": "string"},
                "unique_key": {"type": "string"},
            },
        }
