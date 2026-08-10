#!/usr/bin/env bash
#
# emulate-ci-deploy.sh — run, from a laptop, the same three stages that
# .github/workflows/deploy-test-stand.yml runs after the umbrella chart is
# published: deploy -> seed -> smoke.
#
# WHY THIS EXISTS
#
# The test-stand workflow only ever fires on a merge to main. That makes it the
# worst possible place to find out that the gitops environment is wrong: the
# feedback loop is "merge, wait, read a redacted public log, guess, merge
# again", and every iteration leaves a red X on somebody's merge commit. This
# script closes that loop before the workflow is ever enabled — and keeps it
# closed afterwards, because when CI does go red the first question is always
# "does it reproduce by hand?" and the only answer worth anything is one
# produced by the SAME commands.
#
# So the contract of this file is narrow and deliberate: it does not invent a
# deploy path. Each stage runs the invocation the workflow's matching step runs,
# prints it before running it, and refuses to run any of them against the wrong
# cluster. Everything else — richer output, retries, cleanup, convenience
# defaults CI does not have — was considered and rejected: every divergence
# between this script and the workflow is a way for a green local run to lie.
#
# THE THREE STAGES, AND WHERE THEY COME FROM
#
#   1. deploy   `helm upgrade --install` spelled out, with the OIDC client
#               secret read out of the cluster, `--wait`, `--timeout 10m`, and
#               deliberately no `--atomic`; then three checks the upgrade's exit
#               status cannot answer — that the live release is the chart this
#               run asked for, that the three envFrom-configured services were
#               restarted, and that the Gateway ACCEPTED the two edge routes the
#               release just rendered.
#               NOT `make deploy`: `deploy-insight` chains apply-app-secrets,
#               which hard-requires a sealed manifest this stand has no
#               controller for and then rewrites the chart's config Secrets from
#               a key name this stand does not use, and it hardcodes `--atomic`
#               after $(HELM_UPGRADE_FLAGS) so no knob turns the rollback off.
#               `make diff` DOES work here unchanged, which is why it is this
#               stage's read-only form.
#   2. seed     `seed-stand.sh -n insight --context … --days 730`,
#               verbatim. No wrapper: the seeder discovers the tenant, the
#               datastore coordinates, the IdP source type (there is no
#               --auth-mode any more — it was folded into --idp-source-type,
#               which is itself read off the release) and the dev-lead address
#               from the cluster, and a value supplied from outside is a value
#               that can be wrong while looking right.
#
#               The dev-lead address is the newest member of that list and the
#               one that proves the rule. It used to be supplied — by a GitHub
#               secret in CI, by this script's own committed default locally —
#               while the realm that decides who can actually log in was written
#               by the deploy. Two writers for one roster, no check between
#               them, and they drifted: the realm named one person, the seed
#               another, and the login resolved to nobody. The seeder now reads
#               the address out of the realm ConfigMap the stand applied, so
#               this harness stops supplying one too. `--seed-email` survives as
#               exactly the override CI's secret is — which is the point, since
#               a rehearsal that took the old path would prove the wrong thing.
#   3. smoke    `uv run --project tests --frozen pytest tests/stand/smoke
#               --stand-manifest <captured>` against the public URL. Real DNS,
#               real TLS, real IdP redirect — a stand that works only from
#               inside the cluster is a stand nobody can use.
#
# READ-ONLY BY DEFAULT
#
# The target is a published stand that other people look at. A script that
# deploys by default is a script that deploys by accident, so the default mode
# renders and reports and touches nothing:
#
#   deploy  ->  `make diff` (helm template plus a diff against the last render;
#               contacts the OCI registry and the local git tree, never the
#               cluster), then read-only checks of the two things a render can
#               never answer: that the OIDC client secret the upgrade injects
#               really is in the cluster, and that the Gateway has accepted the
#               edge routes the release renders.
#   seed    ->  `seed-stand.sh --dry-run`, which performs the SAME cluster
#               discovery the real run performs and prints the Job it would
#               apply. Read-only, and a genuine RBAC probe.
#   smoke   ->  one unauthenticated GET of the public URL, then
#               `pytest --collect-only`.
#
# Mutating the stand requires BOTH `--apply` AND `--i-know-this-deploys
# yes-deploy-<env>`. Two flags rather than one because a single `--apply` is
# exactly the kind of thing that ends up in a shell history and gets recalled
# with the wrong `--kubeconfig` still on the line. The token spells out the
# environment, so recalling that line for a different stand fails closed. It is
# deliberately the same string the Makefile's protected-environment CONFIRM=
# uses, so there is one token to remember rather than two.
#
# THE CLUSTER GUARD
#
# `--kubeconfig` and `--expect-cluster` are both required, in every mode,
# including dry-run. The guard is the whole point of the file: this
# organisation runs more than one Insight stand, at least one of which must
# never be touched by this tooling, and a kube-context name is a local alias
# anybody can typo into agreement. So before anything runs we assert, in order,
# all read-only:
#
#   1. the gitops inventory's `kubeContext` equals the kubeconfig's
#      current-context — the same assertion the workflow makes. It matters more
#      than it looks: nothing on this path passes helm an explicit
#      `--kube-context`, so the ambient context IS the target;
#   2. the cluster ENTRY that context points at is named exactly
#      `--expect-cluster`;
#   3. optionally, that the sha256 of that cluster's API server URL equals
#      `--expect-api-server-sha256`.
#
# (3) is optional but is the only check that binds to the cluster rather than to
# names in a file, which is why `--apply` without it prints a warning naming the
# residual risk. It takes a digest rather than the URL itself so the value is
# safe to keep in a GitHub environment variable or a team note: this repo is
# public and a cluster API endpoint is not ours to publish.
#
# WHAT THIS SCRIPT DELIBERATELY DOES NOT DO
#
#   * It does not create, seal, rotate, or print any credential. The OIDC client
#     secret is read out of the cluster into a variable that exists for the
#     length of one helm call; the printed command shows the substitution
#     expression, never its result. Every SMOKE_* login variable arrives from the
#     caller's environment untouched, and the smoke suite validates them itself.
#   * It does not roll back, retry, or clean up. A failed deploy is left exactly
#     where it failed so the evidence survives; recovery is the next merge or a
#     deliberate `make rollback ENV=test-stand`.
#   * It does not apply the HTTPRoutes, and there is nothing left to apply: the
#     umbrella renders both of them from the environment's `gateway.route` and
#     `keycloak.route`, so they arrive with the release and the release owns
#     them. All this harness does with them is read the `Accepted` condition
#     back — which is worth doing on its own account, because `helm --wait`
#     waits on workloads and not on HTTPRoute status, so a route the Gateway
#     refuses leaves the release reporting `deployed` and the stand dark.
#   * It does not print its own failure diagnostics. Those belong to the
#     workflow's stand-diagnostics.sh, a curated redacted allowlist, and reusing
#     it by path is what keeps a laptop run from showing evidence the red CI run
#     will not have.
#   * It does not pipe stage output through redact-stand-log.py. CI must, because
#     its console is public forever; a laptop's is not, and redacting the one copy
#     you are debugging from removes the detail you are debugging. This is one of
#     the deliberate differences from CI — all of them are enumerated in
#     docs/components/deployment/gitops/ci-emulation.md.
#
# Usage: emulate-ci-deploy.sh --kubeconfig PATH --expect-cluster NAME [options]
# Run with --help for the option list, or --print-commands to read the whole
# plan without running anything.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GITOPS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$GITOPS_DIR/../.." && pwd)"

# The gitops environment this harness drives. Overridable so the same script can
# rehearse a second stand later, but the default is the one the workflow
# hardcodes — a harness whose default target differs from CI's rehearses the
# wrong thing.
GITOPS_ENV="test-stand"

WORKFLOW_REL=".github/workflows/deploy-test-stand.yml"
WORKFLOW_FILE="$REPO_ROOT/$WORKFLOW_REL"
SEED_REL="./src/ingestion/tools/seed/seed-stand.sh"
SEED_SCRIPT="$REPO_ROOT/src/ingestion/tools/seed/seed-stand.sh"
SMOKE_SUITE="tests/stand/smoke"

# The umbrella chart, spelled exactly as the workflow's CHART_REF spells it.
CHART_REF="oci://ghcr.io/constructorfabric/charts/insight"

