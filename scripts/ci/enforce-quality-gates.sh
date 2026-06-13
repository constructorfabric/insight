#!/usr/bin/env bash
# ============================================================================
# enforce-quality-gates.sh — make the mandatory gates MECHANICALLY enforced.
#
# Configures, via the GitHub API:
#   1. Branch protection on `main`: all mandatory CI jobs become REQUIRED
#      status checks; PR review with CODEOWNERS is required; force-push and
#      branch deletion are disabled; stale approvals are dismissed.
#   2. GitHub Environments `staging` and `production` with required
#      reviewers (QA lead team + release managers team) — this is the
#      "QA Delivery signed checklist" gate from the Release Process Diagram,
#      enforced in software instead of by convention.
#
# Usage:
#   ./scripts/ci/enforce-quality-gates.sh            # dry-run: print plan
#   ./scripts/ci/enforce-quality-gates.sh --apply    # actually configure
#
# Requirements: gh CLI authenticated with admin:repo scope on $REPO.
#
# IMPORTANT — release automation bypass:
#   bump-descriptors and publish-chart push directly to main using the
#   automation GitHub App (AUTOMATION_APP_ID). That App MUST remain allowed
#   to bypass branch protection or per-merge chart publishing breaks.
#   Classic branch protection cannot scope bypass to an App; verify the App
#   is installed with bypass via a repository ruleset (Settings → Rules →
#   Rulesets → bypass list) after applying this script.
# ============================================================================
set -euo pipefail

REPO="${REPO:-constructorfabric/insight}"
QA_TEAM="${QA_TEAM:-insight-qa-leads}"            # org team slug: QA leads
RM_TEAM="${RM_TEAM:-insight-release-managers}"    # org team slug: release managers
APPLY=false
[[ "${1:-}" == "--apply" ]] && APPLY=true

# Mandatory PR gates = required status checks. Names are CI job names.
REQUIRED_CHECKS=(
  "check"              # backend-checks.yml  — Rust fmt/clippy/test
  "dotnet-identity"    # backend-checks.yml  — .NET unit + integration
  "e2e"                # e2e-bronze-to-api   — bronze→dbt→ClickHouse→API
  "secrets-scan"       # security-gates.yml  — TruffleHog
  "sast"               # security-gates.yml  — Semgrep
  "deps-audit"         # security-gates.yml  — cargo-audit + pip-audit
  "trivy-config"       # security-gates.yml  — IaC/Dockerfile/fs scan
  "helm-validate"      # helm-validate.yml   — lint/template/appVersion guard
)

contexts_json=$(printf '"%s",' "${REQUIRED_CHECKS[@]}"); contexts_json="[${contexts_json%,}]"

protection_payload=$(cat <<JSON
{
  "required_status_checks": { "strict": true, "contexts": ${contexts_json} },
  "enforce_admins": false,
  "required_pull_request_reviews": {
    "dismiss_stale_reviews": true,
    "require_code_owner_reviews": true,
    "required_approving_review_count": 1
  },
  "restrictions": null,
  "allow_force_pushes": false,
  "allow_deletions": false,
  "required_conversation_resolution": true,
  "required_linear_history": false
}
JSON
)

echo "Repository:        $REPO"
echo "Required checks:   ${REQUIRED_CHECKS[*]}"
echo "Environments:      staging (reviewers: $QA_TEAM), production (reviewers: $QA_TEAM + $RM_TEAM)"
echo

if ! $APPLY; then
  echo "[dry-run] Would PUT branch protection on main with payload:"
  echo "$protection_payload"
  echo
  echo "[dry-run] Re-run with --apply to enforce."
  exit 0
fi

org="${REPO%%/*}"
team_id() { gh api "orgs/${org}/teams/$1" --jq .id; }

echo "→ Applying branch protection on main…"
echo "$protection_payload" | gh api -X PUT "repos/${REPO}/branches/main/protection" --input -

echo "→ Creating 'staging' environment (required reviewer: ${QA_TEAM})…"
gh api -X PUT "repos/${REPO}/environments/staging" --input - <<JSON
{ "reviewers": [ { "type": "Team", "id": $(team_id "$QA_TEAM") } ],
  "deployment_branch_policy": { "protected_branches": true, "custom_branch_policies": false } }
JSON

echo "→ Creating 'production' environment (required reviewers: ${QA_TEAM} + ${RM_TEAM}, 5 min wait timer)…"
gh api -X PUT "repos/${REPO}/environments/production" --input - <<JSON
{ "wait_timer": 5,
  "reviewers": [ { "type": "Team", "id": $(team_id "$QA_TEAM") },
                 { "type": "Team", "id": $(team_id "$RM_TEAM") } ],
  "deployment_branch_policy": { "protected_branches": true, "custom_branch_policies": false } }
JSON

echo
echo "✓ Done. Manual follow-ups that the API cannot fully express:"
echo "  1. Verify the automation App retains push/bypass on main (see header)."
echo "  2. In infra/insight-gitops: add the .insight-version allowlist check"
echo "     (CI job that rejects versions absent from approved-versions.txt,"
echo "     which only the production-environment approval workflow appends to)."
