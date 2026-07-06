#!/usr/bin/env python3
"""API endpoint coverage report — which analytics routes the e2e suite exercises.

Two halves:

  1. RECORDING (imported by the rig). `record_response` is an httpx response
     event-hook attached in `AnalyticsProcess.client()` — the single point
     every suite request flows through (metric tests via `call_request`, smoke
     tests directly). It records `(method, path) -> {status codes}` into a
     module-level ledger. `conftest.pytest_sessionfinish` calls `dump_observed`
     to write it to `.artifacts/observed_endpoints.json`.

  2. GATE (run as a plain file, stdlib only — blocking in `./e2e.sh gates`
     and the api-endpoint-coverage-gate CI job). `main` loads that ledger plus
     the committed OpenAPI spec (the universe — kept accurate by the analytics
     OpenAPI drift gate: the `openapi_spec_matches_committed` golden test +
     openapi-specs.yml) and reports, per documented operation, whether the
     suite exercised it and which declared status codes were validated.
     Verdict per operation is binary like the metric gate: exercised -> PASS,
     SKIP_LIST -> baseline PASS, neither -> FAIL; a skip that is now exercised
     or no longer in the spec -> FAIL (actualize). Coverage is total today
     (the api/ contract tests exercise every operation), so SKIP_LIST is
     empty — a new spec operation without a test fails this gate.

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

# Operations the e2e suite does NOT exercise, with the reason. Universe is the
# committed OpenAPI spec; anything here that the suite DOES hit (redundant) or
# that is no longer in the spec (stale) fails the gate so the list stays honest.
# Key = "METHOD path" (path verbatim from the spec, including {param} segments).
#
# EMPTY on purpose: the api/ contract tests exercise every operation in the
# spec. Add an entry only for a new operation that genuinely cannot run in the
# rig (with the reason) — and prefer a contract test instead.
SKIP_LIST: list[tuple[str, str]] = []


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
    out = Path(path)
    out.parent.mkdir(parents=True, exist_ok=True)
    rows = [
        {"method": m, "path": p, "statuses": sorted(codes)}
        for (m, p), codes in sorted(_OBSERVED.items())
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
    # Pre-split spec paths once for template matching.
    spec_paths: dict[str, list[tuple[str, list[str]]]] = {}
    for key in spec_ops:
        method, path = key.split(" ", 1)
        spec_paths.setdefault(method, []).append((path, path.strip("/").split("/")))

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

    @property
    def passed(self) -> bool:
        return not (self.missing or self.redundant_skips or self.stale_skips)


def build_report(spec: dict, observed: list[dict]) -> CoverageReport:
    spec_ops = spec_operations(spec)
    validated, unmatched = match_observed(observed, spec_ops)
    return CoverageReport(spec_ops=spec_ops, validated=validated, unmatched=unmatched, skips=skip_index())


def _statuses(codes) -> str:
    return ", ".join(str(c) for c in sorted(codes)) if codes else "—"


def gate_violations(r: CoverageReport) -> list[str]:
    out = []
    for op in r.missing:
        out.append(f"MISSING: {op} is exercised by no test and not in SKIP_LIST")
    for op in r.redundant_skips:
        out.append(f"REDUNDANT SKIP: {op} is now exercised — drop it from SKIP_LIST")
    for op in r.stale_skips:
        out.append(f"STALE SKIP: {op} is no longer in the spec — drop it from SKIP_LIST")
    return out


def render_markdown(r: CoverageReport) -> str:
    total = len(r.spec_ops)
    verdict = "✅ PASS" if r.passed else "❌ FAIL"
    lines = [
        "# API endpoint coverage — by method+path",
        "",
        f"**Gate: {verdict}.** {len(r.covered)}/{total} operations exercised "
        f"· {len(r.skipped)} baseline-skipped · **{len(r.missing)} missing**.",
        "",
        "_\"statuses\" = response codes the suite actually saw, vs the codes the spec declares._",
        "",
        "| status | method+path | statuses seen | declared |",
        "|---|---|---|---|",
    ]
    for op in sorted(r.spec_ops):
        method, path = op.split(" ", 1)
        declared = _statuses(r.spec_ops[op])
        if op in r.validated:
            mark = "✅ exercised"
            seen = _statuses(r.validated[op])
        elif op in r.skips:
            mark = f"⏭️ {r.skips[op]}"
            seen = "—"
        else:
            mark = "❌ missing"
            seen = "—"
        lines.append(f"| {mark} | `{method} {path}` | {seen} | {declared} |")
    if r.unmatched:
        lines += ["", "## ⚠️ Observed but unmatched (informational)", ""]
        for row in r.unmatched:
            lines.append(f"- `{row['method']} {row['path']}` → {_statuses(row['statuses'])}")
    viol = gate_violations(r)
    if viol:
        lines += ["", "## ❌ Gate violations — actualize SKIP_LIST", ""]
        lines += [f"- {v}" for v in viol]
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
