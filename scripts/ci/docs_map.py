#!/usr/bin/env python3
"""docs_map.py — documentation map generator + MANDATORY documentation gate.

Generates docs/DOCS_MAP.md: every markdown in the repo, categorized
(PRD / DESIGN / ADR / DECOMPOSITION / FEATURE / ...), per-component coverage,
and code-mapping status (registered in cypilot/config/artifacts.toml).

Gate (--check), enforced as a required CI status check:
  1. Every component under docs/components/* and docs/domain/* MUST have
     specs/PRD.md and specs/DESIGN.md.       -> no PRD+DESIGN, no pass.
  2. PRD and DESIGN MUST follow the single canonical template
     (cypilot/config/kits/sdlc/artifacts/{PRD,DESIGN}/template.md).
     Same structure everywhere — no differences.
  3. Every PRD/DESIGN/ADR must be registered in artifacts.toml (mapped to code).
  4. Exceptions only via docs/.docs-gate-waivers (reason + expiry; expired
     waivers fail the gate by themselves).

Usage: docs_map.py [--check]   (always regenerates docs/DOCS_MAP.md)
"""
import re, sys, datetime, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
DOCS = ROOT / "docs"
MAP = DOCS / "DOCS_MAP.md"
WAIVERS = DOCS / ".docs-gate-waivers"
ARTIFACTS = ROOT / "cypilot/config/artifacts.toml"

# Canonical section sets — mirror cypilot/config/kits/sdlc/artifacts/*/template.md.
PRD_SECTIONS = ["Overview", "Actors", "Operational Concept", "Scope",
                "Functional Requirements", "Non-Functional Requirements",
                "Use Cases", "Acceptance Criteria", "Dependencies", "Assumptions"]
DESIGN_SECTIONS = ["Architecture Overview", "Principles & Constraints",
                   "Technical Architecture", "Traceability"]

SCAN_GLOBS = ["docs/**/*.md", "src/**/specs/**/*.md", "src/**/README.md",
              "inbox/**/*.md", "cypilot/config/rules/*.md", "*.md"]
SKIP_PARTS = {"node_modules", ".git", "target", ".hive-tmp"}

def kind_of(p: pathlib.Path) -> str:
    n, parts = p.name.upper(), {q.upper() for q in p.parts}
    if n == "DOCS_MAP.MD": return "GENERATED"
    if n.startswith("PRD") or n.endswith("_PRD.MD") or "PRODUCT_SPECIFICATION" in n: return "PRD"
    if n.startswith("DESIGN") or n.endswith("_DESIGN.MD"): return "DESIGN"
    if "ADR" in parts or n.startswith("ADR") or re.match(r"^\d{4}-", p.name): return "ADR"
    if n.startswith("DECOMPOSITION"): return "DECOMPOSITION"
    if n.startswith("FEATURE") or "FEATURES" in parts: return "FEATURE"
    if "TEST" in n and "SCENARIO" in n: return "TEST-SCENARIOS"
    if n == "README.MD": return "README"
    if "RUNBOOK" in n: return "RUNBOOK"
    if "RULES" in parts or p.parent.name == "rules": return "RULES"
    return "OTHER"

def headings(p: pathlib.Path):
    try: text = p.read_text(errors="replace")
    except OSError: return []
    return [m.group(1).strip() for m in re.finditer(r"^##\s+(.+)$", text, re.M)]

def _norm_heading(h: str) -> str:
    # Drop a leading section number ("1. ", "2) ") and normalize whitespace/case.
    return re.sub(r"^\s*\d+[.)]?\s+", "", h.strip()).lower()

def template_ok(p: pathlib.Path, required):
    # Structural match against actual `##` headings (numbering-tolerant), NOT a
    # substring search over prose — a required section is satisfied only by a
    # real heading that equals it or extends it ("Operational Concept &
    # Environment" satisfies "Operational Concept").
    hs = [_norm_heading(h) for h in headings(p)]
    out = []
    for s in required:
        s_norm = s.strip().lower()
        if not any(h == s_norm or h.startswith(s_norm) for h in hs):
            out.append(s)
    return out

def load_waivers():
    out = {}
    if WAIVERS.exists():
        for line in WAIVERS.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"): continue
            parts = [x.strip() for x in line.split("|")]
            if len(parts) == 3: out[parts[0]] = (parts[1], parts[2])
    return out

