"""Typed model of the seed manifest.

The manifest is the stand's self-description: what was seeded, who exists, what
the stand can do. `deploy/seed/seed.py` writes it to one place:

    deploy/seed/manifest.json

A reader may be told to look elsewhere — `$INSIGHT_STAND_MANIFEST`, or pytest's
`--stand-manifest` — because a runner that does not share the repo's filesystem
has to get the file from somewhere. What is NOT configurable is having one: a
run with no manifest aborts rather than falling back to a default, since a
defaulted manifest turns "this stand was never seeded" into a green suite.

The field shape mirrored here comes from the phase-3 schema document
(`out/manifest-schema.md`) field for field — nothing is invented, guessed or
renamed. Parsing is strict for the same reason: a missing or
mistyped field raises `ManifestError` rather than defaulting.

Two similarly-named things are deliberately kept apart:

* `Manifest.seeded` mirrors the manifest's own `seeded[]` — which *seed steps*
  ran (`identity`, `silver`).
* `Manifest.seeded_names` is the `fixtures{}` catalog's KEYS — the stable names
  a test declares with `@pytest.mark.requires_seed(...)`.
"""

from __future__ import annotations

import json
import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Final

from .errors import ManifestError

# The location the seed writes to, resolved from this file:
#   tests/lib/insight_stand/manifest.py -> ../../../
_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[3]
MANIFEST_PATH: Final[Path] = _REPO_ROOT / "deploy" / "seed" / "manifest.json"

# Point a runner at a manifest it can actually reach. Named the same way as
# $INSIGHT_STAND_BASE_URL and $INSIGHT_STAND_ENV_FILE in stand.py.
MANIFEST_PATH_ENV: Final[str] = "INSIGHT_STAND_MANIFEST"


def default_manifest_path(environ: Mapping[str, str] | None = None) -> Path:
    """Where to read the manifest from when the caller does not say.

    `$INSIGHT_STAND_MANIFEST` first, then the path the seed writes.

    The override earns its place in containers. `MANIFEST_PATH` is derived from
    THIS FILE's location, so in an image that holds the tree at `/tests` it
    resolves to `/deploy/seed/manifest.json` — and a bind mount then has to
    reproduce that arithmetic exactly, or the suite reports an unseeded stand.
    Naming the file is the honest alternative to guessing where `parents[3]`
    landed.
    """
    env = os.environ if environ is None else environ
    override = (env.get(MANIFEST_PATH_ENV) or "").strip()
    return Path(override) if override else MANIFEST_PATH


# The schema revision this model was written against. A stand emitting a
# different version is a hard error, not something to parse optimistically.
SUPPORTED_MANIFEST_VERSION: Final[int] = 1

# Capability fields that answer yes/no, and so can back a `requires_*` marker.
# `idp` is excluded on purpose: it carries a value, not a yes/no.
BOOLEAN_CAPABILITIES: Final[frozenset[str]] = frozenset({"ingestion", "service_principals"})


def _require(doc: Mapping[str, Any], key: str, kind: type | tuple[type, ...], where: str) -> Any:
    if key not in doc:
        raise ManifestError(f"{where}: required field {key!r} is missing")
    value = doc[key]
    if not isinstance(value, kind):
        names = kind.__name__ if isinstance(kind, type) else "/".join(k.__name__ for k in kind)
        raise ManifestError(f"{where}: field {key!r} must be {names}, got {type(value).__name__}")
    return value


def _optional_str(doc: Mapping[str, Any], key: str, where: str) -> str | None:
    """A field the schema declares as nullable (only `team`, for the CEO)."""
    if key not in doc:
        raise ManifestError(f"{where}: required field {key!r} is missing")
    value = doc[key]
    if value is None or isinstance(value, str):
        return value
    raise ManifestError(f"{where}: field {key!r} must be str or null, got {type(value).__name__}")


@dataclass(frozen=True)
class Realm:
    """`realm` — mirrors deploy/compose/keycloak/gen-realm.py's realm."""

    name: str
    issuer: str

    @classmethod
    def parse(cls, doc: Mapping[str, Any]) -> Realm:
        where = "manifest.realm"
        return cls(
            name=_require(doc, "name", str, where),
            # Empty when the stand did not tell the seed what its issuer is.
            # Never guessed — an empty issuer is information, not a defect.
            issuer=_require(doc, "issuer", str, where),
        )


