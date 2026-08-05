import json
from copy import deepcopy
from functools import lru_cache
from pathlib import Path
from typing import Any, FrozenSet, Mapping, MutableMapping

from source_salesforce.envelope import inject_envelope_properties

# Not ``schemas/`` — src/ingestion/.gitignore reserves that name for generated,
# regenerable catalogs. These files are hand-maintained source of truth.
_SCHEMA_DIR = Path(__file__).parent / "stream_schemas"
_SCHEMA_SUFFIX = ".schema.json"


class UnknownStreamSchemaError(Exception):
    pass


@lru_cache(maxsize=None)
def available_stream_names() -> FrozenSet[str]:
    return frozenset(p.name[: -len(_SCHEMA_SUFFIX)] for p in _SCHEMA_DIR.glob(f"*{_SCHEMA_SUFFIX}"))


@lru_cache(maxsize=None)
def _load(stream_name: str) -> Mapping[str, Any]:
    path = _SCHEMA_DIR / f"{stream_name}{_SCHEMA_SUFFIX}"
    if not path.is_file():
        raise UnknownStreamSchemaError(
            f"No static schema for stream '{stream_name}'; expected {path.name}"
        )
    schema: Mapping[str, Any] = json.loads(path.read_text())
    return schema


def declared_field_names(stream_name: str) -> FrozenSet[str]:
    """Salesforce fields that become Bronze columns, envelope fields excluded."""
    return frozenset(_load(stream_name)["properties"])


def stream_schema(stream_name: str) -> MutableMapping[str, Any]:
    # Deep copy: the CDK hands advertised schemas to callers that may mutate them.
    schema = deepcopy(dict(_load(stream_name)))
    return inject_envelope_properties(schema)