# Failure diagnostics belong to the workflow. Overridable with --diagnostics or
# $INSIGHT_STAND_DIAGNOSTICS, but there is one canonical home and probing for
# others would only hide a rename.
DIAGNOSTICS_DEFAULT_REL=".github/workflows/scripts/stand-diagnostics.sh"

# Artefacts land under the gitops .deploy/ directory, which deploy/gitops/
# .gitignore already excludes. That is not tidiness: `make diff` depends on
# `sync-clean`, which fails on ANY file `git status --porcelain` reports, so a
# harness that wrote its captured seed log into the working tree would break the
# very stage it is trying to rehearse. The same reasoning is why --kubeconfig is
# refused when it points inside the tree.
ARTIFACT_DIR="$GITOPS_DIR/.deploy/ci-emulation"

# There is deliberately NO default dev-lead address here any more.
#
# There used to be one — the seeder's committed canonical value, on a
# non-routable domain, justified in this very comment as "a safe default that
# addresses the person the roster describes". It was not safe and it did not
# address that person: the realm applied to a stand is what decides who can log
# in, and a committed constant here is a guess about a value another repository's
# bring-up wrote. CI made the same guess from a GitHub secret, the two guesses
# disagreed with the realm, and a stand came up whose dev-lead authenticated and
# then resolved to nobody.
#
# Removing the default is what makes this harness a rehearsal again rather than a
# demonstration of the old path: with nothing to fall back to, the seeder
# discovers the address from the realm exactly as it does in CI, and the
# --dry-run mode exercises that discovery before any apply. $TEST_STAND_SEED_EMAIL
# is still honoured, because the same-named secret is still honoured in CI and
# parity is the whole product here — but an unset variable now means "let the
# seeder read the stand", not "use the committed guess".

# The seed window. `--days`, NOT `--window-days`: seed-stand.sh's argument parser
# has no such flag and refuses unknown arguments, so the wrong spelling aborts
# the run before a single row is written.
SEED_WINDOW_DAYS="730"

# helm's own budget for the upgrade, matching the workflow. Not the Makefile's
# 30m default: a deploy that is going to fail should say so inside the stage
# budget.
DEPLOY_TIMEOUT="10m"

# The three deployments whose configuration arrives through `envFrom` and is
# therefore read exactly once, at container start. Each subchart's
# `checksum/config` annotation hashes its OWN ConfigMap and not the
# umbrella-rendered insight-*-config Secrets, so without the restart a changed
# host, tenant or client secret produces a `deployed` release with all-Ready pods
# running stale configuration. All three on purpose: a list of two leaves one
# service holding yesterday's configuration with nothing to show for it.
RESTART_TARGETS="deploy/insight-authenticator deploy/insight-analytics deploy/insight-identity-resolution"

# The two edge routes the chart renders from the environment's `gateway.route`
# and `keycloak.route`. The release owns them, so a deploy is what writes them;
# what gets checked afterwards is whether the Gateway ACCEPTED them, which helm
# does not wait for. Every acceptance criterion travels through both.
ROUTE_NAMES="insight-gateway insight-keycloak"

KUBECONFIG_PATH=""
EXPECT_CLUSTER=""
ALLOW_DIRTY=0
EXPECT_API_SHA=""
AS_USER=""
STAGE="all"
CHART_VERSION=""
BASE_URL=""
SEED_DEV_EMAIL=""
MANIFEST_PATH=""
DIAGNOSTICS_SCRIPT="${INSIGHT_STAND_DIAGNOSTICS:-}"
APPLY=0
CONFIRM_TOKEN=""
PRINT_ONLY=0

usage() {
  cat <<'USAGE'
Usage: emulate-ci-deploy.sh --kubeconfig PATH --expect-cluster NAME [options]

Runs the three stages of .github/workflows/deploy-test-stand.yml from a laptop,
against a stand, through an explicit kubeconfig. Read-only unless --apply.

Required:
      --kubeconfig <path>   kubeconfig FILE to act through. Deliberately not the
                            ambient $KUBECONFIG: the guard rests on this being a
                            value you typed for this run. Must live OUTSIDE the
                            repository working tree — `make diff` depends on
                            sync-clean.
      --expect-cluster <name>
                            the kubeconfig CLUSTER entry the current context
                            must point at. Refuses loudly on mismatch.

Safety:
      --apply               actually mutate the stand. Without it, every stage
                            runs its read-only form.
      --allow-dirty         read-only deploy stage only: when the working tree is
                            dirty, render with `helm template` through the same
                            chart/version/values instead of `make diff`, which
                            refuses any uncommitted file. For rehearsing changes
                            that are not committed yet — a real deploy always
                            renders from a clean checkout, and the run says so.
      --i-know-this-deploys <token>
                            required with --apply. Must be exactly
                            yes-deploy-<env> (default: yes-deploy-test-stand).
      --expect-api-server-sha256 <hex>
                            optional, recommended with --apply: sha256 of the
                            cluster's API server URL. The only check that binds
                            to the cluster rather than to names in a file.
      --as-user <name>      run the read-only RBAC probe as this user (e.g. the
                            CI ServiceAccount) instead of as you. Probe only:
                            the stages always run as the kubeconfig's identity.

Selection:
      --stage <s>           deploy | seed | smoke | all             [default: all]
      --env <name>          gitops environment directory            [default: test-stand]
      --chart-version <v>   umbrella version to deploy — the value CI hands over
                            from publish-chart. ALWAYS pass it explicitly with
                            --apply: the default lags the stand by construction
                            and an omitted flag is a silent DOWNGRADE.
                            [default: deploy/gitops/.insight-version]
      --timeout <dur>       helm --timeout for the upgrade           [default: 10m]

Seed and smoke inputs:
      --seed-email <addr>   OVERRIDE the dev-lead address that the seeder would
                            otherwise read out of the realm this stand applied.
                            Unset is the normal case and the one CI runs: the
                            realm says who the dev-lead is and the seed follows
                            it, so the two cannot end up describing different
                            people. Pass this only for a stand whose realm the
                            seeder cannot read — and know that naming anyone but
                            the realm's dev-lead seeds a person nobody can log
                            in as, with every stage still green.
                            [default: $TEST_STAND_SEED_EMAIL, else discovered
                            from the stand]
      --manifest <path>     seed manifest for pytest. [default: captured out of
                            the seed stage into the artefact directory]
      --base-url <url>      the stand's public URL. [default: $SMOKE_BASE_URL,
                            else derived from the committed values.yaml]

Other:
      --diagnostics <path>  failure-diagnostics script to reuse.
                            [default: .github/workflows/scripts/stand-diagnostics.sh]
      --print-commands      print the whole plan and exit, running nothing — not
                            even the cluster guard.
  -h, --help                this text

Credentials this script never supplies and never prints. Export them yourself;
in CI they come from the insight-test-stand GitHub environment:
      SMOKE_LOGIN_MODE                password | override
      SMOKE_PERSONA_PASSWORD          password mode
                                      (SMOKE_PERSONA_PASSWORD__<FIXTURE> overrides one persona)
      SMOKE_BOOTSTRAP_EMAIL
      SMOKE_BOOTSTRAP_PASSWORD        override mode
The smoke suite resolves and validates these itself and names the missing one.

Examples:
  # read the plan without touching anything
  emulate-ci-deploy.sh --print-commands

  # read-only rehearsal of everything against the live stand
  emulate-ci-deploy.sh --kubeconfig ~/.kube/stand.yaml --expect-cluster my-cluster

  # rehearse the seed stage's discovery as the CI ServiceAccount
  emulate-ci-deploy.sh --kubeconfig ~/.kube/stand.yaml --expect-cluster my-cluster \
    --stage seed --as-user system:serviceaccount:insight:ci-deployer

  # the real thing, as CI would run it for a freshly published chart
  emulate-ci-deploy.sh --kubeconfig ~/.kube/stand.yaml --expect-cluster my-cluster \
    --chart-version 0.5.101 --apply --i-know-this-deploys yes-deploy-test-stand
USAGE
}

die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