@dataclass(frozen=True)
class Person:
    """One roster entry — a `personas[]` element or a `fixtures{}` value.

    The two carry the same seven fields, so one type covers both.
    """

    uuid: str
    email: str
    display_name: str
    team: str | None
    org_unit: str
    role: str
    realm_roles: tuple[str, ...]

    @classmethod
    def parse(cls, doc: Mapping[str, Any], where: str) -> Person:
        roles = _require(doc, "realm_roles", list, where)
        if not all(isinstance(role, str) for role in roles):
            raise ManifestError(f"{where}: every 'realm_roles' entry must be a string")
        return cls(
            uuid=_require(doc, "uuid", str, where),
            email=_require(doc, "email", str, where),
            display_name=_require(doc, "display_name", str, where),
            team=_optional_str(doc, "team", where),
            org_unit=_require(doc, "org_unit", str, where),
            role=_require(doc, "role", str, where),
            realm_roles=tuple(roles),
        )


@dataclass(frozen=True)
class Capabilities:
    """`capabilities` — what this stand can actually do.

    `ingestion` is false on compose: the seed writes silver/gold directly and
    no connector ever runs, so the ingestion path is not exercised. Tests that
    need it carry `@pytest.mark.requires_ingestion` and are skipped, with a
    reason, rather than failing.
    """

    ingestion: bool
    service_principals: bool
    idp: str

    @classmethod
    def parse(cls, doc: Mapping[str, Any]) -> Capabilities:
        where = "manifest.capabilities"
        return cls(
            ingestion=_require(doc, "ingestion", bool, where),
            # Optional: a manifest written before this field existed reports
            # False, which skips the S2S tests rather than failing to parse.
            service_principals=bool(doc.get("service_principals", False)),
            idp=_require(doc, "idp", str, where),
        )

    def has(self, name: str) -> bool:
        """Answer a yes/no capability, or raise if the name is not one.

        Raising beats returning False for an unknown name: a typo in a
        capability marker's table would otherwise skip every test carrying it,
        with a reason that reads perfectly plausibly.

        `idp` is deliberately not answerable here — it is a VALUE
        (`keycloak` | `fakeidp`), not a yes/no, so comparing it is the caller's
        job.
        """
        if name not in BOOLEAN_CAPABILITIES:
            known = ", ".join(sorted(BOOLEAN_CAPABILITIES))
            raise ValueError(f"{name!r} is not a boolean capability; known: {known}")
        return bool(getattr(self, name))


@dataclass(frozen=True)
class GoldenMetric:
    """One `golden_metrics[]` entry: an exact, hand-sourced expectation.

    Currently always absent — `golden_metrics` is `[]` on every stand, by
    design. See `deploy/seed/golden_metrics.py` for why, and read
    `Manifest.golden_metrics_note` to tell "none measured yet" apart from
    "measured and genuinely zero".

    No test under `stand/` reads this yet: the harness that did is being
    migrated separately. It is parsed regardless because `Manifest` models the
    document the seed writes — dropping the field would mean a manifest missing
    it stopped being an error, which is the opposite of what a reader is for.
    """

    metric_key: str
    expected: float | int | str
    scope: str
    window: str
    derivation: str

    @classmethod
    def parse(cls, doc: Mapping[str, Any], where: str) -> GoldenMetric:
        return cls(
            metric_key=_require(doc, "metric_key", str, where),
            # The phase-3 schema declares this as number|string; do not narrow.
            expected=_require(doc, "expected", (int, float, str), where),
            scope=_require(doc, "scope", str, where),
            window=_require(doc, "window", str, where),
            derivation=_require(doc, "derivation", str, where),
        )


@dataclass(frozen=True)
class Tenants:
    """Every tenant the stand seeded, named.

    `other` is a tenant holding exactly one person and nothing else. It exists
    because cross-tenant refusal is the one authorization property a
    single-tenant stand cannot show at all: with one tenant there is no caller
    who should be refused, so a service ignoring `tenant_id` entirely would pass
    every test.

    It is optional. A stand seeded before this field existed reports `other` as
    None, and a test that needs it declares `requires_seed("other_tenant_lead")`
    rather than reading this — the fixture catalogue is where a missing person
    is reported by name.
    """

    default: str
    other: str | None

    @classmethod
    def parse(cls, doc: Mapping[str, Any], fallback: str, where: str) -> Tenants:
        other = doc.get("other")
        if other is not None and not isinstance(other, str):
            raise ManifestError(f"{where}.other: must be a string, got {type(other).__name__}")
        return cls(default=str(doc.get("default") or fallback), other=other)


