"""Connector config, parsed once at the boundary into a frozen model."""

from collections.abc import Mapping
from typing import Annotated, Any

from pydantic import BaseModel, BeforeValidator, ConfigDict, ValidationError

DEFAULT_USER_FILTER = "(&(objectClass=user)(objectCategory=person)(!(sAMAccountName=krbtgt)))"

_LDAPS_PORT = 636
_LDAP_PORT = 389


def _tls_switch(value: Any) -> bool:
    """K8s Secrets/env vars always deliver strings (e.g. `"false"`), which are truthy
    in Python — a plain truthiness check would always select LDAPS/636 regardless of
    the operator's intent."""
    if value is None:
        return True
    if isinstance(value, str):
        normalized = value.strip().lower()
        if normalized in ("true", "1", "yes"):
            return True
        if normalized in ("false", "0", "no"):
            return False
        raise ValueError("ad_use_ssl must be a boolean-like value")
    return bool(value)


class ActiveDirectoryConfig(BaseModel):
    model_config = ConfigDict(frozen=True)

    insight_tenant_id: str
    insight_source_id: str
    ad_server_host: str
    ad_port: Annotated[int | None, BeforeValidator(lambda value: value or None)] = None
    ad_use_ssl: Annotated[bool, BeforeValidator(_tls_switch)] = True
    ad_ca_cert_path: str | None = None
    ad_bind_dn: str
    ad_bind_password: str
    ad_search_base: str
    ad_user_filter: str = DEFAULT_USER_FILTER
    ad_page_size: int = 500

    @property
    def port(self) -> int:
        if self.ad_port:
            return self.ad_port
        return _LDAPS_PORT if self.ad_use_ssl else _LDAP_PORT


def describe_errors(error: ValidationError) -> str:
    """Field path and error type only: the input can be a password."""
    return "; ".join(
        f"{'.'.join(str(part) for part in item['loc']) or 'config'}: {item['type']}"
        for item in error.errors(include_input=False)
    )


def parse_config(config: Mapping[str, Any]) -> ActiveDirectoryConfig:
    try:
        return ActiveDirectoryConfig.model_validate(dict(config))
    except ValidationError as exc:
        raise ValueError(f"Invalid config: {describe_errors(exc)}") from None
