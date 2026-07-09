#!/usr/bin/env python3
"""API endpoint coverage report — which analytics routes the e2e suite exercises.

Recording half (imported by the rig): `record_response` is an httpx response
event-hook on the single client every suite request flows through; it records
`(method, path) -> {status codes}` into a module-level ledger that
`conftest.pytest_sessionfinish` dumps to `.artifacts/observed_endpoints.json`.

Gate half (`python3 lib/api_coverage.py`, stdlib only; blocking in `./e2e.sh
gates` and CI): loads that ledger plus the committed OpenAPI spec and reports
per-operation coverage. The gate FAILS only when a documented operation is
exercised by no test, or a SKIP_LIST entry rots. Per-status-code coverage is
REPORTED, not enforced: each declared code is `✓` observed / `✗` unobserved /
`·` excluded (5xx + UNIVERSAL_BOILERPLATE + BLOCKED[op], see below). Excluded-set
hygiene is a non-blocking advisory. Rationale: docs/domain/bronze-to-api-e2e/specs.

    python3 lib/api_coverage.py --observed .artifacts/observed_endpoints.json \
        --spec docs/components/backend/analytics/openapi.json
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from pathlib import Path

_HTTP_METHODS = ("get", "put", "post", "delete", "patch", "head", "options", "trace")

# Operations no test exercises, "METHOD path" -> reason. A listed op the suite
# DOES hit (redundant) or that left the spec (stale) fails the gate. EMPTY on
# purpose: the api/ contract tests exercise every operation.
SKIP_LIST: list[tuple[str, str]] = []

# Server-fault codes (>= this) are declared for spec fidelity but never required:
# a black-box contract test can't deterministically induce a 500.
SERVER_FAULT_FLOOR = 500

# The committed spec is generated from `.standard_errors(openapi)`, which stamps a
# uniform {400,401,403,404,409,429,500} on every route regardless of what the
# handler can answer (spec-fidelity bug #1669). The gate subtracts the codes a
# route provably cannot produce, or it would require statuses the API never
# returns. UNIVERSAL_BOILERPLATE drops from every route: 401 (auth disabled at the
# gateway) and 429 (no rate limiter).
UNIVERSAL_BOILERPLATE = frozenset({401, 429})

# Per-route declared codes the rig cannot observe, subtracted from `required` on
# top of UNIVERSAL_BOILERPLATE — tagged per entry: `.standard_errors` boilerplate
# the handler can't answer (#1669), or a pinned rig/product bug (#1663 legacy-
# threshold reads 500; #1664 admin duplicate-create 500s not 409). Self-cleaning:
# an entry that becomes observed or leaves the spec fails the hygiene advisory.
BLOCKED: dict[str, frozenset[int]] = {
    "GET /v1/metrics": frozenset({400, 403, 404, 409}),  # boilerplate: list, no input/lookup/conflict
    "POST /v1/metrics": frozenset({403, 404, 409}),  # boilerplate
    "GET /v1/metrics/{id}": frozenset({403, 409}),  # boilerplate
    "PUT /v1/metrics/{id}": frozenset({403, 409}),  # boilerplate
    "DELETE /v1/metrics/{id}": frozenset({403, 409}),  # boilerplate
    "POST /v1/metrics/{id}/query": frozenset({403, 409}),  # boilerplate
    "POST /v1/metrics/queries": frozenset({403, 404, 409}),  # boilerplate (per-item errors embed in 200)
    "GET /v1/columns": frozenset({400, 403, 404, 409}),  # boilerplate
    "GET /v1/columns/{table}": frozenset({400, 403, 404, 409}),  # boilerplate: unknown table → empty 200
    "POST /v1/catalog/get_metrics": frozenset({403, 404, 409}),  # boilerplate
    "GET /v1/admin/metric-thresholds": frozenset({403, 404, 409}),  # boilerplate
    "GET /v1/admin/metric-thresholds/{id}": frozenset({403, 409}),  # boilerplate
    "PUT /v1/admin/metric-thresholds/{id}": frozenset({409}),  # boilerplate (403 IS reachable: cross-tenant)
    "DELETE /v1/admin/metric-thresholds/{id}": frozenset({409}),  # boilerplate (403 reachable: cross-tenant)
    # legacy thresholds: 403/409 boilerplate; the success code is #1663 (500 on read-back)
    "GET /v1/metrics/{id}/thresholds": frozenset({200, 403, 409}),  # 200=#1663
    "POST /v1/metrics/{id}/thresholds": frozenset({201, 403, 409}),  # 201=#1663
    "PUT /v1/metrics/{id}/thresholds/{tid}": frozenset({200, 403, 409}),  # 200=#1663
    "DELETE /v1/metrics/{id}/thresholds/{tid}": frozenset({204, 403, 409}),  # 204=#1663
    "POST /v1/admin/metric-thresholds": frozenset({404, 409}),  # 404 boilerplate; 409=#1664
    # persons 200/404 covered via the in-process Identity stub (#1691); rest boilerplate
    "GET /v1/persons/{email}": frozenset({400, 403, 409}),
    # 403/404/409 boilerplate; the 200 happy-path needs seeded observation data (a `✗` gap)
    "POST /v1/metric-results": frozenset({403, 404, 409}),
}


# ── recording half (imported by the rig) ──────────────────────────────────

# (method, path) -> set of observed status codes. Module-level so the single
# serial pytest process accumulates across every test (xdist is off in CI).
_OBSERVED: dict[tuple[str, str], set[int]] = {}


def record_response(response) -> None:
    """httpx response event-hook: log this request's method+path+status.

    Reads only metadata off the (already-received) response — never the body —
    so it is a transparent observer of the existing request path.
    """
    req = response.request
    key = (req.method.upper(), req.url.path)
    _OBSERVED.setdefault(key, set()).add(int(response.status_code))


def reset_observed() -> None:
    _OBSERVED.clear()


def dump_observed(path: str | Path) -> Path:
    """Write the in-process ledger, MERGING into an existing file.

    Each CI lane (e2e-api, e2e-metrics — see e2e-bronze-to-api.yml) is a
    single fresh pytest session on its own runner, so there's no cross-session
    ledger to merge there. The merge exists for LOCAL runs instead: `./e2e.sh
    test api/` followed by `./e2e.sh test metrics/` share one `.artifacts/`
    dir, and a plain overwrite would drop whichever suite ran first — merging
    unions statuses per (method, path) across those local sessions. Delete
    `.artifacts/` first for a from-scratch measurement.
    """
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    merged: dict[tuple[str, str], set[int]] = {}
    if out.exists():
        for row in json.loads(out.read_text(encoding="utf-8")):
            merged.setdefault((row["method"], row["path"]), set()).update(
                int(s) for s in row["statuses"]
            )
    for key, codes in _OBSERVED.items():
        merged.setdefault(key, set()).update(codes)
    rows = [
        {"method": m, "path": p, "statuses": sorted(codes)}
        for (m, p), codes in sorted(merged.items())
    ]
    out.write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    return out


# ── gate half (pure; stdlib only) ─────────────────────────────────────────


def skip_index() -> dict[str, str]:
    idx: dict[str, str] = {}
    for op, reason in SKIP_LIST:
        if op in idx:
            raise ValueError(f"duplicate SKIP_LIST entry: {op}")
        idx[op] = reason
    return idx


def spec_operations(spec: dict) -> dict[str, list[int]]:
    """Map "METHOD path" -> sorted declared status codes, from an OpenAPI doc."""
    ops: dict[str, list[int]] = {}
    for path, methods in spec.get("paths", {}).items():
        for method, op in methods.items():
            if method.lower() not in _HTTP_METHODS:
                continue
            codes = sorted(
                int(c) for c in (op.get("responses") or {}) if str(c).isdigit()
            )
            ops[f"{method.upper()} {path}"] = codes
    return ops


def match_observed(observed: list[dict], spec_ops: dict[str, list[int]]) -> tuple[dict[str, set[int]], list[dict]]:
    """Map each observed concrete request onto a spec operation.

    Returns (validated, unmatched): `validated` is "METHOD path" -> set of
    observed status codes for matched spec ops; `unmatched` are observed
    requests with no spec op (path-template mismatch, or an undocumented route).
    """
    # Pre-split spec paths once for template matching. Within a method, try
    # templates with FEWER {param} segments first, so a literal path (e.g. a
    # future GET /v1/metrics/summary) wins over a same-arity template
    # (GET /v1/metrics/{id}) regardless of spec ordering.
    spec_paths: dict[str, list[tuple[str, list[str]]]] = {}
    for key in spec_ops:
        method, path = key.split(" ", 1)
        spec_paths.setdefault(method, []).append((path, path.strip("/").split("/")))
    for templates in spec_paths.values():
        templates.sort(key=lambda t: sum(s.startswith("{") and s.endswith("}") for s in t[1]))

    validated: dict[str, set[int]] = {}
    unmatched: list[dict] = []
    for row in observed:
        method = row["method"].upper()
        obs_path = row["path"]
        obs_segs = obs_path.strip("/").split("/")
        hit = None
        for tmpl, tmpl_segs in spec_paths.get(method, []):
            if len(tmpl_segs) != len(obs_segs):
                continue
            if all(
                t.startswith("{") and t.endswith("}") or t == o
                for t, o in zip(tmpl_segs, obs_segs)
            ):
                hit = f"{method} {tmpl}"
                break
        if hit is None:
            unmatched.append(row)
        else:
            validated.setdefault(hit, set()).update(int(c) for c in row["statuses"])
    return validated, unmatched


@dataclasses.dataclass
class CoverageReport:
    spec_ops: dict[str, list[int]]  # METHOD path -> declared status codes
    validated: dict[str, set[int]]  # METHOD path -> observed status codes
    unmatched: list[dict]
    skips: dict[str, str]

    def __post_init__(self) -> None:
        ops = set(self.spec_ops)
        self.covered = sorted(op for op in ops if op in self.validated)
        self.skipped = sorted(op for op in ops if op not in self.validated and op in self.skips)
        self.missing = sorted(op for op in ops if op not in self.validated and op not in self.skips)
        # Hygiene: skips that are actually exercised, or no longer in the spec.
        self.redundant_skips = sorted(op for op in self.skips if op in self.validated)
        self.stale_skips = sorted(op for op in self.skips if op not in ops)
        # Per-code coverage: every REQUIRED code must have been observed, where
        # required(op) = declared − {c >= SERVER_FAULT_FLOOR} − UNIVERSAL_BOILERPLATE
        #                − BLOCKED[op].
        self.required: dict[str, set[int]] = {op: self.required_codes(op) for op in ops}
        self.uncovered: dict[str, set[int]] = {}  # op -> required codes never seen
        for op in self.covered:
            gap = self.required[op] - self.validated[op]
            if gap:
                self.uncovered[op] = gap
        # Hygiene on the excluded sets (mirrors SKIP_LIST): an excluded code that
        # is now observed (spec fixed → real code lands, or bug/backend fixed) or
        # no longer declared -> FAIL, forcing the scaffolding to be actualized.
        self.blocked_observed: dict[str, set[int]] = {}
        self.stale_blocked: list[str] = []
        for op, codes in BLOCKED.items():
            if op not in ops:
                self.stale_blocked.append(f"{op} (operation gone from the spec)")
                continue
            gone = set(codes) - set(self.spec_ops[op])
            if gone:
                self.stale_blocked.append(f"{op} (codes {sorted(gone)} no longer declared)")
        # UNIVERSAL_BOILERPLATE (401/429) + per-op BLOCKED: any that is actually
        # observed means the exclusion is wrong — surface it.
        for op in ops:
            excluded = UNIVERSAL_BOILERPLATE | set(BLOCKED.get(op, frozenset()))
            now_seen = excluded & self.validated.get(op, set())
            if now_seen:
                self.blocked_observed[op] = now_seen
        # Per-status-code coverage percentage (REPORTED, not enforced): of the
        # coverable codes (required_codes = declared − 5xx − boilerplate −
        # BLOCKED[op]), how many the suite observed. The `·`/excluded codes are
        # not in the denominator.
        self.covered_codes: dict[str, set[int]] = {
            op: self.required[op] & self.validated.get(op, set()) for op in ops
        }
        self.total_coverable = sum(len(c) for c in self.required.values())
        self.total_covered = sum(len(c) for c in self.covered_codes.values())
        self.coverage_pct = (
            100.0
            if self.total_coverable == 0
            else round(100.0 * self.total_covered / self.total_coverable, 1)
        )

    def required_codes(self, op: str) -> set[int]:
        """Declared codes the suite must observe: drop server-fault 5xx, the
        universal boilerplate (401/429), and the per-op BLOCKED set. May be empty
        (an op all of whose declared codes are 5xx/boilerplate/BLOCKED), in which
        case the op contributes nothing to the coverage % and passes once merely
        exercised."""
        declared = self.spec_ops.get(op, [])
        excluded = BLOCKED.get(op, frozenset())
        return (
            {c for c in declared if c < SERVER_FAULT_FLOOR}
            - UNIVERSAL_BOILERPLATE
            - set(excluded)
        )

    @property
    def passed(self) -> bool:
        # Gate blocks ONLY on a documented operation that no test exercises (a
        # new endpoint), plus SKIP_LIST rot. Per-status-code coverage and
        # excluded-set hygiene are REPORTED (advisories), never enforced.
        return not (self.missing or self.redundant_skips or self.stale_skips)


def build_report(spec: dict, observed: list[dict]) -> CoverageReport:
    spec_ops = spec_operations(spec)
    validated, unmatched = match_observed(observed, spec_ops)
    return CoverageReport(spec_ops=spec_ops, validated=validated, unmatched=unmatched, skips=skip_index())


def _statuses(codes) -> str:
    return ", ".join(str(c) for c in sorted(codes)) if codes else "—"


def gate_violations(r: CoverageReport) -> list[str]:
    """BLOCKING findings — a non-empty list fails the gate (exit 1). Only a
    documented operation no test exercises (a new endpoint), plus SKIP_LIST rot."""
    out = []
    for op in r.missing:
        out.append(
            f"MISSING: {op} is exercised by no test and not in SKIP_LIST — "
            f"every documented operation must be exercised by at least one test"
        )
    for op in r.redundant_skips:
        out.append(f"REDUNDANT SKIP: {op} is now exercised — drop it from SKIP_LIST")
    for op in r.stale_skips:
        out.append(f"STALE SKIP: {op} is no longer in the spec — drop it from SKIP_LIST")
    return out


def advisories(r: CoverageReport) -> list[str]:
    """NON-blocking findings — reported so the coverage picture and the
    suppression lists stay honest, but they never fail the gate."""
    out = []
    for op, gap in sorted(r.uncovered.items()):
        out.append(
            f"uncovered code: {op} has not answered declared {sorted(gap)} "
            f"(saw {sorted(r.validated[op])}) — a coverage gap, not a gate failure"
        )
    for op, seen in sorted(r.blocked_observed.items()):
        out.append(
            f"blocked-now-observed: {op} answered {sorted(seen)}, which BLOCKED marks "
            f"unreachable — the bug/limitation is resolved, drop it from BLOCKED"
        )
    for entry in r.stale_blocked:
        out.append(f"stale BLOCKED: {entry} — drop the entry")
    return out


def render_markdown(r: CoverageReport) -> str:
    total = len(r.spec_ops)
    verdict = "✅ PASS" if r.passed else "❌ FAIL"
    # Columns = every REGISTERED (declared) status code across the spec.
    all_codes = sorted({c for codes in r.spec_ops.values() for c in codes})
    lines = [
        "# API endpoint coverage — by method+path",
        "",
        f"**Gate: {verdict}.** {len(r.covered)}/{total} operations exercised "
        f"· **{len(r.missing)} missing** (a documented operation no test exercises) "
        f"· registered-code coverage **{r.coverage_pct}%** "
        f"({r.total_covered}/{r.total_coverable} coverable codes seen).",
        "",
        "_The gate blocks ONLY on a missing operation (a new endpoint without a "
        "test). Per-status-code coverage below is REPORTED, not enforced: "
        "`✓` observed · `✗` declared but not yet observed · `·` excluded "
        f"(5xx / {sorted(UNIVERSAL_BOILERPLATE)} boilerplate / BLOCKED) · "
        "blank = not declared for that op._",
        "",
        "| operation | " + " | ".join(str(c) for c in all_codes) + " | covered |",
        "|---|" + "---|" * (len(all_codes) + 1),
    ]
    for op in sorted(r.spec_ops):
        declared = set(r.spec_ops[op])
        coverable = r.required[op]
        observed = r.validated.get(op, set())
        row = []
        for c in all_codes:
            if c not in declared:
                row.append("")
            elif c not in coverable:  # 5xx / boilerplate / BLOCKED
                row.append("·")
            elif c in observed:
                row.append("✓")
            else:
                row.append("✗")
        if op in r.missing:
            label = f"❌ `{op}`"
        elif op in r.skips:
            label = f"⏭️ `{op}`"
        else:
            label = f"`{op}`"
        cov = "—" if not coverable else f"{len(coverable & observed)}/{len(coverable)}"
        lines.append(f"| {label} | " + " | ".join(row) + f" | {cov} |")
    # Auditability: the `·` columns — declared codes excluded from the coverage %.
    if BLOCKED:
        lines += ["", "## Excluded from coverage (`·` — declared but not coverable)", ""]
        lines += [
            f"_Server-fault 5xx (500) and UNIVERSAL_BOILERPLATE {sorted(UNIVERSAL_BOILERPLATE)} "
            "(auth disabled / no rate limiter) are excluded on every route. The committed spec "
            "is the `.standard_errors` boilerplate, so most per-op exclusions below are "
            "over-declared codes the handler cannot answer (a SPEC BUG, #1669); the rest are "
            "rig/product (#1663, #1664):_",
            "",
        ]
        for op in sorted(BLOCKED):
            lines.append(f"- `{op}` → {_statuses(BLOCKED[op])}")
    if r.unmatched:
        lines += ["", "## ⚠️ Observed but unmatched (informational)", ""]
        for row in r.unmatched:
            lines.append(f"- `{row['method']} {row['path']}` → {_statuses(row['statuses'])}")
    viol = gate_violations(r)
    if viol:
        lines += ["", "## ❌ Gate violations (blocking)", ""]
        lines += [f"- {v}" for v in viol]
    adv = advisories(r)
    if adv:
        lines += ["", "## ⚠️ Advisories (reported, non-blocking)", ""]
        lines += [f"- {v}" for v in adv]
    return "\n".join(lines) + "\n"


def main() -> int:
    p = argparse.ArgumentParser(description="API endpoint coverage report.")
    p.add_argument("--observed", required=True, help="path to observed_endpoints.json from the suite")
    p.add_argument("--spec", required=True, help="path to the committed OpenAPI spec")
    args = p.parse_args()

    observed_path = Path(args.observed)
    if not observed_path.exists():
        print(
            f"ERROR: {observed_path} not found — the e2e suite must run first "
            f"(it writes the ledger at pytest_sessionfinish)",
            file=sys.stderr,
        )
        return 2
    observed = json.loads(observed_path.read_text(encoding="utf-8"))
    spec = json.loads(Path(args.spec).read_text(encoding="utf-8"))

    report = build_report(spec, observed)
    sys.stdout.write(render_markdown(report))
    return 0 if report.passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