@dataclass(frozen=True)
class DefinitionOverride:
    """The product definition the seed re-labelled for this tenant."""

    metric_key: str
    label: str

    @classmethod
    def parse(cls, doc: Mapping[str, Any], where: str) -> DefinitionOverride:
        return cls(
            metric_key=_require(doc, "metric_key", str, where),
            label=_require(doc, "label", str, where),
        )


@dataclass(frozen=True)
class Catalogue:
    """Rows no endpoint creates, seeded by `deploy/seed/analytics.py`.

    It is optional and a test must treat absence as "cannot assert" rather than
    as failure: a stand seeded without the `analytics` step is a real state.
    `tests/stand/conftest.py`'s `requires_catalogue` marker is how a test
    declares it needs it, so the skip carries a reason instead of the test
    quietly asserting against an empty universe.
    """

    definition_override: DefinitionOverride | None

    @classmethod
    def parse(cls, doc: Mapping[str, Any], where: str) -> Catalogue:
        override = doc.get("definition_override")
        return cls(
            definition_override=(
                None
                if override is None
                else DefinitionOverride.parse(
                    _as_mapping(override, f"{where}.definition_override"),
                    f"{where}.definition_override",
                )
            ),
        )


@dataclass(frozen=True)
class Manifest:
    """The whole document, field for field."""

    manifest_version: int
    tenant: str
    tenants: Tenants
    realm: Realm
    personas: tuple[Person, ...]
    service_urls: Mapping[str, str]
    fixtures: Mapping[str, Person]
    catalogue: Catalogue
    golden_metrics: tuple[GoldenMetric, ...]
    golden_metrics_note: str
    capabilities: Capabilities
    seed_revision: str
    data_window: str
    anchor_date: str
    seeded: tuple[str, ...]
    source_path: Path

    # -- loading ----------------------------------------------------------

    @classmethod
    def load(cls, path: Path | None = None) -> Manifest:
        """Read and parse the manifest, or raise `ManifestError`.

        Most specific first: an explicit `path`, then `$INSIGHT_STAND_MANIFEST`,
        then the path the seed writes. Whichever wins is carried on the parsed
        manifest as `source_path`, so every message about the stand can name the
        document it read.
        """
        target = Path(path) if path is not None else default_manifest_path()
        try:
            raw = target.read_text(encoding="utf-8")
        except FileNotFoundError as exc:
            raise ManifestError(
                f"no seed manifest at {target} — the stand is not seeded.\n"
                "Bring it up and seed it with:  ./dev-compose.sh test-stand up"
            ) from exc
        except OSError as exc:
            raise ManifestError(f"cannot read seed manifest at {target}: {exc}") from exc

        try:
            doc = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise ManifestError(f"seed manifest at {target} is not valid JSON: {exc}") from exc
        if not isinstance(doc, dict):
            raise ManifestError(
                f"seed manifest at {target} must be a JSON object, got {type(doc).__name__}"
            )
        return cls.parse(doc, source_path=target)

    @classmethod
    def parse(cls, doc: Mapping[str, Any], *, source_path: Path) -> Manifest:
        where = "manifest"
        version = _require(doc, "manifest_version", int, where)
        if version != SUPPORTED_MANIFEST_VERSION:
            raise ManifestError(
                f"{source_path}: manifest_version {version} is not supported "
                f"(this suite understands {SUPPORTED_MANIFEST_VERSION}). "
                "Re-seed the stand, or update insight_stand.manifest."
            )

        personas_raw = _require(doc, "personas", list, where)
        personas = tuple(
            Person.parse(_as_mapping(entry, f"{where}.personas[{i}]"), f"{where}.personas[{i}]")
            for i, entry in enumerate(personas_raw)
        )

        fixtures_raw = _require(doc, "fixtures", dict, where)
        fixtures = {
            name: Person.parse(
                _as_mapping(entry, f"{where}.fixtures[{name!r}]"), f"{where}.fixtures[{name!r}]"
            )
            for name, entry in fixtures_raw.items()
        }

        urls_raw = _require(doc, "service_urls", dict, where)
        if not all(isinstance(k, str) and isinstance(v, str) for k, v in urls_raw.items()):
            raise ManifestError(f"{where}.service_urls: every key and value must be a string")

        golden_raw = _require(doc, "golden_metrics", list, where)
        golden = tuple(
            GoldenMetric.parse(
                _as_mapping(entry, f"{where}.golden_metrics[{i}]"),
                f"{where}.golden_metrics[{i}]",
            )
            for i, entry in enumerate(golden_raw)
        )

        # Optional, unlike every field above: a manifest written before the
        # analytics seed step existed is still readable, and reports an empty
        # catalogue rather than failing to parse.
        catalogue = Catalogue.parse(
            _as_mapping(doc.get("catalogue") or {}, f"{where}.catalogue"),
            f"{where}.catalogue",
        )

        seeded_raw = _require(doc, "seeded", list, where)
        if not all(isinstance(step, str) for step in seeded_raw):
            raise ManifestError(f"{where}.seeded: every entry must be a string")

        tenant = _require(doc, "tenant", str, where)
        return cls(
            manifest_version=version,
            tenant=tenant,
            # Optional, like `catalogue`: a manifest written before the second
            # tenant existed still parses, and reports `other` as absent.
            tenants=Tenants.parse(
                _as_mapping(doc.get("tenants") or {}, f"{where}.tenants"),
                tenant,
                f"{where}.tenants",
            ),
            realm=Realm.parse(_as_mapping(_require(doc, "realm", dict, where), f"{where}.realm")),
            personas=personas,
            service_urls=dict(urls_raw),
            fixtures=fixtures,
            catalogue=catalogue,
            golden_metrics=golden,
            golden_metrics_note=_require(doc, "golden_metrics_note", str, where),
            capabilities=Capabilities.parse(
                _as_mapping(_require(doc, "capabilities", dict, where), f"{where}.capabilities")
            ),
            seed_revision=_require(doc, "seed_revision", str, where),
            data_window=_require(doc, "data_window", str, where),
            anchor_date=_require(doc, "anchor_date", str, where),
            seeded=tuple(seeded_raw),
            source_path=source_path,
        )

    # -- accessors the suite is built on ----------------------------------

    @property
    def seeded_names(self) -> frozenset[str]:
        """Names `@pytest.mark.requires_seed(...)` may declare.

        The `fixtures{}` catalog's keys — stable, role-shaped contracts
        (`dev_lead`, `ceo`, `development_ic`, ...) that survive roster churn.
        NOT the same as `Manifest.seeded`, which lists the seed STEPS that ran.
        """
        return frozenset(self.fixtures)

    def fixture(self, name: str) -> Person:
        """Resolve one catalog entry, or fail naming what is available."""
        try:
            return self.fixtures[name]
        except KeyError:
            available = ", ".join(sorted(self.fixtures)) or "<none>"
            raise ManifestError(
                f"no fixture {name!r} in {self.source_path}; available: {available}"
            ) from None

    def has_capability(self, name: str) -> bool:
        """Capability lookup for the `requires_*` capability markers."""
        return self.capabilities.has(name)

    @property
    def persona_emails(self) -> frozenset[str]:
        return frozenset(person.email for person in self.personas)

    @property
    def internal_gateway_url(self) -> str:
        """The gateway address as seen from ANOTHER CONTAINER on the compose
        network — never a host port.

        Phase 7's browser runner joins that network and should pass this value
        as `INSIGHT_STAND_BASE_URL`. A host-side pytest run must NOT use it;
        see `insight_stand.stand.resolve_base_url`.
        """
        try:
            return self.service_urls["gateway"]
        except KeyError:
            raise ManifestError(
                f"{self.source_path}: service_urls has no 'gateway' entry"
            ) from None


def _as_mapping(value: Any, where: str) -> Mapping[str, Any]:
    if not isinstance(value, Mapping):
        raise ManifestError(f"{where}: must be an object, got {type(value).__name__}")
    return value


def load_manifest(path: Path | None = None) -> Manifest:
    """Module-level convenience wrapper over `Manifest.load`."""
    return Manifest.load(path)


__all__: Sequence[str] = (
    "BOOLEAN_CAPABILITIES",
    "MANIFEST_PATH",
    "MANIFEST_PATH_ENV",
    "SUPPORTED_MANIFEST_VERSION",
    "Capabilities",
    "Catalogue",
    "DefinitionOverride",
    "GoldenMetric",
    "Manifest",
    "Person",
    "Realm",
    "Tenants",
    "default_manifest_path",
    "load_manifest",
)
