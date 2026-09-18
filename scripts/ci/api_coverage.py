#!/usr/bin/env python3
"""Endpoint coverage for the authenticator: which documented routes its e2e suite exercises.

The ledger is written by the Rust harness (`src/backend/services/authenticator/tests/`,
`common/mod.rs`) as `{method, path, statuses}` rows; the universe is the committed OpenAPI
document, kept fresh by the service's drift test. This half is pure and stdlib-only.

The gate FAILS when a documented operation is exercised by no test, or when a SKIP_LIST
entry rots. Per-status-code coverage is REPORTED, not enforced: each declared code is
`✓` observed / `✗` unobserved / `·` excluded (5xx or BLOCKED). Excluded-set hygiene is a
non-blocking advisory.

    python3 scripts/ci/api_coverage.py --observed observed_authenticator_endpoints.json \
        --spec docs/components/backend/authenticator/openapi.json
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import sys
from pathlib import Path

_HTTP_METHODS = ("get", "put", "post", "delete", "patch", "head", "options", "trace")

# Operations no test exercises, "METHOD path" -> reason. A listed op the suite DOES hit
# (redundant) or that left the spec (stale) fails the gate.
SKIP_LIST: list[tuple[str, str]] = [
    (
        "DELETE /auth/admin/users/{person_id}/sessions",
        "needs the gateway-JWT authn pipeline (TLS discovery front); exercised "
        "in the gateway compose e2e instead (see e2e_sessions.rs)",
    )
]

# Server-fault codes (>= this) are declared for spec fidelity but never required: a
# black-box contract test can't deterministically induce a 500.
SERVER_FAULT_FLOOR = 500

# Declared codes the suite cannot observe, subtracted from `required`. Self-cleaning: an
# entry that becomes observed or leaves the spec fails the hygiene advisory.
# back-channel-logout's 200 answers the IdP's server-side POST (covered in
# e2e_backchannel), never the test client.
BLOCKED: dict[str, frozenset[int]] = {"POST /auth/oidc/back-channel-logout": frozenset({200})}


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
            codes = sorted(int(c) for c in (op.get("responses") or {}) if str(c).isdigit())
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
            if all(t.startswith("{") and t.endswith("}") or t == o for t, o in zip(tmpl_segs, obs_segs)):
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
        for op in ops:
            now_seen = set(BLOCKED.get(op, frozenset())) & self.validated.get(op, set())
            if now_seen:
                self.blocked_observed[op] = now_seen
        self.covered_codes: dict[str, set[int]] = {op: self.required[op] & self.validated.get(op, set()) for op in ops}
        self.total_coverable = sum(len(c) for c in self.required.values())
        self.total_covered = sum(len(c) for c in self.covered_codes.values())
        self.coverage_pct = (
            100.0 if self.total_coverable == 0 else round(100.0 * self.total_covered / self.total_coverable, 1)
        )

    def required_codes(self, op: str) -> set[int]:
        """Declared codes the suite must observe: drop server-fault 5xx and the per-op
        BLOCKED set. May be empty, in which case the op contributes nothing to the
        coverage % and passes once merely exercised."""
        declared = self.spec_ops.get(op, [])
        return {c for c in declared if c < SERVER_FAULT_FLOOR} - set(BLOCKED.get(op, frozenset()))

    @property
    def passed(self) -> bool:
        return not gate_violations(self)


def build_report(spec: dict, observed: list[dict]) -> CoverageReport:
    spec_ops = spec_operations(spec)
    validated, unmatched = match_observed(observed, spec_ops)
    return CoverageReport(spec_ops=spec_ops, validated=validated, unmatched=unmatched, skips=skip_index())


def _statuses(codes) -> str:
    return ", ".join(str(c) for c in sorted(codes)) if codes else "—"


def gate_violations(r: CoverageReport) -> list[str]:
    """BLOCKING findings — a non-empty list fails the gate (exit 1): a documented
    operation no test exercises, or SKIP_LIST rot."""
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
    all_codes = sorted({c for codes in r.spec_ops.values() for c in codes})
    lines = [
        "# API endpoint coverage — by method+path",
        "",
        f"**Gate: {verdict}.** {len(r.covered)}/{total} operations exercised "
        f"· **{len(r.missing)} missing** (a documented operation no test exercises) "
        f"· registered-code coverage **{r.coverage_pct}%** "
        f"({r.total_covered}/{r.total_coverable} coverable codes seen).",
        "",
        "_The gate blocks on a missing operation or SKIP_LIST rot (a new endpoint without a "
        "test). Per-status-code coverage below is REPORTED, not enforced: "
        "`✓` observed · `✗` declared but not yet observed · `·` excluded (5xx / BLOCKED) · "
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
            "_Server-fault 5xx is excluded on every route. Per-op exclusions below are "
            "declared codes the suite cannot observe; each entry's rationale lives beside it "
            "in the BLOCKED table:_",
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
    p = argparse.ArgumentParser(description="Authenticator endpoint coverage report.")
    p.add_argument("--observed", required=True, help="path to observed_endpoints.json from the suite")
    p.add_argument("--spec", required=True, help="path to the committed OpenAPI spec")
    args = p.parse_args()

    observed_path = Path(args.observed)
    if not observed_path.exists():
        print(  # noqa: T201 — CLI diagnostic on stderr
            f"ERROR: {observed_path} not found — the authenticator e2e suite must run "
            f"first (it writes the ledger as it makes requests)",
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