def main():
    check = "--check" in sys.argv
    today = datetime.date.today().isoformat()
    art_reg = ARTIFACTS.read_text() if ARTIFACTS.exists() else ""
    waivers, errors, warns = load_waivers(), [], []

    files = []
    for g in SCAN_GLOBS:
        for p in ROOT.glob(g):
            if p.is_file() and not (SKIP_PARTS & set(p.parts)):
                files.append(p)
    files = sorted(set(files))
    inv = {}
    for p in files:
        inv.setdefault(kind_of(p), []).append(p.relative_to(ROOT))

    # per-component coverage
    comps = sorted([d for base in ("docs/components", "docs/domain")
                    for d in (ROOT / base).glob("*/") if d.is_dir()])
    rows = []
    for c in comps:
        rel = str(c.relative_to(ROOT)).rstrip("/")
        prd, des = c / "specs/PRD.md", c / "specs/DESIGN.md"
        adrs = len(list(c.glob("specs/ADR/*.md")))
        missing = [n for n, f in (("PRD", prd), ("DESIGN", des)) if not f.exists()]
        tmpl_bad = []
        if prd.exists():
            miss = template_ok(prd, PRD_SECTIONS)
            if miss: tmpl_bad.append(f"PRD lacks: {', '.join(miss)}")
        if des.exists():
            miss = template_ok(des, DESIGN_SECTIONS)
            if miss: tmpl_bad.append(f"DESIGN lacks: {', '.join(miss)}")
        mapped = all(str(f.relative_to(ROOT)) in art_reg for f in (prd, des) if f.exists())
        status = "✅"
        for problem in ([f"missing specs/{m}.md" for m in missing] + tmpl_bad +
                        ([] if mapped else ["not registered in artifacts.toml"])):
            key = rel
            if key in waivers:
                reason, expiry = waivers[key]
                try:
                    expired = datetime.date.fromisoformat(expiry) < datetime.date.today()
                except ValueError:
                    errors.append(f"{rel}: waiver has invalid expiry '{expiry}' (expected YYYY-MM-DD) — {problem}")
                    status = "❌"
                    continue
                if expired:
                    errors.append(f"{rel}: waiver EXPIRED ({expiry}) — {problem}")
                    status = "❌"
                else:
                    warns.append(f"{rel}: WAIVED until {expiry} ({reason}) — {problem}")
                    status = f"⚠️ waived→{expiry}"
            else:
                errors.append(f"{rel}: {problem}")
                status = "❌"
        rows.append((rel, "✅" if prd.exists() else "❌", "✅" if des.exists() else "❌",
                     adrs, "✅" if mapped else "❌", status))

    order = ["PRD", "DESIGN", "ADR", "DECOMPOSITION", "FEATURE", "TEST-SCENARIOS",
             "RULES", "RUNBOOK", "README", "OTHER", "GENERATED"]
    out = ["<!-- GENERATED by scripts/ci/docs_map.py — do not edit. Regenerate: make docs-map -->",
           "# Documentation Map", "",
           f"Total markdown files: **{len(files)}**.", "",
           "## Coverage gate — components & domains",
           "", "| Unit | PRD | DESIGN | ADRs | Mapped (artifacts.toml) | Gate |", "|---|---|---|---|---|---|"]
    out += [f"| `{r[0]}` | {r[1]} | {r[2]} | {r[3]} | {r[4]} | {r[5]} |" for r in rows]
    out += ["", "## Inventory by category", ""]
    for k in order:
        if k not in inv: continue
        out.append(f"### {k} ({len(inv[k])})\n")
        out += [f"- `{p}`" for p in inv[k]]
        out.append("")
    MAP.write_text("\n".join(out) + "\n")
    print(f"wrote {MAP.relative_to(ROOT)} ({len(files)} files, {len(rows)} units)")

    for w in warns: print(f"  WAIVED  {w}")
    if check and errors:
        print(f"\n✗ documentation gate FAILED ({len(errors)}):")
        for e in errors: print(f"  ✗ {e}")
        sys.exit(1)
    if check: print("✓ documentation gate passed")

if __name__ == "__main__":
    main()
