#!/usr/bin/env python3
"""
Audit read-time deduplication of ReplacingMergeTree (RMT) tables across the
whole dbt project (staging + silver).

WHY
  Every staging/silver table is RMT (project default engine in profiles.yml:
  ReplacingMergeTree(_version)), and bronze tables are promoted to RMT too.
  RMT only collapses duplicates during background merges — never guaranteed at
  query time. If an upstream table holds transient pre-merge duplicates (e.g.
  an erroneous Airbyte full_refresh|append re-appending every row, as happened
  across all virtuozzo connectors), a plain SELECT leaks them downstream and
  inflates metrics. Per ADR-0001 every read of an RMT relation MUST dedup at
  read time: FINAL / argMax / QUALIFY ROW_NUMBER / LIMIT 1 BY.

  There are FOUR read seams:
    1. staging -> silver class_*   via the union_by_tag() macro
    2. class_git_* -> fct_git_*    via ref()
    3. fct_git_* -> mtr_git_*      via ref() + aggregation
    4. class_* -> class_focus_metrics via ref() + aggregation
  union_by_tag() dedups internally (QUALIFY/LIMIT 1 BY), so calling it counts as
  deduped. Direct ref()/source() reads must carry FINAL (or another dedup) in
  the read's own subquery scope.

WHAT IT FLAGS (a read of an RMT relation without read-time dedup):
  * direct ref()/source() to an RMT model/promoted-bronze table, no FINAL/etc.
  * the snapshot() macro (reads its source_ref bare — fix in the macro).

LIMITATIONS (candidate generator, confirm by reading):
  * outer-scope dedup over a wrapped subquery can read as a false positive;
  * GROUP BY is NOT counted as dedup (aggregating un-deduped rows inflates).

USAGE
  python3 audit_rmt_read_dedup.py [path-to-src/ingestion]   # default: ../ from dbt/
EXIT: non-zero if any gap is found (usable as a CI gate).
"""
import re, glob, sys, os

ROOT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..")
os.chdir(ROOT)

def strip(s):
    s = re.sub(r"\{#.*?#\}", " ", s, flags=re.S)
    s = re.sub(r"/\*.*?\*/", " ", s, flags=re.S)
    return re.sub(r"--[^\n]*", " ", s)

# --- promoted bronze tables (RMT) ---
prom = set()
for f in glob.glob("connectors/**/*.sql", recursive=True):
    for m in re.finditer(r"promote_bronze_to_rmt\(\s*table\s*=\s*['\"]([^'\"]+)['\"]", strip(open(f).read())):
        prom.add(m.group(1))

# --- map model name -> is its materialized table RMT? (default yes unless config says otherwise) ---
ALL = sorted(glob.glob("connectors/**/*.sql", recursive=True) + glob.glob("silver/**/*.sql", recursive=True))
def model_name(path): return os.path.splitext(os.path.basename(path))[0]
non_rmt = set()   # models whose engine is NOT replacing, or delete+insert (no read dedup needed)
for f in ALL:
    t = strip(open(f).read())
    eng = re.search(r"engine\s*=\s*['\"]([^'\"]+)['\"]", t)
    strat = re.search(r"incremental_strategy\s*=\s*['\"]([^'\"]+)['\"]", t)
    is_rmt = ('Replacing' in eng.group(1)) if eng else True   # project default = RMT
    if strat and strat.group(1) == 'delete+insert':
        is_rmt = False   # physically unique table; readers need no FINAL
    if not is_rmt:
        non_rmt.add(model_name(f))

DEDUP = re.compile(r"\bFINAL\b|LIMIT\s+1\s+BY|QUALIFY|ROW_NUMBER\s*\(|argMax\s*\(", re.I)
TRAIL = re.compile(r"^\s*\}*\s*(AS\s+\w+\s+|\w+\s+)?FINAL\b", re.I)
read_re = re.compile(r"(ref|source)\(\s*['\"]([^'\"]+)['\"](?:\s*,\s*['\"]([^'\"]+)['\"])?\s*\)")

