---
cf: true
type: project-rule
topic: code-conventions
version: 1.0
---

# Code Conventions

Hard rules for writing or reviewing imperative code in this project — shell scripts, Python helpers, Argo Workflow / Kubernetes YAML, dbt macros, deploy scripts. **All rules are MUST**, not SHOULD; violations block merge.

The point of these rules is to keep the source of truth in **one place per concept** so that when something changes, exactly one file moves. Defaults, inline copies, and hidden fallbacks fight that goal.

<!-- toc -->

- [No default values](#no-default-values)
- [No inline scripts](#no-inline-scripts)
- [No inline YAML](#no-inline-yaml)
- [Fail-fast over silent fallback](#fail-fast-over-silent-fallback)
- [Audit recipe](#audit-recipe)

<!-- /toc -->

## No default values

> **THIS IS THE #1 RULE. AI agents that generate code MUST follow this without exception.**
>
> Every value that comes from outside the script — env var, CLI arg, K8s API response, ConfigMap, Secret annotation, file content — **MUST fail-fast on missing**. No silent fallback to a hardcoded default. **Including** "obvious" defaults like `:-insight` for namespace, `:-localhost:8001` for URL, `:-airbyte-auth-secrets` for secret name.
>
> **The bar is**: if I can `unset VAR && bash script.sh` and it runs to completion using a wired-in default value, the script is broken — even if the default happened to be correct.

**Forbidden** in any imperative code path:

- Bash: `${VAR:-default}`, `${VAR:=default}`. Only `${VAR:?error message}` is permitted (the `:?` form aborts and prints the message — that IS the fail-fast pattern).
- Python (helpers, scripts): `os.environ.get("KEY", default_value)`, `dict.get(key, default_value)` where the value is required input (config, K8s state, secret data). Use `os.environ["KEY"]` (raises `KeyError`) or explicit `if key not in d: sys.exit(...)`.
- Argo Workflow `inputs.parameters[*].default:` for runtime values from a caller (connection IDs, source IDs, image tags). Only chart-rendered constants resolved at install time (e.g. `clickhouse_host` from Helm `include`) may use `default:`.
- Helm chart `default` values for required operator inputs — pair with `required "msg"` so missing config fires an explicit error.

**Why** (read once, then internalize): a silent default moves the source of truth from one place (the caller / config) into N places (every default site). The damage modes:
- Operator forgets to set `INSIGHT_NAMESPACE=prod-tenant`, code falls through to `:-insight`, **writes to the wrong cluster's wrong namespace**, data lands in the wrong tenant. No error. Discovered weeks later.
- Admin renames `airbyte-auth-secrets` to a per-tenant name, code keeps reading the old name from its default, silently uses a missing/stale secret. No error. Discovered when an audit fails.
- "Works on dev, fails on prod" debug loops eat days because the failure mode is wrong-data, not crash.

Fail-fast crashes are **loud, immediate, and tell the operator exactly what to fix in the first second**. Default values are landmines.

**Before generating ANY line that touches a config input**, ask:
1. Where does this value come from outside the script?
2. What happens if `unset VAR && script.sh` is run? — if the answer is "uses my default", you have a bug.
3. Is silent fallback (running with a wrong value) ever better than a loud error? — almost never.

If you cannot articulate a context where the silent default is **provably correct for every operator who will ever run this code**, drop it.

**Allowed exceptions** — narrowly: defaults are permitted ONLY when:
- The value has no other source AND the developer **explicitly states in the prompt** that the default is correct for the named context, OR
- The default is a chart-rendered constant resolved at Helm install time (operator sets it once via `values.yaml`), OR
- The value is purely UX/cosmetic (log timestamp format, retry backoff units, output verbosity) — not a config input.

**Every exception MUST be tagged with `# RULE-DEFAULTS-OK: <one-line reason>` on the line itself.** Untagged defaults block merge.

**How to apply**:
- Shell: `: "${VAR:?VAR must be set (e.g. <example>; see <docs path>)}"` — at the top of every script that uses `$VAR`.
- Python: `os.environ["VAR"]` (raises `KeyError`), not `os.environ.get(...)`. For dicts, `d["k"]` not `d.get("k", default)`.
- Argo: omit `default:` for runtime parameters. The submitter (CronWorkflow / run-sync.sh / api-gateway) must pass every value.
- Helm: `{{ required "X is required when Y=true" .Values.x }}` — pair `required` with `default` if you also want a fallback for missing-but-Y-false case.
- **Do NOT** write `${INSIGHT_NAMESPACE:-insight}`, `${SECRET_NAME:-airbyte-auth-secrets}`, `${AIRBYTE_URL:=http://localhost:8001}`, `os.environ.get("FOO", "bar")` without the explicit `# RULE-DEFAULTS-OK:` tag justifying the named default. **An untagged default is a bug, even if the value seems "obvious".**

## No inline scripts

**Forbidden**: embedding a non-trivial Python (or other-language) script inside a heredoc inside a shell script.

```bash
# BAD
SOURCE_ID=$(kubectl get secret -o json | python3 -c "
import json, os, sys
...")

# OK (1) — pure shell + jq
SOURCE_ID=$(kubectl get secret -o json | jq -r '.items[0].metadata.name')

# OK (2) — Python in its own file
SOURCE_ID=$(kubectl get secret -o json | python3 src/ingestion/scripts/resolve_source_id.py)
```

**Threshold**: any heredoc Python with imports beyond `sys` is "non-trivial". One-liner string manipulation is OK; anything that reads structured data, applies multi-step logic, or has error handling goes in a `.py` file.

**Why**:
- Embedded scripts can't be linted, type-checked, or unit-tested.
- They share `${VAR}` substitution between the parent shell and the child interpreter — a recurring source of injection bugs (parent shell expands `$x` before Python sees it).
- They duplicate when the same logic appears in two shell scripts (resolve-source-id was on its way to becoming a duplicate before this rule).

**Preferred order**:
1. Pure shell + `jq` for JSON, `yq` for YAML, `awk`/`sed` for text. Almost everything is reachable this way.
2. Standalone `.py` (or `.sh`) file in `src/ingestion/scripts/` (or appropriate location), called as a subprocess.
3. Inline only when the logic is one expression and won't be reused.

## No inline YAML

**Forbidden**: rendering Argo `Workflow` / `WorkflowTemplate` / `CronWorkflow` / `Service` / any K8s object as a heredoc inside a shell script.

```bash
# BAD
kubectl create -f - <<EOF
apiVersion: argoproj.io/v1alpha1
kind: Workflow
metadata:
  generateName: ${CONNECTOR}-
...
EOF

# OK
envsubst < src/ingestion/workflows/onetime/run-sync.yaml.tpl | kubectl create -f -
```

**Why**:
- Inline YAML can't be validated by `kubeval` / `helm lint` / IDE schema tools.
- It diverges from the WorkflowTemplate it shadows (we now have ingestion-pipeline as a chart-rendered template **and** an inline copy in run-sync.sh — when the template grows a parameter, the inline copy silently keeps the old shape).
- It can't be diff-reviewed cleanly: changes show up as shell-string edits, not YAML edits.

**Where to put extracted templates**:
- One-shot `Workflow` submissions → `src/ingestion/workflows/onetime/<name>.yaml.tpl`
- `CronWorkflow` schedules → `src/ingestion/workflows/schedules/<name>.yaml.tpl` (existing pattern — `sync.yaml.tpl`)
- Reusable `WorkflowTemplate`s → `charts/insight/templates/ingestion/<name>.yaml` (chart-rendered; no envsubst, uses Helm templating)

**Renderer**:
- `envsubst` for shell-driven templates with `${VAR}` placeholders. Document expected variables at the top of the template.
- Helm for chart-rendered templates that ship inside the umbrella.

## Fail-fast over silent fallback

When a configuration value, secret annotation, or input is missing, **error explicitly** with a message pointing at how to fix it. Do not silently:

- Match an unannotated Secret to the requested tenant (cross-tenant misresolution).
- Substitute an empty string for a missing `${VAR}`.
- Assume a default region / namespace / image tag.

The first time someone deploys with a missing value, they should see a clear "set X" error — not a successful run that silently does the wrong thing on a different tenant's data.

**Apply this every time** you're tempted to write `or default`, `?:`, `try/except: pass`, or `coalesce(x, y)` — those are the common shapes of silent fallback. Sometimes they're correct; usually they hide a bug.

## Audit recipe

Run this scan on the files your PR touches before requesting review. Each match must either be fixed (replaced with a required env var, extracted to a file, etc.) **or** tagged with a `# RULE-DEFAULTS-OK: <reason>` comment that names the reason.

```bash
# Files touched by the current branch (vs main).
FILES=$(git diff --name-only main...HEAD)

# 1. Bash defaults — `${VAR:-default}`, excluding the abort-form `${VAR:?...}`
echo "$FILES" | xargs -r grep -nE '\$\{[A-Z_][A-Z_0-9]*:-[^?}]' 2>/dev/null

# 2. Python config defaults — non-trivial fallbacks via `.get(k, v)`
echo "$FILES" | xargs -r grep -nE 'os\.environ\.get\([^)]+,\s*[^)]+\)' 2>/dev/null
echo "$FILES" | xargs -r grep -nE '\.get\([^)]+,\s*['"'"'"`]' 2>/dev/null

# 3. Argo / K8s YAML — `default:` lines in chart and workflow templates
echo "$FILES" | grep -E '\.(ya?ml)$' | xargs -r grep -nE '^\s+default:' 2>/dev/null

# 4. Inline Python / YAML inside shell — heredoc and `python3 -c "$"` patterns
echo "$FILES" | grep -E '\.sh$' | xargs -r grep -lE 'python3 -c "$|<<EOF$' 2>/dev/null
```

The first three categories surface defaults; the fourth surfaces inline-script / inline-YAML violations. A clean run produces only `RULE-DEFAULTS-OK`-tagged matches and lines from this rule document itself.
