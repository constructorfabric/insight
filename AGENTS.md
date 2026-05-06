<!-- @cpt:root-agents -->
## ⚠️ Top project rules (do NOT skip)

**No silent defaults for config inputs.** Every value that comes from env, CLI args, K8s API, ConfigMap, Secret, or file content MUST fail-fast on missing. **Forbidden** without a `# RULE-DEFAULTS-OK: <reason>` comment on the same line:

- Bash: `${VAR:-default}`, `${VAR:=default}` — use `${VAR:?error msg}` instead.
- Python: `os.environ.get("X", default)`, `dict.get("k", default)` for required inputs — use `os.environ["X"]` / `d["k"]`.
- Helm: `{{ default "x" .Values.y }}` for required values — pair with `{{ required "msg" .Values.y }}`.
- Argo: `inputs.parameters[*].default:` for runtime values from a caller.

This is **non-negotiable**: the cost of a silent default writing data to the wrong namespace / using the wrong secret / hitting the wrong URL on prod is days of debugging. Loud failure on missing config is the only acceptable behavior. Full rationale + audit recipe in `cypilot/config/rules/code-conventions.md` (top section).

## Cypilot AI Agent Navigation

**Remember these variables while working in this project:**

```toml
cypilot_path = "cypilot"
```

## Navigation Rules

ALWAYS open and follow `{cypilot_path}/.gen/AGENTS.md` FIRST

ALWAYS open and follow `{cypilot_path}/config/AGENTS.md` FIRST

ALWAYS invoke `{cypilot_path}/.core/skills/cypilot/SKILL.md` WHEN user asks to do something with Cypilot

<!-- /@cpt:root-agents -->
