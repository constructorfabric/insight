"""Generate (or verify) the generated half of `stand/api/schemas/` from the committed specs.

    uv run --project tests --frozen python tests/generate_schemas.py
    uv run --project tests --frozen python tests/generate_schemas.py --check

One entry per backend service the stand talks to. A service is GENERATED only
when its published document both describes response bodies and can be trusted to
describe them correctly; the rest are declared here with the reason they cannot
be, and `--check` re-derives that reason rather than trusting the prose. So the
day a service becomes generatable — the analytics document is proof that it
happens — the check says "generate it" instead of the gap sitting unnoticed
behind a comment.

The two reasons are not interchangeable and each has its own re-derivation.
`Bodyless` is about SHAPE: there is nothing a generator could turn into a model,
so publishing one body invalidates it. `Untrusted` is about PROVENANCE: the
document describes bodies a service does not implement, and no amount of them
makes generating from it right — only the real document arriving does.

Generating is sound for analytics because its document is itself generated from
the handlers' own types (`cargo run -p analytics -- openapi`) and drift-gated in
CI by `.github/workflows/openapi-specs.yml`. There is no second source of truth
— the models describe the very structs that serialize the wire.

The output is COMMITTED. A test run must never need the generator, which is a
dev-only dependency and absent from the ui-tests image; `--check` is what keeps
the committed copy honest, the same arrangement `src/ingestion/tools/seed/render_profile.py`
uses for PROFILE.md.

Deliberately NOT a pytest case. `tests/stand/` exists to assert things about a
deployed stand, and whether a generated file in this repository is current is a
statement about the repository. The same reasoning retired
`test_credentials_contract.py`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

_TESTS = Path(__file__).resolve().parent
_REPO_ROOT = _TESTS.parent
_SPECS = _REPO_ROOT / "docs" / "components" / "backend"
_SCHEMAS = _TESTS / "stand" / "api" / "schemas"

# `--disable-timestamp` matters: without it every run writes a new header and
# `--check` can never pass. `--extra-fields forbid` is the generated half's
# strictness — these models are regenerated in the same change that adds a
# field, so an undeclared one is real drift rather than a benign addition.
CODEGEN_ARGS = (
    "--input-file-type",
    "openapi",
    "--output-model-type",
    "pydantic_v2.BaseModel",
    "--target-python-version",
    "3.13",
    "--use-standard-collections",
    "--use-union-operator",
    "--use-schema-description",
    "--field-constraints",
    "--extra-fields",
    "forbid",
    "--disable-timestamp",
    "--formatters",
    "ruff-format",
    "--formatters",
    "ruff-check",
)

ANALYTICS_HEADER = '''"""Analytics response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/analytics/openapi.json`, which is itself
generated from the analytics handlers' own types and drift-gated in CI. These
models therefore describe the structs that serialize the wire, and a validation
failure means the service and its published contract disagree — a contract test,
unlike the hand-written models in `identity.py`.

`extra="forbid"` throughout: an undeclared field is drift, and the models are
regenerated in the same change that would add one.

BODIES ONLY. This document's per-operation status-code lists are stamped
uniformly by `.standard_errors` and describe nothing (#1669), so no test takes a
status code from here — every one is asserted from observed behaviour.

ONE substitution is applied after generation: every `AwareDatetime` becomes
`UnzonedDatetime`, because the contract declares `format: date-time` while the
service serialises timestamps with no offset. See `common.UnzonedDatetime`.
"""

'''


AUTHENTICATOR_HEADER = '''"""Authenticator response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/authenticator/openapi.json`, generated offline
by `cargo run -p authenticator -- openapi` and drift-gated in CI beside the
analytics document — same provenance, same guarantee that these models describe
the structs that serialize the wire.

SMALL ON PURPOSE, and it is the document that is small: every success body on
`/auth/*` is declared as a bare `type: object` with no properties, because those
handlers answer untyped JSON. So what the contract describes today is the error
envelope, and that is what this module holds. A handler that gains a typed
response appears here on the next regeneration — which is the reason this file
exists rather than the service being listed as having nothing to generate.

Two consequences worth knowing while it stays this small:

* The envelope here is the CONTRACT's, generated and drift-gated; the one in
  `common.ProblemDocument` is hand-written from observed bodies and is what the
  suite asserts with today. They agree field for field, with one difference: the
  contract REQUIRES `context`, where the hand-written model defaults it to `{}`.
