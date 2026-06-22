#!/usr/bin/env bash
#
# doctor.sh — verify the local toolchain meets the documented minimums.
# Exits non-zero on the first failure with an installation hint.

set -euo pipefail

C_RED=$(tput setaf 1 2>/dev/null || echo "")
C_GRN=$(tput setaf 2 2>/dev/null || echo "")
C_YEL=$(tput setaf 3 2>/dev/null || echo "")
C_RST=$(tput sgr0   2>/dev/null || echo "")

_fail=0
_warn=0

check() {
  local name="$1" cmd="$2" min="$3" hint="$4" version_cmd="$5"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "${C_RED}MISSING${C_RST} $name — install: $hint"
    _fail=$((_fail + 1))
    return
  fi
  local got
  got="$(eval "$version_cmd" 2>/dev/null | head -n1 || echo "")"
  echo "${C_GRN}OK${C_RST}      $name ${got:-(version unknown)} (need $min+)"
}

echo "Tooling:"
check helm     helm     "3.14"  "brew install helm"            'helm version --short | sed s/^v//'

# Refuse helm v4.2.1 — known regression where `--wait` hangs the full
# --timeout on fast hook-resource deletions, turning every
# `before-hook-creation` lifecycle into a 5–10 minute stall. Affects
# every bootstrap-* / system-* step. See helm/helm#32214 (regression)
# and helm/helm#32230 (proposed revert). Pin to v4.2.0 or v3.x until a
# v4.2.2+ release lands the fix.
if command -v helm >/dev/null 2>&1; then
  _helm_ver="$(helm version --short 2>/dev/null | sed 's/+.*//; s/^v//')"
  if [ "$_helm_ver" = "4.2.1" ]; then
    echo "${C_RED}BAD${C_RST}     helm $_helm_ver — known --wait regression"
    echo "          https://github.com/helm/helm/issues/32214"
    echo "          Pin to v4.2.0: curl -fL https://get.helm.sh/helm-v4.2.0-\$(uname -s | tr A-Z a-z)-\$(uname -m | sed s/x86_64/amd64/).tar.gz | tar -xz"
    echo "          Or v3.21.1: brew install helm@3 (and brew link --overwrite helm@3)"
    _fail=$((_fail + 1))
  fi
fi

check kubectl  kubectl  "1.27"  "brew install kubectl"          'kubectl version --client -o json | jq -r .clientVersion.gitVersion'
check kubeseal kubeseal "0.27"  "brew install kubeseal"         'kubeseal --version'
check skopeo   skopeo   "1.14"  "brew install skopeo"           'skopeo --version'
check yq       yq       "4.x"   "brew install yq"               'yq --version'
check jq       jq       "1.6"   "brew install jq"               'jq --version'
check git      git      "2.40"  "system / brew install git"     'git --version'
check make     make     "3.81"  "system / brew install make"    'make --version | head -n1'

echo
echo "Secret backend:"
# `make seal-secret` shells out to scripts/secret-fetch.sh. The sample stub
# reads from a local YAML; replace it with your own password-manager
# integration. We can't validate the backend generically — print a note.
if [ -x scripts/secret-fetch.sh ]; then
  echo "${C_GRN}OK${C_RST}      scripts/secret-fetch.sh present (review it for your backend before running 'make seal-secret')"
else
  echo "${C_YEL}NOTE${C_RST}    scripts/secret-fetch.sh missing or not executable — chmod +x once you adapt the stub"
  _warn=$((_warn + 1))
fi

echo
echo "Optional:"
if command -v gitleaks >/dev/null 2>&1; then
  echo "${C_GRN}OK${C_RST}      gitleaks present — pre-commit hook can run"
else
  echo "${C_YEL}NOTE${C_RST}    gitleaks absent — install for pre-commit secret scanning"
  _warn=$((_warn + 1))
fi

echo
if [ "$_fail" -gt 0 ]; then
  echo "${C_RED}${_fail} required tool(s) missing.${C_RST}"
  exit 1
fi

echo "${C_GRN}all required tooling present${C_RST}${_warn:+ (with $_warn warnings)}"