def encl(txt, start):
    d = 0; i = start; o = -1
    while i > 0:
        c = txt[i]
        if c == ')': d += 1
        elif c == '(':
            if d == 0: o = i; break
            d -= 1
        i -= 1
    if o == -1: return None
    d = 0; j = o
    while j < len(txt):
        if txt[j] == '(': d += 1
        elif txt[j] == ')':
            d -= 1
            if d == 0: break
        j += 1
    return txt[o:j+1]

gaps, ok = [], []
for f in ALL:
    txt = strip(open(f).read())
    snap = re.search(r"\{\{\s*snapshot\(", txt) is not None
    if snap:
        gaps.append((f, "<source_ref via snapshot() macro>", "snapshot"))
        continue
    for m in read_re.finditer(txt):
        kind = m.group(1)
        if kind == 'source':
            target = f"{m.group(2)}.{m.group(3)}"
            is_rmt = target in prom
        else:
            target = m.group(2)
            is_rmt = target not in non_rmt   # every dbt model is RMT unless delete+insert/non-RMT
        if not is_rmt:
            continue
        sub = encl(txt, m.start())
        deduped = bool(DEDUP.search(sub)) if sub is not None else bool(TRAIL.match(txt[m.end():m.end()+40]))
        (ok if deduped else gaps).append((f, target, kind))

# --- classify each model's write strategy (for the gold-view layer check) ---
# dedup-on-write (delete+insert) or full rebuild (table) => readers need no FINAL.
# incremental + append + RMT => readers MUST use FINAL.
strategy = {}   # model name -> 'safe' | 'needs_final'
for f in ALL:
    t = strip(open(f).read())
    mat = re.search(r"materialized\s*=\s*['\"]([^'\"]+)['\"]", t)
    strat = re.search(r"incremental_strategy\s*=\s*['\"]([^'\"]+)['\"]", t)
    name = model_name(f)
    if mat and mat.group(1) in ('table', 'view', 'ephemeral'):
        strategy[name] = 'safe'
    elif strat and strat.group(1) in ('delete+insert', 'insert_overwrite'):
        strategy[name] = 'safe'
    else:
        strategy[name] = 'needs_final'   # incremental + append (or default) => RMT, needs FINAL on read

# --- gold layer: raw CREATE VIEW migrations reading silver/staging by name ---
# NOTE: migrations are immutable history — a later migration may DROP+recreate a
# view, so an older file can flag a read that no longer exists in the live view.
# Treat hits as "verify against the latest migration that (re)defines this view".
# Reads of silver tables are safe now (silver is delete+insert); a real gap here
# is a gold view reading a `staging.*` (append+RMT) table directly without FINAL.
gold_gaps = []
gold_files = sorted(glob.glob("scripts/migrations/*.sql"))
goldread = re.compile(r"\b(?:FROM|JOIN)\s+(?:silver|staging|identity|insight)\.([A-Za-z0-9_]+)\b", re.I)
for f in gold_files:
    txt = strip(open(f).read())
    for m in goldread.finditer(txt):
        tbl = m.group(1)
        if strategy.get(tbl) != 'needs_final':
            continue   # delete+insert/table/view producer, or not a dbt model (e.g. a gold view) => safe
        sub = encl(txt, m.start())
        deduped = bool(DEDUP.search(sub)) if sub is not None else bool(TRAIL.match(txt[m.end():m.end()+40]))
        if not deduped:
            gold_gaps.append((f, tbl))

def short(p): return p.replace("connectors/", "c/").replace("silver/", "s/").replace("scripts/migrations/", "m/")
print(f"RMT bronze tables promoted: {len(prom)} | dedup-on-write (delete+insert/table) models: {len(non_rmt)}")
print(f"\n### GOLD-VIEW GAPS — gold view reads an append+RMT silver/staging table without FINAL ({len(gold_gaps)})")
for f, tbl in gold_gaps:
    print(f"  {short(f):60} {tbl}  (producer is incremental+append → needs FINAL or make it delete+insert)")
print(f"\n### GAPS — RMT read without read-time dedup ({len(gaps)})")
for f, tgt, k in gaps:
    print(f"  {short(f):60} <{k}> {tgt}")
print(f"\n### OK — deduped RMT reads: {len(ok)} (union_by_tag calls dedup internally and are not listed)")
sys.exit(1 if (gaps or gold_gaps) else 0)
