"""What the suite actually exercised, and whether that is enough.

Two halves, and they never run at the same time.

**Recording** is imported by the suite: `ApiClient.request` calls `record` on
every response, so the ledger is a byproduct of the requests the tests already
make rather than a list somebody maintains. `tests/stand/conftest.py` writes it
out at `pytest_sessionfinish`.

**The gate** (`python3 coverage.py --observed … --spec …`, stdlib only, no
stand) reads that ledger back and answers two questions:

  1. Did every operation the CATALOGUE names get exercised, by something other
     than the anonymous sweep? `api/test_gateway.py` calls all 48 operations
     without a session, so every one has an observation — and a route whose only
     observed code is 401 was swept and never tested. Counting it as covered is
     precisely the mistake this gate exists to prevent.
  2. Did every status code the analytics CONTRACT declares get observed?

Only analytics is gated on its spec. That was once because the committed
identity document was the retired .NET contract; it no longer is — identity
emits its own document and CI drift-gates it beside analytics. What still blocks
(2) for identity is the other side of the comparison: every status code the
document declares has to be OBSERVED, and `.standard_errors` stamps the full
error set onto every operation. Identity stays held to (1) until the suite
either observes those codes or the gate learns to discount the stamped ones
(#1669), which is a change to the gate rather than to this note.

This is a port of `src/ingestion/tests/e2e/lib/api_coverage.py`. The universal
table agrees with it — the rig dropped 401 from its own exclusions once its host
began verifying the gateway JWT, so 429 is all either one drops.

403 is where they still part. The rig blocks it per-route as `.standard_errors`
boilerplate: with no role gate in front of an in-process service, a refusal it
cannot produce cannot be required of it. Here every request crosses a real
gateway carrying a real session, so 403 is reachable and REQUIRED. A gate that
inherited the rig's per-route exclusions would be blind to the authorization
behaviour this suite exists for.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections.abc import Iterable, Iterator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

_HTTP_METHODS = ("get", "put", "post", "delete", "patch", "head", "options", "trace")

#: Server faults are declared for fidelity and cannot be induced deterministically
#: from outside, so they never count against coverage.
SERVER_FAULT_FLOOR = 500

#: Excluded on every analytics route. 429 only — nothing rate-limits this stand.
#:
#: NOT 401 and NOT 403. See the module docstring: they are the two codes a
#: deployed stand is uniquely able to prove, so excluding them here would
#: discard the reason this suite exists. The rig now agrees on 401 and still
#: blocks 403 per-route.
UNIVERSAL_BOILERPLATE = frozenset({429})

#: Per-operation declared codes this suite cannot reach, with the reason. The
#: committed analytics spec is stamped by `.standard_errors`, which puts
#: {400,401,403,404,409,429,500} on every route regardless of what the handler
#: can answer (#1669), so without this the gate would demand statuses the API
#: never returns.
#:
#: Self-cleaning: an entry is reported when it becomes observed, when its
#: operation leaves the spec, and when the spec stops declaring the code it
#: subtracts — the last of which is what every #1669 fidelity fix produces.
#:
#: Two codes are subtracted here, each for a reason read out of the service.
#:
#: 403 — analytics has three places that can produce one, and most of its
#: operations reach none of them.
#:
#:   * `domain/person_visibility.rs`  — asking about somebody outside the
#:     caller's visible set. Reached only by `POST /v1/metric-results`.
#:   * `api/admin/error_map.rs::not_tenant_admin_response` — reached by the
#:     admin-threshold surface, but its `is_tenant_admin` is a STUB returning
#:     true for every session ("until the real Auth wiring lands",
#:     `domain/auth.rs`). So the role half is unreachable product-wide, and only
#:     the OTHER trigger fires: a write addressing a row in another tenant.
#:     That is `update` and `delete`, which take an id; `list`, `get_one` and
#:     `create` cannot reach it, and a cross-tenant READ is 404 by opacity
#:     (`threshold_not_found_response`).
#:   * `admin_threshold/lock_enforcer.rs` — `threshold_locked`, on all three
#:     WRITES. `create` reaches it too (service.rs step 8, `check_broader_locks`):
#:     a broader scope's lock shadows a narrower create. Reads do not.
#:
#: Nothing else in the service gates on anything: `is_tenant_admin`,
#: `authorize_*` and `require_admin` appear nowhere in `handlers.rs`,
#: `saved_queries.rs`, `catalog.rs`, `metric_definitions.rs` or
#: `metric_drilldown.rs`. The spec declared 403 on every route anyway, because
#: `.standard_errors` stamped the same seven codes on all of them (#1669); the
#: fidelity fixes are correcting that route by route, and each one that lands
#: retires an entry below.
#:
#: This does NOT retract the module docstring's point. 403 stays out of
#: UNIVERSAL_BOILERPLATE — where a handler HAS the path, this suite is the only
#: one that can prove it, and it does. What is subtracted here is
#: per-route and sourced, which is also how the entries expire: when the real
#: authorization wiring lands, these become observable and the gate says so.
_NO_AUTHORIZATION_PATH = frozenset({403})

#: 409 is declared on every route and NO route can send one: `already_exists`,
#: `aborted` and `conflict` appear nowhere in the service. A route that grows a
#: real conflict path makes its entry observable, and this gate's
#: `blocked-now-observed` advisory names it.
_NO_CONFLICT_PATH = frozenset({409})
_NO_AUTHZ_OR_CONFLICT = _NO_AUTHORIZATION_PATH | _NO_CONFLICT_PATH

BLOCKED: dict[str, frozenset[int]] = {
    # Saved queries — no owner check and no role check; cross-tenant is 404.
    "GET /v1/queries": _NO_AUTHZ_OR_CONFLICT,
    "POST /v1/queries": _NO_AUTHZ_OR_CONFLICT,
    "GET /v1/queries/{id}": _NO_AUTHZ_OR_CONFLICT,
    "PUT /v1/queries/{id}": _NO_AUTHZ_OR_CONFLICT,
    "DELETE /v1/queries/{id}": _NO_AUTHZ_OR_CONFLICT,
    "POST /v1/queries/{id}/run": _NO_AUTHZ_OR_CONFLICT,
    # `POST /v1/metric-drilldown` and `GET /v1/metric-definitions` used to sit
    # here. #2134 corrected their declarations, so there is no longer a 403 to
    # subtract — the gate reported both as stale and they came out.
    # `/export` still carries the old boilerplate and the same absent gate.
    "POST /v1/metric-drilldown/export": _NO_AUTHZ_OR_CONFLICT,
    # `POST /v1/metric-results` needs no entry at all: #2134 already removed
    # 409 from its declaration, and this gate said so when it was added here.
    # Custom metrics — tenant-scoped, no owner check and no role gate, so no 403
    # path on any of them. Most have no conflict path either; `POST /v1/metrics`
    # is the exception — a duplicate `metric_key` is a real 409, covered by a
    # test rather than blocked, so it subtracts only 403.
    "GET /v1/metrics": _NO_AUTHZ_OR_CONFLICT,
    "POST /v1/metrics": _NO_AUTHORIZATION_PATH,
    "GET /v1/metrics/export": _NO_AUTHZ_OR_CONFLICT,
    "POST /v1/metrics/import": _NO_AUTHZ_OR_CONFLICT,
    "GET /v1/metrics/{metric_key}": _NO_AUTHZ_OR_CONFLICT,
    "PUT /v1/metrics/{metric_key}": _NO_AUTHZ_OR_CONFLICT,
    "DELETE /v1/metrics/{metric_key}": _NO_AUTHZ_OR_CONFLICT,
}


def spec_operations(spec: Mapping[str, Any]) -> dict[str, list[int]]:
    """`"METHOD /path"` -> its declared status codes, from an OpenAPI document."""
    ops: dict[str, list[int]] = {}
    for path, methods in (spec.get("paths") or {}).items():
        for method, operation in methods.items():
            if method.lower() not in _HTTP_METHODS:
                continue
            codes = sorted(
                int(code) for code in (operation.get("responses") or {}) if str(code).isdigit()
            )
            ops[f"{method.upper()} {path}"] = codes
    return ops


def path_template_index(
    spec_ops: Iterable[str],
) -> dict[str, list[tuple[str, list[str]]]]:
    """Pre-split templates by method, fewest `{param}` segments first.

    Ordering matters: a literal path must win over a same-arity template
    regardless of the order the document happened to list them in.
    """
    index: dict[str, list[tuple[str, list[str]]]] = {}
    for key in spec_ops:
        method, path = key.split(" ", 1)
        index.setdefault(method, []).append((path, path.strip("/").split("/")))
    for templates in index.values():
        templates.sort(key=lambda t: sum(s.startswith("{") and s.endswith("}") for s in t[1]))
    return index


def match_path(
    method: str, path: str, index: Mapping[str, list[tuple[str, list[str]]]]
) -> str | None:
    """The `"METHOD template"` a concrete request path belongs to, or None."""
    verb = method.upper()
    segments = path.strip("/").split("/")
    for template, template_segments in index.get(verb, []):
        if len(template_segments) != len(segments):
            continue
        # No `strict=`: the length check above already guarantees the pairing,
        # and this module must stay runnable on an older interpreter than the
        # suite needs — it is a stdlib gate that CI runs without uv.
        if all(
            (t.startswith("{") and t.endswith("}")) or t == o
            for t, o in zip(template_segments, segments)  # noqa: B905
        ):
            return f"{verb} {template}"
    return None


# ── recording half (imported by the suite) ─────────────────────────────────

#: `(METHOD, gateway path)` -> observed status codes. Module-level because the
#: suite is one process; xdist would need this reduced per worker, which is why
#: the workflow runs it serially.
_OBSERVED: dict[tuple[str, str], set[int]] = {}


def record(method: str, path: str, status_code: int) -> None:
    """Note one response. Metadata only — never the body."""
    _OBSERVED.setdefault((method.upper(), path), set()).add(int(status_code))


def reset() -> None:
    _OBSERVED.clear()


@contextmanager
def isolated() -> Iterator[None]:
    """Record into a private ledger, restoring the caller's on exit.

    The suite records into the module global above and `tests/stand/conftest.py`
    dumps it once, at session end. So anything that calls `reset()` part-way
    through a session does not start a new measurement — it DELETES the run's,
    and the gate then reports on whatever happened afterwards.

    Not hypothetical: a gate that reads a wiped ledger reports a catastrophe in
    the suite rather than a bug in itself, which is the most expensive way to be
    wrong. `tests/stand/meta/conftest.py` applies this autouse for that reason.
    """
    saved = {key: set(codes) for key, codes in _OBSERVED.items()}
    try:
        yield
    finally:
        _OBSERVED.clear()
        _OBSERVED.update(saved)


def observed_rows() -> list[dict[str, Any]]:
    return [
        {"method": method, "path": path, "statuses": sorted(codes)}
        for (method, path), codes in sorted(_OBSERVED.items())
    ]


def dump(path: str | Path) -> Path:
    """Write the ledger, MERGING into whatever is already there.

    Merging rather than overwriting because a developer runs the suite in
    pieces — `-k`, one directory, then another — against one stand, and a plain
    overwrite would report the last slice as if it were the whole run. Delete
    the file for a from-scratch measurement.
    """
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)

    merged: dict[tuple[str, str], set[int]] = {}
    if out.is_file():
        for row in json.loads(out.read_text(encoding="utf-8")):
            merged.setdefault((row["method"], row["path"]), set()).update(
                int(code) for code in row["statuses"]
            )
    for key, codes in _OBSERVED.items():
        merged.setdefault(key, set()).update(codes)

    rows = [
        {"method": method, "path": path, "statuses": sorted(codes)}
        for (method, path), codes in sorted(merged.items())
    ]
    out.write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    return out


# ── gate half (pure, stdlib only) ──────────────────────────────────────────


@dataclass(frozen=True)
class Operation:
    """One catalogued operation, as the ledger will have seen it.

    `template` is the parameterised form (`/v1/queries/{id}`); `path` is the
    concrete url the catalogue names, with a stand-in substituted. They differ
    for every operation that takes a path parameter, and conflating them is
    what this field exists to stop — see `fold_onto_catalogue`.
    """

    method: str
    path: str
    template: str | None = None

    @property
    def label(self) -> str:
        return f"{self.method} {self.path}"

    @property
    def key(self) -> str:
        """How the operation is identified in a report: the template if it has
        one, so `PUT /v1/queries/{id}` is one row rather than one per id."""
        return f"{self.method} {self.template or self.path}"


def fold_onto_catalogue(
    observed: Mapping[str, set[int]], catalogue: Sequence[Operation]
) -> dict[str, set[int]]:
    """Group observed calls by the catalogued operation they belong to.

    Without this the catalogue half compares literal paths while the spec half
    folds templates, and the two disagree about the same run. A test that
    updates a real threshold records
    `PUT /api/analytics/v1/queries/019fc6c8-…`, which matches
    the catalogue's stand-in id nowhere — so the only call left against the
    catalogued url is the anonymous sweep's, and the operation is reported
    SWEPT ONLY while a passing test is exercising it.

    Not hypothetical: it is what the gate said about both admin-threshold
    writes on the run that first turned green, each of which had answered 200
    and 403 to a real session moments earlier.
    """
    index = path_template_index(operation.key for operation in catalogue)
    folded: dict[str, set[int]] = {}
    for label, codes in observed.items():
        method, _, path = label.partition(" ")
        matched = match_path(method, path, index)
        if matched is not None:
            folded.setdefault(matched, set()).update(codes)
    return folded


@dataclass
class CatalogueReport:
    """Which catalogued operations were genuinely exercised.

    "Genuinely" is the whole content of this report. `api/test_gateway.py`
    sweeps every operation anonymously, so presence in the ledger proves nothing
    — an operation whose only observed status is 401 was swept and never tested.
    """

    catalogue: Sequence[Operation]
    observed: Mapping[str, set[int]]
    swept_only: list[str] = field(default_factory=list)
    unobserved: list[str] = field(default_factory=list)
    exercised: list[str] = field(default_factory=list)

    def __post_init__(self) -> None:
        folded = fold_onto_catalogue(self.observed, self.catalogue)
        for operation in self.catalogue:
            codes = folded.get(operation.key)
            if not codes:
                self.unobserved.append(operation.key)
            elif codes <= {401}:
                self.swept_only.append(operation.key)
            else:
                self.exercised.append(operation.key)

    @property
    def passed(self) -> bool:
        return not self.unobserved and not self.swept_only


@dataclass
class SpecReport:
    """Per-status-code coverage of one trusted OpenAPI document."""

    spec_ops: Mapping[str, list[int]]
    validated: Mapping[str, set[int]]
    unmatched: Sequence[Mapping[str, Any]]

    def __post_init__(self) -> None:
        ops = set(self.spec_ops)
        self.missing = sorted(op for op in ops if op not in self.validated)
        self.required = {op: self.required_codes(op) for op in ops}
        self.uncovered = {
            op: gap
            for op in sorted(ops)
            if op in self.validated and (gap := self.required[op] - self.validated[op])
        }
        # An excluded code that turns up anyway means the exclusion is wrong.
        self.blocked_observed = {
            op: seen
            for op in sorted(ops)
            if (
                seen := (UNIVERSAL_BOILERPLATE | set(BLOCKED.get(op, frozenset())))
                & self.validated.get(op, set())
            )
        }
        self.stale_blocked = [op for op in sorted(BLOCKED) if op not in ops]
        # The commoner staleness, and the one the operation-level check above
        # misses entirely: the operation is still documented, but a code this
        # list subtracts is no longer declared for it. That is exactly the shape
        # every #1669 fidelity fix produces — the document stops over-declaring,
        # and an exclusion written against the old text starts suppressing
        # nothing while still reading as a live judgement about the route.
        self.blocked_undeclared = {
            op: gone
            for op in sorted(BLOCKED)
            if op in ops and (gone := set(BLOCKED[op]) - set(self.spec_ops[op]))
        }
        # Codes the suite proved that the document does not declare — the
        # under-declaration half of #1669, invisible in the matrix below.
        self.undeclared = {
            op: extra
            for op in sorted(ops)
            if (extra := self.validated.get(op, set()) - set(self.spec_ops[op]))
        }

        coverable = sum(len(codes) for codes in self.required.values())
        covered = sum(len(self.required[op] & self.validated.get(op, set())) for op in ops)
        self.total_coverable = coverable
        self.total_covered = covered
        self.coverage_pct = 100.0 if not coverable else round(100.0 * covered / coverable, 1)

    def required_codes(self, op: str) -> set[int]:
        declared = self.spec_ops.get(op, [])
        return (
            {code for code in declared if code < SERVER_FAULT_FLOOR}
            - UNIVERSAL_BOILERPLATE
            - set(BLOCKED.get(op, frozenset()))
        )

    @property
    def passed(self) -> bool:
        return not self.missing


def load_ledger(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = json.loads(path.read_text(encoding="utf-8"))
    return rows


def by_label(rows: Iterable[Mapping[str, Any]]) -> dict[str, set[int]]:
    """The ledger keyed by `"METHOD path"`, for the catalogue check.

    Concrete paths, not templates: the catalogue names concrete paths too, so
    the two line up without any matching.
    """
    out: dict[str, set[int]] = {}
    for row in rows:
        label = f"{row['method']} {row['path']}"
        out.setdefault(label, set()).update(int(code) for code in row["statuses"])
    return out


def match_against_spec(
    rows: Iterable[Mapping[str, Any]], prefix: str, spec_ops: Mapping[str, list[int]]
) -> tuple[dict[str, set[int]], list[dict[str, Any]]]:
    """Fold gateway-prefixed observations onto the service's own operations.

    The suite calls `/api/analytics/v1/queries`; the document describes
    `/v1/queries`. `strip_prefix: true` in the gateway's route table is what
    makes those the same request, so stripping it here is reading the route
    table, not guessing.
    """
    index = path_template_index(spec_ops)
    validated: dict[str, set[int]] = {}
    unmatched: list[dict[str, Any]] = []

    for row in rows:
        path = str(row["path"])
        if not path.startswith(prefix):
            continue
        hit = match_path(str(row["method"]), path[len(prefix) :] or "/", index)
        if hit is None:
            unmatched.append(dict(row))
        else:
            validated.setdefault(hit, set()).update(int(code) for code in row["statuses"])
    return validated, unmatched


def violations(catalogue: CatalogueReport, spec: SpecReport | None) -> list[str]:
    """Blocking findings. A non-empty list fails the gate."""
    out = [
        f"NEVER CALLED: {label} is in the operation catalogue and the ledger has no "
        "record of it — operations.py names a route no test reaches"
        for label in catalogue.unobserved
    ]
    out += [
        f"SWEPT ONLY: {label} was called anonymously and never with a session, so the "
        "only thing proven about it is that the edge refuses it"
        for label in catalogue.swept_only
    ]
    if spec is not None:
        out += [f"MISSING: {op} is documented and exercised by no test" for op in spec.missing]
    return out


def advisories(spec: SpecReport | None) -> list[str]:
    """Reported, never blocking — the picture, and the suppression lists' hygiene."""
    if spec is None:
        return []
    out = [
        f"uncovered code: {op} has not answered declared {sorted(gap)} "
        f"(saw {sorted(spec.validated[op])})"
        for op, gap in spec.uncovered.items()
    ]
    out += [
        f"blocked-now-observed: {op} answered {sorted(seen)}, which the exclusions call "
        "unreachable — drop the entry"
        for op, seen in spec.blocked_observed.items()
    ]
    out += [
        f"stale BLOCKED: {op} is no longer in the spec — drop the entry"
        for op in spec.stale_blocked
    ]
    out += [
        f"stale BLOCKED: {op} no longer declares {sorted(gone)} — the document was "
        "corrected, so the exclusion suppresses nothing; drop it"
        for op, gone in spec.blocked_undeclared.items()
    ]
    out += [
        f"observed but undeclared: {op} answered {sorted(codes)}, which the document does "
        "not declare — the suite covers it, the contract does not describe it"
        for op, codes in spec.undeclared.items()
    ]
    return out