# Distinct from die() on purpose: a guard rejection is not a bug in the run, it
# is the guard doing its job, and it must be impossible to miss in a scrollback
# full of helm output.
refuse() {
  printf '\n' >&2
  printf '%s\n' '================================================================' >&2
  printf 'REFUSING TO ACT: %s\n' "$1" >&2
  shift
  while [ $# -gt 0 ]; do
    printf '  %s\n' "$1" >&2
    shift
  done
  printf '%s\n' '================================================================' >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 \
    || die "$1 is required but not on PATH (see: make -C deploy/gitops doctor)."
}

note()  { printf '     %s\n' "$*"; }
head1() { printf '\n%s\n' "== $* =="; }
head2() { printf '\n%s\n' "-- $* --"; }

# Print a command exactly as it will be run, in a form that can be pasted back
# into a shell. This is half the value of the whole script: the printed line is
# the evidence that the local path and the CI path are the same commands, and it
# is what a reviewer diffs against the workflow YAML.
show_cmd() {
  local prefix="$1"
  shift
  printf '   $ %s%s\n' "$prefix" "$(printf '%q ' "$@")"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s' "$1" | sha256sum | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    printf '%s' "$1" | shasum -a 256 | awk '{print $1}'
  else
    die "neither sha256sum nor shasum is on PATH; --expect-api-server-sha256 cannot be checked."
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --kubeconfig)          KUBECONFIG_PATH="${2:?--kubeconfig needs a value}"; shift 2 ;;
    --expect-cluster)      EXPECT_CLUSTER="${2:?--expect-cluster needs a value}"; shift 2 ;;
    --allow-dirty)         ALLOW_DIRTY=1; shift ;;
    --expect-api-server-sha256)
                           EXPECT_API_SHA="${2:?--expect-api-server-sha256 needs a value}"; shift 2 ;;
    --as-user)             AS_USER="${2:?--as-user needs a value}"; shift 2 ;;
    --stage)               STAGE="${2:?--stage needs a value}"; shift 2 ;;
    --env)                 GITOPS_ENV="${2:?--env needs a value}"; shift 2 ;;
    --chart-version)       CHART_VERSION="${2:?--chart-version needs a value}"; shift 2 ;;
    --timeout)             DEPLOY_TIMEOUT="${2:?--timeout needs a value}"; shift 2 ;;
    --base-url)            BASE_URL="${2:?--base-url needs a value}"; shift 2 ;;
    --seed-email)          SEED_DEV_EMAIL="${2:?--seed-email needs a value}"; shift 2 ;;
    --manifest)            MANIFEST_PATH="${2:?--manifest needs a value}"; shift 2 ;;
    --diagnostics)         DIAGNOSTICS_SCRIPT="${2:?--diagnostics needs a value}"; shift 2 ;;
    --apply)               APPLY=1; shift ;;
    --i-know-this-deploys) CONFIRM_TOKEN="${2:?--i-know-this-deploys needs a value}"; shift 2 ;;
    --print-commands)      PRINT_ONLY=1; shift ;;
    -h|--help)             usage; exit 0 ;;
    *)                     usage >&2; die "unknown argument: $1" ;;
  esac
done

case "$STAGE" in
  deploy|seed|smoke|all) ;;
  *) die "--stage must be one of deploy, seed, smoke, all (got '$STAGE')." ;;
esac

CONFIRM_EXPECTED="yes-deploy-${GITOPS_ENV}"
ENV_REL="deploy/gitops/environments/$GITOPS_ENV"
ENV_DIR="$REPO_ROOT/$ENV_REL"
INVENTORY="$ENV_DIR/inventory.yaml"
VALUES_REL="$ENV_REL/values.yaml"
VALUES="$REPO_ROOT/$VALUES_REL"

need yq
need helm
need kubectl
need jq
need make

# The override chain, and it ends in EMPTY on purpose — this line used to end in
# a committed address, which is what let a local rehearsal pass the seeder a
# dev-lead the stand's realm had never heard of. Empty is not a missing value
# here; it is the instruction to let seed-stand.sh read the realm, which is the
# path CI takes and therefore the only path worth rehearsing.
#
# $TEST_STAND_SEED_EMAIL is still consulted so the two sides stay the same shape:
# CI reads the same-named environment secret and appends --email only when it is
# non-empty. Set it in neither place and both discover; set it in one and that
# side overrides — which is a divergence you can see in the printed command
# rather than one hiding in a default.
[ -n "$SEED_DEV_EMAIL" ] || SEED_DEV_EMAIL="${TEST_STAND_SEED_EMAIL:-}"

# ── The environment the harness drives ──────────────────────────────────────
# Read from the committed gitops environment rather than taken as flags: the
# stand's configuration lives in the repo, and a harness that accepted the
# namespace and release on the command line could rehearse a topology the
# workflow will never deploy. The workflow pins STAND_NAMESPACE / STAND_RELEASE
# to the same values and reads kubeContext out of the same inventory.
#
# --print-commands is the one mode that tolerates a missing environment: it
# exists so a reviewer can read the contract before the environment lands, and
# it substitutes obviously-fake placeholders rather than plausible ones.
if [ -f "$INVENTORY" ]; then
  KUBE_CTX="$(yq -r '.kubeContext // ""' "$INVENTORY")"
  NS_APP="$(yq -r '.namespaces.services // "insight"' "$INVENTORY")"
  RELEASE="$(yq -r '.release          // "insight"' "$INVENTORY")"
  [ -n "$KUBE_CTX" ] && [ "$KUBE_CTX" != "null" ] \
    || die "$INVENTORY has no kubeContext; every guard in this script, in the workflow and in the Makefile keys off it."
elif [ "$PRINT_ONLY" -eq 1 ]; then
  KUBE_CTX="<inventory.kubeContext>"
  NS_APP="<inventory.namespaces.services>"
  RELEASE="<inventory.release>"
else
  die "no gitops inventory at $INVENTORY — the '$GITOPS_ENV' environment has not landed in this checkout yet."
fi

if [ ! -f "$VALUES" ] && [ "$PRINT_ONLY" -eq 0 ]; then
  die "no values file at $VALUES — the '$GITOPS_ENV' environment is incomplete."
fi

# The default is a CONVENIENCE FOR THE READ-ONLY MODES and nothing more, and the
# reason is structural rather than a matter of the file being out of date: CI
# never reads .insight-version at all. It takes the version from publish-chart's
# output, and publish-chart commits the bump as its LAST step — so a checkout of
# any commit names the release BEFORE the one the stand is running, and the lag
# grows by one for every merge since this branch was cut.
#
# Under --apply that turns an omitted flag into an unannounced ROLLBACK of a
# published stand, and — this is the part worth stating out loud — the
# post-upgrade verification below would call it a success: it compares the live
# chart against the version this run REQUESTED, not against the version that was
# on the stand a minute ago. There is no check anywhere on this path that a
# deploy moves forward.
#
# So name --chart-version explicitly for anything that mutates. Making the script
# refuse to default under --apply, or refuse a version older than the deployed
# release, is a behaviour change and belongs in its own reviewed commit.
if [ -z "$CHART_VERSION" ]; then
  CHART_VERSION="$(cat "$GITOPS_DIR/.insight-version" 2>/dev/null || true)"
  [ -n "$CHART_VERSION" ] \
    || die "no --chart-version, and deploy/gitops/.insight-version is empty. In CI this value is the publish-chart job's output; locally you must name it."
  if [ "$APPLY" -eq 1 ]; then
    printf 'WARNING: no --chart-version, so this run would install %s, read out of\n' "$CHART_VERSION" >&2
    printf '         deploy/gitops/.insight-version. That file names the release published\n' >&2
    printf '         BEFORE the stand was last deployed, so this may be a downgrade — and\n' >&2
    printf '         the post-upgrade check compares against the version requested, not\n' >&2
    printf '         against what was there, so it would report success either way.\n' >&2
  fi
fi

# ── Where the stand answers ─────────────────────────────────────────────────
# Derived from the committed values file so a local smoke run addresses the same
# origin the deployed authenticator redirects back to. Deriving beats a hardcoded
# constant twice over: the URL is never written into this script, and a values
# file that changes hosts moves the smoke target with it instead of silently
# testing the old one. (CI takes it from a repository variable instead — the same
# address arriving by a different route.)
resolve_base_url() {
  local candidate
  [ -f "$VALUES" ] || return 1

  # The authenticator's registered redirect is by definition an absolute URL on
  # the stand's public origin, and always ends in the callback path.
  candidate="$(yq -r '.authenticator.oidc.redirectUri // ""' "$VALUES")"
  if [ -n "$candidate" ] && [ "$candidate" != "null" ]; then
    candidate="${candidate%/auth/callback}"
    printf '%s' "${candidate%/}"
    return 0
  fi

  # Fallback: the bundled Keycloak's hostname is the same origin with the realm
  # prefix appended.
  candidate="$(yq -r '.keycloak.hostname // ""' "$VALUES")"
  if [ -n "$candidate" ] && [ "$candidate" != "null" ]; then
    candidate="${candidate%/kc}"
    printf '%s' "${candidate%/}"
    return 0
  fi

  return 1
}

