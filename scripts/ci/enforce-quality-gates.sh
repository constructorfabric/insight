#!/usr/bin/env bash
# ============================================================================
# enforce-quality-gates.sh — make the mandatory gates MECHANICALLY enforced.
#
# Configures, via the GitHub API:
#   Branch protection on `main`: all mandatory CI jobs become REQUIRED status
#   checks; PR review with CODEOWNERS is required; force-push and branch
#   deletion are disabled; stale approvals are dismissed.
#
# (Deploy-environment gating — `staging`/`production` Environments with required
# reviewer teams — is deliberately NOT configured here: those org teams and
# environments don't exist yet, and nothing deploys via GitHub Actions, so an
# Environment would gate nothing. Add it where deploys actually run if ever.)
#
# WHO RUNS THIS, AND WHEN:
#   A repository ADMIN runs it BY HAND, from their own machine — it is NOT a CI
#   job and nothing triggers it automatically. Editing branch protection needs
#   admin rights on the repo, so a normal contributor or the default CI token
#   cannot apply it. Run it once to set the rules up, and again only when the
#   required-check names change.
#
# Usage:
#   ./scripts/ci/enforce-quality-gates.sh            # DRY-RUN: just print the plan (safe, anyone)
#   ./scripts/ci/enforce-quality-gates.sh --apply    # APPLY the settings (repo admin only)
#
# Prerequisite for --apply: the `gh` CLI logged in (`gh auth login`) as a user
#   who is an admin on $REPO. Check with:
#     gh api repos/$REPO -q .permissions.admin     # must print: true
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
echo

if ! $APPLY; then
  echo "[dry-run] Would PUT branch protection on main with payload:"
  echo "$protection_payload"
  echo
  echo "[dry-run] Re-run with --apply to enforce."
  exit 0
fi

org="${REPO%%/*}"

echo "→ Applying branch protection on main…"
echo "$protection_payload" | gh api -X PUT "repos/${REPO}/branches/main/protection" --input -

echo
echo "✓ Done. Branch protection now makes review MANDATORY: 1 approving review"
echo "  from a CODEOWNER (require_code_owner_reviews), stale approvals dismissed —"
echo "  no MR merges without it. GitHub auto-requests the owning team (per"
echo "  .github/CODEOWNERS) on every PR."
echo
echo "  Manual follow-ups that the API cannot express:"
echo "  1. TWO random reviewers per PR: in each owning team's settings"
echo "     (Org → Teams → <team> → Code review assignment), enable auto-assignment,"
echo "     'Number of reviewers' = 2, routing = 'Load balance' (or 'Round robin')."
echo "     GitHub then assigns 2 random members whose code-owner approval counts."
echo "     Owning teams: * → @${org}/insight-app-maintainers ; .github/** → @${org}/security"
echo "  2. Verify the automation App retains push/bypass on main (see header)."