def render(catalogue: CatalogueReport, spec: SpecReport | None) -> str:
    blocking = violations(catalogue, spec)
    verdict = "✅ PASS" if not blocking else "❌ FAIL"
    lines = [
        "# Stand API coverage",
        "",
        f"**Gate: {verdict}.** {len(catalogue.exercised)}/{len(catalogue.catalogue)} catalogued "
        f"operations exercised with a session"
        + (f" · {len(catalogue.swept_only)} swept only" if catalogue.swept_only else "")
        + (f" · {len(catalogue.unobserved)} never called" if catalogue.unobserved else ""),
    ]
    if spec is not None:
        lines.append(
            f"Analytics contract: **{spec.coverage_pct}%** of coverable status codes "
            f"({spec.total_covered}/{spec.total_coverable})."
        )
    lines += [
        "",
        "_Blocking: a catalogued operation never called, or called only by the anonymous "
        "sweep, or a documented operation no test exercises. Per-status-code coverage is "
        "reported, not enforced._",
        "",
    ]

    if spec is not None:
        codes = sorted({code for declared in spec.spec_ops.values() for code in declared})
        lines += [
            "| operation | " + " | ".join(str(c) for c in codes) + " | covered |",
            "|---|" + "---|" * (len(codes) + 1),
        ]
        for op in sorted(spec.spec_ops):
            declared = set(spec.spec_ops[op])
            coverable = spec.required[op]
            seen = spec.validated.get(op, set())
            row = [
                ""
                if code not in declared
                else "·"
                if code not in coverable
                else "✓"
                if code in seen
                else "✗"
                for code in codes
            ]
            covered = "—" if not coverable else f"{len(coverable & seen)}/{len(coverable)}"
            label = f"❌ `{op}`" if op in spec.missing else f"`{op}`"
            lines.append(f"| {label} | " + " | ".join(row) + f" | {covered} |")
        if spec.unmatched:
            lines += ["", "## ⚠️ Observed but unmatched (informational)", ""]
            lines += [
                f"- `{row['method']} {row['path']}` → {sorted(row['statuses'])}"
                for row in spec.unmatched
            ]

    if blocking:
        lines += ["", "## ❌ Gate violations (blocking)", ""] + [f"- {v}" for v in blocking]
    notes = advisories(spec)
    if notes:
        lines += ["", "## ⚠️ Advisories (reported, non-blocking)", ""] + [f"- {n}" for n in notes]
    return "\n".join(lines) + "\n"


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Stand API coverage gate.")
    parser.add_argument("--observed", required=True, help="ledger written at pytest_sessionfinish")
    parser.add_argument("--catalogue", required=True, help="JSON list of catalogued operations")
    parser.add_argument("--spec", help="committed analytics OpenAPI document")
    parser.add_argument("--prefix", default="/api/analytics", help="gateway prefix for --spec")
    args = parser.parse_args(argv)

    ledger_path = Path(args.observed)
    if not ledger_path.is_file():
        print(
            f"ERROR: {ledger_path} not found — the suite must run first (it writes the "
            "ledger at pytest_sessionfinish)",
            file=sys.stderr,
        )
        return 2

    rows = load_ledger(ledger_path)
    catalogue = CatalogueReport(
        catalogue=[
            Operation(
                method=entry["method"],
                path=entry["path"],
                # Absent in a ledger written by an older suite; falling back to
                # the concrete path reproduces the pre-template behaviour rather
                # than crashing on it.
                template=entry.get("template"),
            )
            for entry in json.loads(Path(args.catalogue).read_text(encoding="utf-8"))
        ],
        observed=by_label(rows),
    )

    spec_report: SpecReport | None = None
    if args.spec:
        spec_ops = spec_operations(json.loads(Path(args.spec).read_text(encoding="utf-8")))
        validated, unmatched = match_against_spec(rows, args.prefix, spec_ops)
        spec_report = SpecReport(spec_ops=spec_ops, validated=validated, unmatched=unmatched)

    sys.stdout.write(render(catalogue, spec_report))
    return 0 if not violations(catalogue, spec_report) else 1


__all__: Sequence[str] = (
    "BLOCKED",
    "SERVER_FAULT_FLOOR",
    "UNIVERSAL_BOILERPLATE",
    "CatalogueReport",
    "Operation",
    "SpecReport",
    "advisories",
    "by_label",
    "dump",
    "fold_onto_catalogue",
    "isolated",
    "match_against_spec",
    "match_path",
    "observed_rows",
    "path_template_index",
    "record",
    "render",
    "reset",
    "spec_operations",
    "violations",
)


if __name__ == "__main__":
    raise SystemExit(main())