* No status code comes from this document either — the same `.standard_errors`
  stamping applies (#1669).
"""

'''


IDENTITY_HEADER = '''"""Identity Resolution response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/identity-resolution/openapi.json`, generated
offline by `cargo run -p identity-resolution -- openapi` and drift-gated in CI
beside the analytics and authenticator documents. Until that subcommand existed
this module was hand-written from the Rust DTOs, because the committed contract
was still the retired .NET one; these models now describe the structs that
serialize the wire, so a validation failure is a contract disagreement rather
than a stale transcription.

The names are the contract's, not the suite's: `SubchartResponse` where the
hand-written module said `Subchart`. `stand/api/schemas/__init__.py` re-exports
them under the names the tests already use, so the rename stops at this package.

BODIES ONLY — no status code comes from this document. Its per-operation lists
are stamped uniformly by `.standard_errors` and describe nothing (#1669), the
same limitation the analytics and authenticator documents carry.
"""

'''


@dataclass(frozen=True)
class Generated:
    """A service whose document describes bodies: models are generated and committed."""

    name: str
    spec: Path
    output: Path
    header: str
    #: Apply the pinned `AwareDatetime` → `UnzonedDatetime` deviation (see
    #: `common.UnzonedDatetime`). Per-service because it is a claim about how
    #: THAT service serialises timestamps, not a house style.
    unzoned_datetime: bool = False


@dataclass(frozen=True)
class Bodyless:
    """A service whose document describes no body a generator could model.

    `--check` re-derives the reason instead of trusting it: the entry fails the
    moment the document starts describing a body, which is the signal to promote
    it to `Generated`.
    """

    name: str
    spec: Path
    reason: str


@dataclass(frozen=True)
class Untrusted:
    """A service whose document describes bodies that must not be modelled anyway.

    Provenance is the disqualifier, so body count says nothing about it: a
    contract the service does not implement records its errors as fact whether it
    describes one body or fifty. `still_the_wrong_document` is what `--check`
    re-derives instead — a property of the committed file that stops holding when
    the real document replaces it.
    """

    name: str
    spec: Path
    reason: str
    still_the_wrong_document: Callable[[Path], str | None]


def declares_only_200(spec_path: Path) -> str | None:
    """None while every operation in the document declares nothing but `200`.

    The retired .NET contract's signature, and a property no Rust service in this
    repository can have: they register through `OperationBuilder`, whose
    `.standard_errors(openapi)` stamps 400/401/403/404/409/429/500 onto every
    route (that over-stamping is its own problem, #1669 — but it means a single
    declared error code proves the document was emitted by the Rust service).
    """
    declared: set[str] = set()
    spec: dict[str, Any] = json.loads(spec_path.read_text(encoding="utf-8"))

    for operations in (spec.get("paths") or {}).values():
        for operation in operations.values():
            declared |= {
                status for status in (operation.get("responses") or {}) if status.isdigit()
            }

    if declared <= {"200"}:
        return None

    return f"declares {sorted(declared - {'200'})}, which only the Rust service's own document does"


TARGETS: tuple[Generated | Bodyless | Untrusted, ...] = (
    Generated(
        name="analytics",
        spec=_SPECS / "analytics" / "openapi.json",
        output=_SCHEMAS / "analytics.py",
        header=ANALYTICS_HEADER,
        unzoned_datetime=True,
    ),
    Generated(
        name="authenticator",
        spec=_SPECS / "authenticator" / "openapi.json",
        output=_SCHEMAS / "authenticator.py",
        header=AUTHENTICATOR_HEADER,
    ),
    # NGINX + Lua (`access_by_lua`), no binary that could emit a document and no
    # `openapi.json` under docs/components/backend/gateway/. It exposes exactly
    # one endpoint of its own — `GET /healthz`, `text/plain` "ok" — and is
    # otherwise a proxy: `/auth/*` to the authenticator, the generated `/api/*`
    # locations to analytics and identity, `/internal/*` and unmatched `/api/*`
    # to 404, `/` to the SPA. Its route table (`deploy/compose/gateway/routes.yaml`
    # and the chart's `gateway.routes`, compiled to nginx by `tools/routegen`) is
    # the machine-readable edge contract; the 401 envelope it produces of its own
    # is `common.ProblemDocument`.
    Bodyless(
        name="gateway",
        spec=_SPECS / "gateway" / "openapi.json",
        reason="publishes no OpenAPI document (NGINX + Lua; `GET /healthz` is its only own route)",
    ),
    Generated(
        name="identity-resolution",
        spec=_SPECS / "identity-resolution" / "openapi.json",
        output=_SCHEMAS / "identity.py",
        header=IDENTITY_HEADER,
    ),
)


def modellable_bodies(spec_path: Path) -> list[str]:
    """The success responses a generator could turn into a model: those whose
    schema declares properties, an `items`, or a `$ref`. Empty for a document
    that does not exist.

    SUCCESS responses only, and reached through the responses rather than by
    listing `components.schemas`: a document may declare component schemas for
    its REQUEST bodies (the .NET identity contract declares five) while
    describing no response at all, and request models are not what this module
    generates. Error responses are excluded because every service declares the
    same `Problem`, hand-written once as `common.ProblemDocument`.
    """
    if not spec_path.is_file():
        return []

    spec: dict[str, Any] = json.loads(spec_path.read_text(encoding="utf-8"))
    found: list[str] = []

    paths: dict[str, dict[str, Any]] = spec.get("paths") or {}
    for path, operations in sorted(paths.items()):
        for method, operation in sorted(operations.items()):
            responses: dict[str, Any] = operation.get("responses") or {}
            for status, response in sorted(responses.items()):
                if not status.isdigit() or int(status) >= 300:
                    continue
                for media in (response.get("content") or {}).values():
                    schema = media.get("schema") or {}
                    if {"properties", "$ref", "items"} & set(schema):
                        found.append(f"{method.upper()} {path} {status}")

    return found


def generate(target: Generated) -> str:
    """Run the generator for `target` and return the module source, header included."""
    with tempfile.TemporaryDirectory() as tmp:
        out = Path(tmp) / target.output.name
        subprocess.run(
            ["datamodel-codegen", "--input", str(target.spec), *CODEGEN_ARGS, "--output", str(out)],
            check=True,
            capture_output=True,
            text=True,
        )
        body = out.read_text(encoding="utf-8")

    # The generator's own two-line provenance comment is replaced by the target's
    # header, which says the same thing plus what a reader needs to know about
    # trusting these models.
    lines = body.splitlines(keepends=True)
    while lines and lines[0].startswith("#"):
        lines.pop(0)
    source = "".join(lines).lstrip("\n")

    if target.unzoned_datetime:
        source = _substitute_unzoned_datetime(source)

    return target.header + source


def _substitute_unzoned_datetime(source: str) -> str:
    """The single pinned deviation — see `common.UnzonedDatetime`. Applied here
    rather than by hand because the file is regenerated: an edit would be lost,
    and this way `--check` still passes on a clean tree."""
    source = source.replace("AwareDatetime", "UnzonedDatetime")
    source = source.replace("from pydantic import UnzonedDatetime, ", "from pydantic import ")
    source = source.replace("from pydantic import UnzonedDatetime\n", "")
    # After `from __future__`, which must stay the first statement in the module.
    return source.replace(
        "from __future__ import annotations\n",
        "from __future__ import annotations\n\nfrom .common import UnzonedDatetime\n",
        1,
    )


def write(target: Generated) -> None:
    target.output.write_text(generate(target), encoding="utf-8")
    print(f"wrote {target.output}")


def check(target: Generated) -> str | None:
    """None when the committed module matches a fresh generation, else why not."""
    current = target.output.read_text(encoding="utf-8") if target.output.is_file() else ""
    if current == generate(target):
        print(f"{target.output.name} is up to date")
        return None

    return (
        f"{target.output} is STALE — the committed models no longer match "
        f"{target.spec.name}.\n"
        "Regenerate:  uv run --project tests --frozen python tests/generate_schemas.py"
    )


def check_bodyless(target: Bodyless) -> str | None:
    """None while the service still describes no body, else what to do about it."""
    found = modellable_bodies(target.spec)
    if not found:
        print(f"{target.name}: nothing to generate — {target.reason}")
        return None

    listed = ", ".join(found[:5]) + (f", … ({len(found)} total)" if len(found) > 5 else "")
    return (
        f"{target.name} now describes bodies ({listed}) — the reason it is listed as "
        f"Bodyless ({target.reason}) no longer holds.\n"
        "Promote it to a Generated entry in TARGETS."
    )


def check_untrusted(target: Untrusted) -> str | None:
    """None while the committed document is still the one that cannot be trusted."""
    if not target.spec.is_file():
        return f"{target.spec} is gone — drop the {target.name} entry from TARGETS."

    changed = target.still_the_wrong_document(target.spec)
    if changed is None:
        print(f"{target.name}: nothing to generate — {target.reason}")
        return None

    return (
        f"{target.name}'s document {changed} — the reason it is listed as "
        f"Untrusted ({target.reason}) no longer holds.\n"
        "Promote it to a Generated entry in TARGETS and retire the hand-written "
        "stand/api/schemas/identity.py in the same change."
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if a committed module differs from a fresh generation",
    )
    args = parser.parse_args()

    problems: list[str] = []
    for target in TARGETS:
        # A not-generated claim is verified in BOTH modes: a document that
        # outgrew its exemption is news whichever way the script was invoked.
        if isinstance(target, Bodyless):
            problem = check_bodyless(target)
        elif isinstance(target, Untrusted):
            problem = check_untrusted(target)
        elif args.check:
            problem = check(target)
        else:
            write(target)
            problem = None

        if problem:
            problems.append(problem)

    for problem in problems:
        print(problem, file=sys.stderr)

    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