[ -n "$BASE_URL" ] || BASE_URL="${SMOKE_BASE_URL:-}"
[ -n "$BASE_URL" ] || BASE_URL="$(resolve_base_url || true)"

mkdir -p "$ARTIFACT_DIR"
ARTIFACT_REL="${ARTIFACT_DIR#"$REPO_ROOT"/}"
RUN_STAMP="$(date -u +%Y%m%d-%H%M%S)"
SEED_LOG="$ARTIFACT_DIR/seed-$RUN_STAMP.log"
MANIFEST_EXTRACTOR="$REPO_ROOT/.github/workflows/scripts/extract-seed-manifest.py"
SEED_LOG_REL="${SEED_LOG#"$REPO_ROOT"/}"
[ -n "$MANIFEST_PATH" ] || MANIFEST_PATH="$ARTIFACT_DIR/seed-manifest.json"
MANIFEST_REL="${MANIFEST_PATH#"$REPO_ROOT"/}"

# ── Command definitions ─────────────────────────────────────────────────────
# One array per command, defined once, printed before use and never rebuilt
# inline — so what the script announces and what the script runs cannot drift,
# and so --print-commands can show the whole plan without executing it.
#
# The one command NOT fully materialised here is the helm upgrade: its
# --set-string carries the OIDC client secret, which is read out of the cluster
# at exec time and must never reach the terminal. The display form shows the
# substitution expression instead — which is also what documents where the value
# comes from.

restart_args=()
for _target in $RESTART_TARGETS; do
  restart_args+=("$_target")
done
unset _target

route_args=()
for _route in $ROUTE_NAMES; do
  route_args+=("$_route")
done
unset _route

deploy_helm_cmd=(
  helm upgrade --install "$RELEASE" "$CHART_REF"
  --version "$CHART_VERSION"
  --namespace "$NS_APP"
  --values "$VALUES_REL"
  --wait --timeout "$DEPLOY_TIMEOUT"
  --history-max 10
)

# shellcheck disable=SC2016  # the command substitution is displayed, never run
deploy_helm_secret_display='--set-string authenticator.oidc.clientSecret="$(kubectl -n '"$NS_APP"' get secret insight-oidc -o '"'"'jsonpath={.data.client-secret}'"'"' | base64 --decode)"'

deploy_verify_cmd=(
  helm list -n "$NS_APP"
  --deployed --failed --pending --uninstalling
  --filter "^${RELEASE}\$" -o json
)

deploy_restart_cmd=(kubectl -n "$NS_APP" rollout restart "${restart_args[@]}")
deploy_rollout_cmd=(kubectl -n "$NS_APP" rollout status --timeout=5m "${restart_args[@]}")

route_table_cmd=(
  kubectl -n "$NS_APP" get httproute "${route_args[@]}"
  -o 'custom-columns=NAME:.metadata.name,ACCEPTED:.status.parents[0].conditions[?(@.type=="Accepted")].status'
)
route_status_cmd=(
  kubectl -n "$NS_APP" get httproute "${route_args[@]}"
  -o 'jsonpath={range .items[*]}{.metadata.name}{"="}{.status.parents[0].conditions[?(@.type=="Accepted")].status}{"\n"}{end}'
)

deploy_cmd_dry=(make diff "ENV=$GITOPS_ENV" "INSIGHT_VERSION=$CHART_VERSION")

# The --allow-dirty render. Same chart, same version, same values file as the
# upgrade above; no cluster contact, no clean-tree assertion.
deploy_render_cmd=(
  helm template "$RELEASE" "$CHART_REF"
  --version "$CHART_VERSION"
  --namespace "$NS_APP"
  --values "$VALUES_REL"
)

# `--email` is APPENDED, not interpolated, and the difference is load-bearing:
# `--email ""` is not "no address, go and discover one", it is
# `${2:?--email needs a value}` in the seeder's argument parser killing the run
# before it reads anything. An argument that is only sometimes present has to be
# absent, not empty — which is the same reason the workflow builds its own
# email_args array instead of passing the secret through unconditionally.
#
# The bash-3.2-safe `${arr[@]+"${arr[@]}"}` spelling is used for the same reason
# it is used for the RBAC probe's --as-user args below: macOS ships bash 3.2,
# where a bare "${arr[@]}" on an empty array is an unbound-variable error under
# `set -u`, and this script's normal case is now precisely the empty one.
seed_email_args=()
if [ -n "$SEED_DEV_EMAIL" ]; then
  seed_email_args=(--email "$SEED_DEV_EMAIL")
fi

seed_cmd_apply=(
  "$SEED_REL"
  -n "$NS_APP"
  --context "$KUBE_CTX"
  ${seed_email_args[@]+"${seed_email_args[@]}"}
  --days "$SEED_WINDOW_DAYS"
)

seed_cmd_dry=("${seed_cmd_apply[@]}" --dry-run)

smoke_cmd_apply=(
  uv run --project tests --frozen
  pytest "$SMOKE_SUITE" --stand-manifest "$MANIFEST_PATH"
)

smoke_cmd_dry=("${smoke_cmd_apply[@]}" --collect-only -q)

print_plan() {
  head1 "stage commands"
  note "Diff these against $WORKFLOW_REL; they must match."
  note "Stage 1's dry-run form runs from deploy/gitops; everything else runs"
  note "from the repository root."

  head2 "stage 1 - deploy"
  note "apply:"
  show_cmd "KUBECONFIG=<file> " "${deploy_helm_cmd[@]}"
  printf '       %s\n' "$deploy_helm_secret_display"
  note "then, because a release can be 'deployed' and still be the wrong chart:"
  show_cmd "KUBECONFIG=<file> " "${deploy_verify_cmd[@]}"
  note "then, because envFrom configuration is read once at container start:"
  show_cmd "KUBECONFIG=<file> " "${deploy_restart_cmd[@]}"
  show_cmd "KUBECONFIG=<file> " "${deploy_rollout_cmd[@]}"
  note "then, because helm --wait does not wait on HTTPRoute status:"
  show_cmd "KUBECONFIG=<file> " "${route_table_cmd[@]}"
  note "dry-run substitute for the upgrade (offline render and diff):"
  show_cmd "" "${deploy_cmd_dry[@]}"

  head2 "stage 2 - seed"
  # Said in the plan rather than only in the source, because the absence of a
  # flag is the one thing a printed command cannot show. A reader diffing this
  # against the workflow needs to know that the missing --email is deliberate on
  # both sides and that the address is coming from the same place on both.
  if [ -n "$SEED_DEV_EMAIL" ]; then
    note "dev-lead address: OVERRIDDEN (--seed-email / \$TEST_STAND_SEED_EMAIL)."
    note "  The stand's own realm is not being consulted. An address that is not"
    note "  the realm's dev-lead seeds a person who cannot log in, and every"
    note "  stage stays green while it happens."
  else
    note "dev-lead address: none passed — seed-stand.sh reads it from the realm"
    note "  ConfigMap the stand applied. The workflow omits --email for the same"
    note "  reason: the realm is where the roster is written, and the seed follows"
    note "  it so a regenerated realm cannot leave the data behind."
  fi
  note "apply:"
  show_cmd "KUBECONFIG=<file> " "${seed_cmd_apply[@]}"
  note "dry-run:"
  show_cmd "KUBECONFIG=<file> " "${seed_cmd_dry[@]}"

  head2 "stage 3 - smoke"
  note "apply:"
  show_cmd "SMOKE_BASE_URL=${BASE_URL:-<--base-url>} " "${smoke_cmd_apply[@]}"
  note "dry-run:"
  show_cmd "SMOKE_BASE_URL=${BASE_URL:-<--base-url>} " "${smoke_cmd_dry[@]}"
  printf '\n'
}

if [ "$PRINT_ONLY" -eq 1 ]; then
  print_plan
  exit 0
fi

# ── Argument validation that only matters for a run ─────────────────────────

[ -n "$KUBECONFIG_PATH" ] || { usage >&2; die "--kubeconfig is required."; }
[ -n "$EXPECT_CLUSTER" ] || { usage >&2; die "--expect-cluster is required. It is the guard that stops this run reaching a stand it was not aimed at."; }
[ -f "$KUBECONFIG_PATH" ] || die "kubeconfig not found at $KUBECONFIG_PATH."

