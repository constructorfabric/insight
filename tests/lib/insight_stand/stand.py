"""Where the stand is — base-URL resolution for a host-side or in-network run.

This is deliberately NOT read from the manifest. Per the phase-4 stand-verbs
contract, the manifest's `service_urls` are compose-network-internal addresses
(`http://gateway:8080`) that only resolve from another container; a host-side
pytest run reaches the same gateway through a published port. Asking the
manifest for a host URL would give a value that silently fails to connect.

Resolution order — each step is SOURCED, none is a hardcoded stand address:

1. `INSIGHT_STAND_BASE_URL`, used verbatim. This is how a runner that joins the
   compose network points the suite at `Manifest.internal_gateway_url`
   (phase 7's browser container), and how any non-compose stand is targeted.
2. The stand's own env file — `.env.compose.test-stand` if present (written by
   `./dev-compose.sh test-stand up`), else `.env.compose`. `GATEWAY_PORT` from
   that file is combined with the host `dev-compose.sh` publishes on,
   `localhost`, giving `http://localhost:<GATEWAY_PORT>`.
3. Nothing else. If neither is available the suite raises
   `StandConnectionError` and stops — it never assumes a port.

Only the gateway is ever addressed. Backend services publish their own host
ports (analytics 8081, identity 8082) but reaching them directly would bypass
the edge that terminates the session, so this module exposes no way to do it.
"""

from __future__ import annotations

import os
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Final

from .errors import StandConnectionError

_REPO_ROOT: Final[Path] = Path(__file__).resolve().parents[3]

BASE_URL_ENV: Final[str] = "INSIGHT_STAND_BASE_URL"
ENV_FILE_ENV: Final[str] = "INSIGHT_STAND_ENV_FILE"

# Most specific first: the test stand's generated file beats a developer's own.
CANDIDATE_ENV_FILES: Final[tuple[str, ...]] = (".env.compose.test-stand", ".env.compose")

# The host `dev-compose.sh` publishes its port mappings on — see
# `report_service_urls` in dev-compose.sh, which prints exactly this host.
PUBLISHED_HOST: Final[str] = "localhost"

GATEWAY_PORT_KEY: Final[str] = "GATEWAY_PORT"

#: Where the run's artefacts land — the coverage ledger and the operation
#: catalogue. Overridable because the two runners disagree about where the repo
#: root is: from a checkout it is the directory holding `tests/`, while the
#: ui-tests image places the suite at `/tests` with nothing above it, so the
#: same walk-upwards lands on `/`.
#:
#: That used to be silent. The image ran as root, `/.artifacts` was created
#: without complaint, and the ledger was written somewhere nothing collected it
#: from — a wrong answer that looked like no answer. Dropping root turned it
#: into a PermissionError, which is how it was found.
ARTIFACT_DIR_ENV: Final[str] = "INSIGHT_STAND_ARTIFACT_DIR"


def artifact_dir(fallback: Path) -> Path:
    """`$INSIGHT_STAND_ARTIFACT_DIR`, else the caller's repo-relative guess."""
    import os

    override = (os.environ.get(ARTIFACT_DIR_ENV) or "").strip()
    return Path(override) if override else fallback


@dataclass(frozen=True)
class StandEndpoint:
    """A resolved stand address plus where the value came from.

    `source` is carried so a connection failure can say which file or variable
    produced the URL that did not answer, instead of only that it did not.
    """

    base_url: str
    source: str

    def __str__(self) -> str:
        return f"{self.base_url} (from {self.source})"


def parse_env_file(path: Path) -> Mapping[str, str]:
    """Parse a `KEY=VALUE` compose env file.

    Handles what `dev-compose.sh` actually writes: comments, blank lines,
    optional `export ` prefixes and optionally quoted values. Deliberately not
    a shell — no interpolation, no command substitution.
    """
    values: dict[str, str] = {}
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        raise StandConnectionError(f"cannot read stand env file {path}: {exc}") from exc

    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        if stripped.startswith("export "):
            stripped = stripped[len("export ") :].lstrip()
        key, _, value = stripped.partition("=")
        key = key.strip()
        if not key:
            continue
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        values[key] = value
    return values


def _candidate_env_files(repo_root: Path, environ: Mapping[str, str]) -> list[Path]:
    override = (environ.get(ENV_FILE_ENV) or "").strip()
    if override:
        return [Path(override)]
    return [repo_root / name for name in CANDIDATE_ENV_FILES]


def resolve_endpoint(
    *,
    repo_root: Path | None = None,
    environ: Mapping[str, str] | None = None,
) -> StandEndpoint:
    """Resolve the stand's base URL, or raise `StandConnectionError`."""
    env = os.environ if environ is None else environ
    root = _REPO_ROOT if repo_root is None else repo_root

    explicit = (env.get(BASE_URL_ENV) or "").strip()
    if explicit:
        return StandEndpoint(base_url=explicit.rstrip("/"), source=f"${BASE_URL_ENV}")

    tried: list[str] = []
    for candidate in _candidate_env_files(root, env):
        if not candidate.is_file():
            tried.append(f"{candidate} (absent)")
            continue
        port = parse_env_file(candidate).get(GATEWAY_PORT_KEY, "").strip()
        if not port:
            tried.append(f"{candidate} (no {GATEWAY_PORT_KEY})")
            continue
        return StandEndpoint(
            base_url=f"http://{PUBLISHED_HOST}:{port}",
            source=f"{GATEWAY_PORT_KEY}={port} in {candidate.name}",
        )

    raise StandConnectionError(
        "cannot resolve the stand's base URL — refusing to assume one.\n"
        "  tried: " + "; ".join(tried) + "\n"
        f"  Bring a stand up (./dev-compose.sh test-stand up), which writes "
        f"{CANDIDATE_ENV_FILES[0]},\n"
        f"  or set {BASE_URL_ENV} explicitly (an in-network runner should pass "
        "the manifest's service_urls.gateway)."
    )


def resolve_base_url(
    *,
    repo_root: Path | None = None,
    environ: Mapping[str, str] | None = None,
) -> str:
    """`resolve_endpoint().base_url`, for callers that want only the URL."""
    return resolve_endpoint(repo_root=repo_root, environ=environ).base_url


__all__: Sequence[str] = (
    "BASE_URL_ENV",
    "CANDIDATE_ENV_FILES",
    "ENV_FILE_ENV",
    "GATEWAY_PORT_KEY",
    "PUBLISHED_HOST",
    "StandEndpoint",
    "parse_env_file",
    "resolve_base_url",
    "resolve_endpoint",
)