case "$KUBECONFIG_PATH" in
  "$REPO_ROOT"/*)
    die "the kubeconfig is inside the repository working tree ($KUBECONFIG_PATH). \`make diff\` depends on sync-clean, which fails on any file git reports — keep the kubeconfig elsewhere." ;;
esac

if [ "$APPLY" -eq 1 ] && [ "$CONFIRM_TOKEN" != "$CONFIRM_EXPECTED" ]; then
  refuse "--apply was given without the matching confirmation token." \
    "This run would change a published stand that other people look at." \
    "" \
    "Re-run with: --i-know-this-deploys $CONFIRM_EXPECTED"
fi
if [ "$APPLY" -eq 0 ] && [ -n "$CONFIRM_TOKEN" ]; then
  die "--i-know-this-deploys was given without --apply. Both are required to mutate; neither alone does anything."
fi

# ── Cluster guard ───────────────────────────────────────────────────────────
# Everything below is either a `kubectl config` read (a local file) or a
# `get`/`auth can-i` (read-only), and it all runs before any stage, in every
# mode — including the dry run, which for the seed stage does reach the cluster.

kc() { kubectl --kubeconfig "$KUBECONFIG_PATH" "$@"; }

guard_cluster() {
  head1 "cluster guard"

  local current cluster server actual_sha
  local as_args=()
  local probe verb resource why answer

  current="$(kc config current-context 2>/dev/null || true)"
  [ -n "$current" ] || refuse "the kubeconfig has no current-context." \
    "File: $KUBECONFIG_PATH" \
    "Set one with: kubectl --kubeconfig $KUBECONFIG_PATH config use-context <name>"

  # CLUSTER IDENTITY IS CHECKED FIRST, AND IT IS THE ONLY CHECK THAT IS FATAL.
  #
  # The context NAME is a label inside a file; the cluster entry is what decides
  # which API server gets written to. Checking the name first (as this did
  # originally) refuses runs that are perfectly safe — an admin kubeconfig
  # downloaded from a provider is routinely called `default` — while proving
  # nothing about the target. So: verify the cluster, THEN reconcile the name.
  cluster="$(kc config view -o "jsonpath={.contexts[?(@.name==\"$current\")].context.cluster}" 2>/dev/null || true)"
  [ -n "$cluster" ] || refuse "the current context names no cluster entry." \
    "This kubeconfig is malformed; nothing further can be verified about the target."

  if [ "$cluster" != "$EXPECT_CLUSTER" ]; then
    refuse "the target cluster is not the one you said you expected." \
      "--expect-cluster : $EXPECT_CLUSTER" \
      "kubeconfig says  : $cluster" \
      "" \
      "Nothing has been run. If the kubeconfig is right, fix the flag; if the" \
      "flag is right, you have the wrong kubeconfig on the command line."
  fi
  note "cluster entry matches --expect-cluster"

  # Nothing on this path passes helm an explicit --kube-context, and the gitops
  # Makefile's own `kube-ctx` guard asserts `kubectl config current-context` ==
  # inventory.kubeContext. In CI that holds by construction: provision-ci-
  # deployer.sh writes the generated kubeconfig with the context already named
  # after the environment. A human rehearsing with an admin kubeconfig has
  # whatever name the provider chose, so rather than refuse, derive a normalised
  # COPY with the context renamed and run everything through that. The original
  # file is never modified, and the rename is only reached after the cluster
  # entry above has already been proven to be the intended one.
  if [ "$current" != "$KUBE_CTX" ]; then
    local normalised="$ARTIFACT_DIR/kubeconfig-$KUBE_CTX.yaml"
    ( umask 077; kc config view --raw --minify > "$normalised" ) \
      || refuse "could not derive a context-normalised kubeconfig." \
           "Source: $KUBECONFIG_PATH"
    kubectl --kubeconfig "$normalised" config rename-context "$current" "$KUBE_CTX" >/dev/null 2>&1 \
      || refuse "could not rename the context in the derived kubeconfig." \
           "Derived file: $normalised"
    kubectl --kubeconfig "$normalised" config use-context "$KUBE_CTX" >/dev/null 2>&1 \
      || refuse "could not select the renamed context in the derived kubeconfig." \
           "Derived file: $normalised"
    chmod 600 "$normalised" 2>/dev/null || true
    KUBECONFIG_PATH="$normalised"
    note "context '$current' renamed to '$KUBE_CTX' in a derived copy (original untouched)"
    note "  derived: ${normalised#"$REPO_ROOT"/}"
  else
    note "current-context matches inventory.kubeContext"
  fi

  server="$(kc config view -o "jsonpath={.clusters[?(@.name==\"$cluster\")].cluster.server}" 2>/dev/null || true)"
  [ -n "$server" ] || refuse "the cluster entry has no server URL." \
    "Nothing about the target can be verified; refusing rather than guessing."

  if [ -n "$EXPECT_API_SHA" ]; then
    actual_sha="$(sha256_of "$server")"
    if [ "$actual_sha" != "$EXPECT_API_SHA" ]; then
      refuse "the API server fingerprint does not match." \
        "expected sha256 : $EXPECT_API_SHA" \
        "actual sha256   : $actual_sha" \
        "" \
        "Names in a kubeconfig can be edited into agreement; this cannot. The" \
        "server URL itself is deliberately not printed — this repo is public" \
        "and terminal output gets pasted into issues."
    fi
    note "API server fingerprint matches"
  else
    note "no --expect-api-server-sha256 given"
    note "  Context and cluster names are LOCAL ALIASES: a kubeconfig for a"
    note "  different stand whose entries happen to carry these names passes"
    note "  every check above. Pin the fingerprint before using --apply:"
    note "    kubectl --kubeconfig <file> config view --minify \\"
    note "      -o 'jsonpath={.clusters[0].cluster.server}' | shasum -a 256"
    if [ "$APPLY" -eq 1 ]; then
      note "  WARNING: mutating a published stand with the weaker guard."
    fi
  fi

  # Reachability, and the namespace the whole run addresses. `get namespace`
  # rather than `cluster-info`: a namespace-scoped token — which is exactly what
  # the CI deployer credential is — can fail cluster-info while being perfectly
  # able to do its job, and a guard that rejects the real CI credential is a
  # guard nobody will run.
  if ! kc --request-timeout=10s get namespace "$NS_APP" -o name >/dev/null 2>&1; then
    refuse "cannot read namespace '$NS_APP' on the target cluster." \
      "Either the cluster is unreachable (VPN?) or this credential cannot see it." \
      "Nothing has been run."
  fi
  note "namespace $NS_APP is reachable"

  # An RBAC rehearsal, not a gate: it answers "would the CI ServiceAccount get
  # through?" before a merge has to. Printed, never enforced — the verbs a stage
  # needs are the stage's business, and a can-i list that disagreed with reality
  # would be one more thing to keep in sync.
  head2 "RBAC probe${AS_USER:+ (as $AS_USER)}"
  [ -n "$AS_USER" ] && as_args=(--as "$AS_USER")
  for probe in \
      "get:secret:read insight-oidc for the client secret" \
      "get:configmap:read the platform coordinates (seed discovery)" \
      "patch:deployment:helm upgrade, and the rollout restart after it" \
      "create:job:apply the seed Job" \
      "get:httproute:confirm the edge still routes"; do
    verb="${probe%%:*}"
    resource="${probe#*:}"
    why="${resource#*:}"
    resource="${resource%%:*}"
    answer="$(kc ${as_args[@]+"${as_args[@]}"} auth can-i "$verb" "$resource" -n "$NS_APP" 2>/dev/null || true)"
    printf '     %-7s %-11s %-5s  %s\n' "$verb" "$resource" "${answer:-?}" "$why"
  done
}

# ── Parity report ───────────────────────────────────────────────────────────
# Cheap, never fatal, printed on every run. It cannot prove the two paths are
# identical — only a human diff of the printed commands against the workflow can
# do that — but it does catch the failure mode that matters in practice: the
# workflow and this harness quietly growing two ways to do one thing. The named
# checks at the end are for divergences already known to break a run; each is an
# assertion about the workflow's TEXT, so it goes quiet the moment CI is fixed.

parity_report() {
  head1 "parity with $WORKFLOW_REL"
  if [ ! -f "$WORKFLOW_FILE" ]; then
    note "the workflow does not exist in this checkout — nothing to compare against."
    note "When it lands, every command this script prints must appear in it verbatim."
    return 0
  fi
  local anchor
  for anchor in \
      "helm upgrade --install" \
      "$CHART_REF" \
      "$VALUES_REL" \
      "authenticator.oidc.clientSecret" \
      "--wait --timeout $DEPLOY_TIMEOUT" \
      "helm list" \
      "--deployed --failed --pending --uninstalling" \
      "rollout restart" \
      "get httproute" \
      "seed-stand.sh" \
      "--days $SEED_WINDOW_DAYS" \
      "--stand-manifest" \
      "SMOKE_BASE_URL" \
      "$SMOKE_SUITE" \
      "stand-diagnostics.sh"; do
    if grep -qF -- "$anchor" "$WORKFLOW_FILE"; then
      printf '     %-6s %s\n' "found" "$anchor"
    else
      printf '     %-6s %s\n' "ABSENT" "$anchor"
    fi
  done

  # Known-wrong spelling: seed-stand.sh has no --window-days flag and refuses
  # unknown arguments, so a workflow carrying it fails the seed stage every run.
  if grep -qF -- "--window-days" "$WORKFLOW_FILE"; then
    printf '     %-6s %s\n' "BUG" "the workflow passes --window-days; seed-stand.sh only accepts --days"
  fi

  # The dev-lead address. Deliberately NOT an anchor in the list above: --email
  # is now conditional on both sides, so the literal appears in the workflow's
  # text whether or not any run passes it, and an anchor asserting its presence
  # would prove nothing while an anchor asserting its absence would fire on the
  # override machinery itself. What can be asserted is the workflow's disposition
  # towards the value, and these two checks cover the two ways it can be wrong.
  #
  # The first is loud but harmless: a workflow that still hard-requires the
  # secret fails its own preflight the moment an operator unsets a secret nothing
  # reads. The second is the dangerous one — a workflow that still passes
  # --email unconditionally seeds an address that the realm may never have
  # carried, the Job succeeds, the release stays `deployed`, and the only
  # symptom is a persona who authenticates and resolves to nobody. That is the
  # exact failure the discovery removes, so a harness that discovered while CI
  # supplied would be rehearsing the fixed path against the broken one.
  if grep -qF -- 'missing="$missing secrets.TEST_STAND_SEED_EMAIL"' "$WORKFLOW_FILE"; then
    printf '     %-6s %s\n' "BUG" "the workflow REQUIRES secrets.TEST_STAND_SEED_EMAIL; seed-stand.sh discovers the dev-lead address from the stand's realm, so nothing needs it"
  fi
  if grep -qE '^[[:space:]]*--email "\$SEED_EMAIL"' "$WORKFLOW_FILE"; then
    printf '     %-6s %s\n' "BUG" "the workflow passes --email unconditionally; it must append the flag only when the override secret is set, or the realm is never consulted"
  fi

  # The smoke suite is aimed by $SMOKE_BASE_URL and by nothing else: its conftest
  # raises pytest.UsageError when the command line names the directory and that
  # variable is unset, so a workflow setting only the shared INSIGHT_STAND_*
  # names never reaches a single check.
  if grep -qF -- "INSIGHT_STAND_BASE_URL" "$WORKFLOW_FILE" \
     && ! grep -qF -- "SMOKE_BASE_URL" "$WORKFLOW_FILE"; then
    printf '     %-6s %s\n' "BUG" "the workflow sets INSIGHT_STAND_BASE_URL; tests/stand/smoke reads SMOKE_BASE_URL"
  fi
  if grep -qF -- "INSIGHT_STAND_PERSONA_PASSWORD" "$WORKFLOW_FILE" \
     && ! grep -qF -- "SMOKE_PERSONA_PASSWORD" "$WORKFLOW_FILE"; then
    printf '     %-6s %s\n' "BUG" "the workflow sets INSIGHT_STAND_PERSONA_PASSWORD; tests/stand/smoke/login.py reads SMOKE_PERSONA_PASSWORD"
  fi

  note ""
  note "ABSENT means the workflow does something this harness does not emulate,"
  note "or the reverse. BUG means the two disagree in a way that fails a run."
  note "Reconcile either way, or the local rehearsal proves the wrong thing."
}

# ── Failure diagnostics ─────────────────────────────────────────────────────
# The workflow's own script, invoked with its documented positional interface
# (<namespace> <release>) and reading the ambient kubeconfig, exactly as CI
# invokes it. Never allowed to change the exit status: the stage's failure is the
# verdict, and a diagnostics script that died would otherwise mask it.

emit_diagnostics() {
  local stage="$1"
  local script="$DIAGNOSTICS_SCRIPT"
  head1 "diagnostics after a failed '$stage' stage"

  [ -n "$script" ] || script="$REPO_ROOT/$DIAGNOSTICS_DEFAULT_REL"
  if [ ! -f "$script" ]; then
    note "no diagnostics script at ${script#"$REPO_ROOT"/}."
    note "It belongs to the workflow (a curated, redacted allowlist — a public"
    note "repo's run logs are public). Point at it with --diagnostics, or set"
    note "INSIGHT_STAND_DIAGNOSTICS."
    note ""
    note "Deliberately NOT falling back to an ad-hoc dump here: output this"
    note "harness produced but CI never would is exactly what teaches an"
    note "engineer to expect evidence the red run will not have."
    return 0
  fi

  # It emits GitHub workflow commands (::group::, ::error::). On a laptop those
  # show as literal text; that is cosmetic and is left alone, because teaching it
  # a second output mode would make the local and the CI evidence differ in a way
  # nobody could then compare.
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " bash "${script#"$REPO_ROOT"/}" "$NS_APP" "$RELEASE"
  KUBECONFIG="$KUBECONFIG_PATH" bash "$script" "$NS_APP" "$RELEASE" \
    || note "(the diagnostics script itself exited non-zero; the stage failure above is still the verdict)"
}

fail_stage() {
  local stage="$1"
  local code="$2"
  emit_diagnostics "$stage"
  printf '\n%s\n' "== stage '$stage' FAILED (exit $code) =="
  note "Nothing has been rolled back. That is the intended disposition: the"
  note "failed state is the evidence, which is also why the upgrade runs with"
  note "--wait and deliberately without --atomic. Recovery is the next deploy,"
  note "or a deliberate 'make -C deploy/gitops rollback ENV=$GITOPS_ENV'."
  exit "$code"
}

# ── Shared checks ───────────────────────────────────────────────────────────

# Presence of the KEY, never its value: nothing here captures the secret into a
# variable, and the pipeline's only consumer is `grep -q`.
check_oidc_secret() {
  head2 "the OIDC client secret the upgrade injects"
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " kubectl -n "$NS_APP" get secret insight-oidc \
    -o 'jsonpath={.data.client-secret}'
  if kc -n "$NS_APP" get secret insight-oidc -o "jsonpath={.data.client-secret}" 2>/dev/null \
     | grep -q .; then
    note "present (presence only — no value is read into this shell or printed)"
    return 0
  fi
  note "ABSENT: Secret insight-oidc has no non-empty 'client-secret' key."
  note "An upgrade without it writes a BLANK client secret into the"
  note "authenticator's config: release deployed, pods Ready, every login"
  note "broken at the confidential-client token exchange."
  return 1
}

# The chart renders both routes, from the environment's `gateway.route` and
# `keycloak.route`, so the release owns them and a deploy is what writes them.
# This is therefore a POST-CONDITION on the upgrade rather than a check on an
# object somebody else applied — and it is still needed, because `helm --wait`
# waits on workloads and not on HTTPRoute status. A route helm stored
# successfully can be refused by the Gateway it names (a parentRef pointing at a
# Gateway that is not there, a sectionName naming a listener that does not
# exist), which leaves the release `deployed`, every pod Ready, and nobody able
# to reach the stand. Nothing is applied from here; there is nothing to apply.
check_routes() {
  head2 "the edge routes (read back, never applied from here)"
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${route_table_cmd[@]}"
  if ! kc "${route_table_cmd[@]:1}" 2>&1; then
    note "could not read the routes at all."
    return 1
  fi
  local not_accepted
  not_accepted="$(kc "${route_status_cmd[@]:1}" 2>/dev/null | grep -v '=True$' || true)"
  if [ -n "$not_accepted" ]; then
    note "an edge route is not Accepted:"
    printf '%s\n' "$not_accepted"
    note "The release is 'deployed' and its pods are Ready; the stand is still"
    note "unreachable, and the smoke would fail at its first request with a much"
    note "less useful message. The route came out of the deploy, so start in the"
    note "values file that produced it — $VALUES_REL, keys gateway.route and"
    note "keycloak.route — and check the parentRef they name against the Gateway"
    note "the cluster actually has."
    return 1
  fi
  return 0
}

# ── Stage 1 · deploy ────────────────────────────────────────────────────────

stage_deploy() {
  head1 "stage 1 - deploy (chart $CHART_VERSION -> release $RELEASE in $NS_APP)"

  local rc=0

  if [ "$APPLY" -eq 0 ]; then
    note "read-only. CI would run:"
    show_cmd "KUBECONFIG=<file> " "${deploy_helm_cmd[@]}"
    printf '       %s\n' "$deploy_helm_secret_display"
    note "running the Makefile's own offline render and diff instead — it goes"
    note "through the same values file the upgrade would, and never contacts"
    note "the cluster:"
    show_cmd "" "${deploy_cmd_dry[@]}"
    note "It depends on sync-clean, so a dirty working tree fails it exactly as"
    note "it would fail a real deploy from a clean CI checkout."

    # The chicken-and-egg this flag exists for: `make diff` refuses on ANY file
    # `git status --porcelain` reports, which is correct for a deploy (CI always
    # renders from a clean checkout) and impossible while the environment, the
    # workflow and this script are themselves uncommitted work in progress. The
    # strict path stays the default; --allow-dirty swaps in a bare `helm
    # template` through the SAME chart, version and values file, so the render is
    # still the real one — only the clean-tree assertion is skipped, and the
    # output says so rather than quietly passing.
    if [ "$ALLOW_DIRTY" -eq 1 ] && ! ( cd "$REPO_ROOT" && git diff --quiet HEAD -- . 2>/dev/null && [ -z "$(git status --porcelain)" ] ); then
      note ""
      note "--allow-dirty: the tree is dirty, so 'make diff' would refuse. Rendering"
      note "the same chart+values directly instead. THIS SKIPS THE CLEAN-TREE GATE"
      note "that a real deploy enforces — commit before trusting a green rehearsal."
      show_cmd "" "${deploy_render_cmd[@]}"
      ( cd "$REPO_ROOT" && "${deploy_render_cmd[@]}" > "$ARTIFACT_DIR/render-$RUN_STAMP.yaml" ) || rc=$?
      if [ "$rc" -eq 0 ]; then
        note "rendered $(grep -c '^kind:' "$ARTIFACT_DIR/render-$RUN_STAMP.yaml" 2>/dev/null || echo '?') objects -> ${ARTIFACT_DIR#"$REPO_ROOT"/}/render-$RUN_STAMP.yaml"
      fi
    else
      ( cd "$GITOPS_DIR" && "${deploy_cmd_dry[@]}" ) || rc=$?
    fi
    [ "$rc" -eq 0 ] || fail_stage deploy "$rc"

    # Reported, not fatal: a rehearsal that stopped at the first missing
    # prerequisite would hide the second, and finding both in one pass is the
    # point of running this before a merge.
    check_oidc_secret || note "(reported, not failed: the dry run changes nothing)"
    check_routes || note "(reported, not failed: the dry run changes nothing)"
    return 0
  fi

  # Refuse before touching the release rather than after: an upgrade that blanks
  # the client secret leaves a healthy-looking, unusable stand.
  check_oidc_secret || fail_stage deploy 1

  head2 "upgrade"
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${deploy_helm_cmd[@]}"
  printf '       %s\n' "$deploy_helm_secret_display"

  # The secret lives in a variable for the length of one helm call and is never
  # echoed. `set -x` is not used anywhere in this script for exactly this reason.
  local oidc_client_secret
  oidc_client_secret="$(kc -n "$NS_APP" get secret insight-oidc \
    -o "jsonpath={.data.client-secret}" 2>/dev/null | base64 --decode)"
  [ -n "$oidc_client_secret" ] || fail_stage deploy 1

  ( cd "$REPO_ROOT" \
    && KUBECONFIG="$KUBECONFIG_PATH" "${deploy_helm_cmd[@]}" \
         --set-string "authenticator.oidc.clientSecret=$oidc_client_secret" ) || rc=$?
  oidc_client_secret=""
  [ "$rc" -eq 0 ] || fail_stage deploy "$rc"

  # A release can be `deployed` and still be the wrong chart — a resumed run, a
  # hand-deploy that raced this one, an input that did not say what it meant.
  # Checked separately from the upgrade's exit status because they answer
  # different questions. The status flags are enumerated rather than `--all`,
  # which Helm 4 removed: the interesting failures are the ones the default
  # listing hides — a `pending-upgrade` release must be reported as itself, not
  # as "absent" — but `--all` errors out on a v4 client and reports exactly that
  # false "absent".
  head2 "verify the release is the chart this run asked for"
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${deploy_verify_cmd[@]}"

  local listed status chart revision
  listed="$( KUBECONFIG="$KUBECONFIG_PATH" "${deploy_verify_cmd[@]}" 2>/dev/null || printf '[]' )"
  status="$(printf '%s' "$listed" | jq -r '.[0].status // "absent"')"
  chart="$(printf '%s' "$listed" | jq -r '.[0].chart // "absent"')"
  revision="$(printf '%s' "$listed" | jq -r '.[0].revision // "?"')"
  note "release $RELEASE: status=$status chart=$chart revision=$revision"

  if [ "$status" != "deployed" ]; then
    note "release '$RELEASE' is '$status', not 'deployed'. It has been left in"
    note "that state on purpose — read the diagnostics below, then fix forward"
    note "with another deploy or roll back by hand."
    fail_stage deploy 1
  fi
  if [ "$chart" != "insight-$CHART_VERSION" ]; then
    note "the stand is running '$chart', not 'insight-$CHART_VERSION'. helm"
    note "reported success while installing something else — treat this as a"
    note "problem with the chart reference or the version input, not the stand."
    fail_stage deploy 1
  fi

  head2 "restart what the chart cannot know to restart"
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${deploy_restart_cmd[@]}"
  ( KUBECONFIG="$KUBECONFIG_PATH" "${deploy_restart_cmd[@]}" ) || rc=$?
  [ "$rc" -eq 0 ] || fail_stage deploy "$rc"
  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${deploy_rollout_cmd[@]}"
  ( KUBECONFIG="$KUBECONFIG_PATH" "${deploy_rollout_cmd[@]}" ) || rc=$?
  [ "$rc" -eq 0 ] || fail_stage deploy "$rc"

  check_routes || fail_stage deploy 1
  note "deploy stage complete"
}

# ── Stage 2 · seed ──────────────────────────────────────────────────────────

stage_seed() {
  head1 "stage 2 - seed"
  [ -f "$SEED_SCRIPT" ] || die "seeder not found at $SEED_SCRIPT."

  local rc=0

  if [ "$APPLY" -eq 0 ]; then
    note "read-only. CI would run:"
    show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${seed_cmd_apply[@]}"
    note "running the seeder's own dry run instead — it performs the SAME"
    note "cluster discovery (ConfigMap and Secret reads) and prints the Job it"
    note "would apply, which is the honest read-only rehearsal of this stage:"
    show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${seed_cmd_dry[@]}"
    ( cd "$REPO_ROOT" && KUBECONFIG="$KUBECONFIG_PATH" "${seed_cmd_dry[@]}" ) || rc=$?
    [ "$rc" -eq 0 ] || fail_stage seed "$rc"
    note ""
    note "A dry run produces no manifest: the seeder emits one only after a real"
    note "run completes. Stage 3's dry run therefore has nothing to read unless"
    note "--manifest names one from an earlier real seed."
    return 0
  fi

  show_cmd "KUBECONFIG=$KUBECONFIG_PATH " "${seed_cmd_apply[@]}"
  note "raw log -> $SEED_LOG_REL"

  # The seed runs as a Job whose filesystem is discarded with the pod, so the
  # manifest exists only in the log the seeder streams back. Capturing it is the
  # same plumbing the workflow's "capture the seed manifest" step performs, from
  # the same raw stream: without it stage 3 has no manifest and pytest aborts at
  # collection. CI additionally pipes the console copy through the redactor;
  # locally the console is the engineer's own, so it is left intact.
  #
  # `set -o pipefail` is in force, so the pipeline's status is the seeder's and
  # not tee's.
  ( cd "$REPO_ROOT" && KUBECONFIG="$KUBECONFIG_PATH" "${seed_cmd_apply[@]}" ) 2>&1 \
    | tee "$SEED_LOG" || rc=$?
  [ "$rc" -eq 0 ] || fail_stage seed "$rc"

  extract_manifest || fail_stage seed 1
  head2 "manifest"
  note "written to $MANIFEST_REL"
  note "manifest_version: $(jq -r '.manifest_version // "?"' "$MANIFEST_PATH")"
  note "anchor_date:      $(jq -r '.anchor_date // "?"' "$MANIFEST_PATH")"
  note "personas:         $(jq -r '(.personas // []) | length' "$MANIFEST_PATH")"
  note "fixtures:         $(jq -r '(.fixtures // {}) | keys | join(\", \")' "$MANIFEST_PATH")"
  note ""
  note "A summary, not the document. Fixture NAMES are contract; the file itself"
  note "also carries persona addresses, UUIDs, the tenant and in-cluster service"
  note "URLs, and stays under .deploy/ (gitignored). Never attach it to an"
  note "issue, a PR, or a CI artifact."
}

# Manifest recovery is DELEGATED, not reimplemented.
#
# This used to be a local awk scan for the last `^{$` through the next `^}$`,
# with a comment claiming it matched the workflow's extractor. When the
# workflow's was fixed for interleaved log lines, this one was not — and the
# comment went on asserting a parity that no longer held, which is worse than
# not checking at all. The whole point of this harness is that the laptop path
# and the CI path run the same commands; a divergence hidden inside it is the
# one bug it cannot report.
#
# So both callers now invoke .github/workflows/scripts/extract-seed-manifest.py.
# The parity is structural rather than asserted: there is one algorithm, and
# neither side can drift without deleting that file. Read its header for why the
# naive brace scan is wrong (the manifest is stdout, the seeder's log is stderr,
# and the two arrive merged and interleaved).
extract_manifest() {
  local out rc
  if [ ! -x "$MANIFEST_EXTRACTOR" ] && [ ! -f "$MANIFEST_EXTRACTOR" ]; then
    note "the shared manifest extractor is missing: ${MANIFEST_EXTRACTOR#"$REPO_ROOT"/}"
    note "This harness does not carry its own copy on purpose — see the comment above."
    return 1
  fi
  # Diagnostics on stderr, summary on stdout, exit status is the verdict. The
  # script is deliberately annotation-agnostic so CI can wrap a failure in a
  # `::error::` and this can print it plainly.
  out="$(python3 "$MANIFEST_EXTRACTOR" "$SEED_LOG" "$MANIFEST_PATH" 2>&1)" && rc=0 || rc=$?
  printf '%s\n' "$out" | while IFS= read -r line; do note "$line"; done
  if [ "$rc" -ne 0 ]; then
    note "Recover it by hand from $SEED_LOG_REL and pass --manifest."
    return 1
  fi
  return 0
}

# ── Stage 3 · smoke ─────────────────────────────────────────────────────────

stage_smoke() {
  head1 "stage 3 - smoke"
  need uv
  need curl
  [ -d "$REPO_ROOT/$SMOKE_SUITE" ] \
    || die "no smoke suite at $SMOKE_SUITE — it is what turns this from a deploy into a gate."
  [ -n "$BASE_URL" ] \
    || die "no base URL. Pass --base-url, export SMOKE_BASE_URL, or set authenticator.oidc.redirectUri in $VALUES_REL."

  local rc=0
  local code

  # Through the public URL, no port-forwards. Prove the origin answers before
  # spending a suite's worth of time discovering that it does not.
  head2 "reachability"
  show_cmd "" curl -sS -o /dev/null -w '%{http_code}' "$BASE_URL/"
  code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 20 "$BASE_URL/" 2>/dev/null || printf '000')"
  note "GET / -> HTTP $code"
  if [ "$code" = "000" ]; then
    note "the stand's public URL did not answer at all. Check the edge routes"
    note "the deploy stage reported before looking at the release."
    fail_stage smoke 1
  fi

  if [ "$APPLY" -eq 0 ]; then
    note "read-only. CI would run:"
    show_cmd "SMOKE_BASE_URL=$BASE_URL " "${smoke_cmd_apply[@]}"
    if [ ! -f "$MANIFEST_PATH" ]; then
      note ""
      note "Collection needs a seed manifest and there is none at $MANIFEST_REL."
      note "The suite resolves its personas from that document at COLLECTION"
      note "time, so this is a hard stop rather than a skip. Run the seed stage"
      note "for real once, or pass --manifest from a previous run."
      return 0
    fi
    show_cmd "SMOKE_BASE_URL=$BASE_URL " "${smoke_cmd_dry[@]}"
    ( cd "$REPO_ROOT" \
      && SMOKE_BASE_URL="$BASE_URL" \
         INSIGHT_STAND_ARTIFACT_DIR="$ARTIFACT_DIR" \
         INSIGHT_STAND_MANIFEST="$MANIFEST_PATH" \
         "${smoke_cmd_dry[@]}" ) || rc=$?
    [ "$rc" -eq 0 ] || fail_stage smoke "$rc"
    return 0
  fi

  [ -f "$MANIFEST_PATH" ] \
    || die "no seed manifest at $MANIFEST_REL. Run --stage seed --apply first, or pass --manifest."

  # The suite's own credential resolution names the exact variable missing for
  # the chosen SMOKE_LOGIN_MODE, so this is a pointer rather than a second copy
  # of that rule — duplicating it here would be one more thing to keep in sync
  # with the login code, and it is the login code that has to be right.
  head2 "credentials"
  note "mode = ${SMOKE_LOGIN_MODE:-password [the suite default]}"
  note "Read from the environment by the suite itself, which fails naming the"
  note "missing variable. Nothing is defaulted, bridged, or printed here."

  show_cmd "SMOKE_BASE_URL=$BASE_URL " "${smoke_cmd_apply[@]}"
  # SMOKE_BASE_URL is what aims the suite — its conftest copies the value into
  # the shared stand resolution and refuses an explicit request without it.
  # INSIGHT_STAND_* are set explicitly because the library computes its defaults
  # from its own file location, and neither a runner nor an arbitrary working
  # directory is a developer's checkout.
  ( cd "$REPO_ROOT" \
    && SMOKE_BASE_URL="$BASE_URL" \
       INSIGHT_STAND_ARTIFACT_DIR="$ARTIFACT_DIR" \
       INSIGHT_STAND_MANIFEST="$MANIFEST_PATH" \
       "${smoke_cmd_apply[@]}" ) || rc=$?
  [ "$rc" -eq 0 ] || fail_stage smoke "$rc"
}

# ── Run ─────────────────────────────────────────────────────────────────────

head1 "emulating $WORKFLOW_REL"
if [ "$APPLY" -eq 1 ]; then
  printf '     %-18s %s\n' "mode" "APPLY — this run changes the stand"
else
  printf '     %-18s %s\n' "mode" "read-only (dry run)"
fi
printf '     %-18s %s\n' "stage(s)"      "$STAGE"
printf '     %-18s %s\n' "gitops env"    "$GITOPS_ENV"
printf '     %-18s %s\n' "chart version" "$CHART_VERSION"
printf '     %-18s %s\n' "release / ns"  "$RELEASE / $NS_APP"
printf '     %-18s %s\n' "kubeconfig"    "$KUBECONFIG_PATH"
printf '     %-18s %s\n' "base url"      "${BASE_URL:-<unresolved>}"
printf '     %-18s %s\n' "artefacts"     "$ARTIFACT_REL"

parity_report
guard_cluster

case "$STAGE" in
  deploy) stage_deploy ;;
  seed)   stage_seed ;;
  smoke)  stage_smoke ;;
  all)
    # Sequential, and each stage function exits the script on failure rather
    # than returning — so this ordering IS the gate that keeps smoke from ever
    # running after a failed seed. The workflow gets the same guarantee from
    # steps without `if:`.
    stage_deploy
    stage_seed
    stage_smoke
    ;;
esac

head1 "done"
if [ "$APPLY" -eq 1 ]; then
  note "stage(s) '$STAGE' completed against $EXPECT_CLUSTER."
else
  note "read-only rehearsal of stage(s) '$STAGE' completed. Nothing was changed."
fi
note ""
note "This output names your cluster, context and kubeconfig path, and is NOT"
note "redacted. The repo is public — scrub it before pasting any of it into an"
note "issue or a PR, or re-run the stage piping through"
note "  python3 .github/workflows/scripts/redact-stand-log.py"
note "to see what CI would have published."
