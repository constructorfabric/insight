#!/usr/bin/env bash
# Seed a Kubernetes stand with the demo organisation and its activity.
#
# Every coordinate the seeder needs is already in the cluster, so this script
# reads them from the stand rather than asking an operator to copy them:
#
#   ConfigMap <release>-platform          MariaDB + ClickHouse hosts, ports,
#                                         users, and the product database that
#                                         holds the analytics catalogue
#   Secret insight-identity-resolution-*  the stand's tenant, and the database
#                                         holding `persons`
#   helm get values <release>             the seed image the release pins
#
# Credentials are never read: the rendered Job references the release's own
# database Secret by key, so nothing sensitive passes through this shell.
#
# Nothing is defaulted. A value that can neither be discovered nor supplied is
# a hard error naming the flag that fixes it.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JOB_TEMPLATE="$SCRIPT_DIR/seed-job.yaml.tpl"

# RULE-DEFAULTS-OK: `all` is the seeder's own documented default command and
# every step is idempotent, so seeding everything is the safe reading of "seed
# this stand"; the flag exists to do LESS than that.
STEP="all"
# RULE-DEFAULTS-OK: wall-clock ceiling for a pod, not a config input — the gold
# rebuild dominates a full run and an hour is well beyond it.
DEADLINE_SECONDS="3600"

NAMESPACE=""
CONTEXT=""
RELEASE=""
DEV_EMAIL=""
TENANT=""
IMAGE=""
ANALYTICS_DB=""
IDENTITY_DB=""
DB_SECRET=""
PULL_SECRETS=""
IDP_SOURCE_TYPE=""
WINDOW_DAYS=""
ANCHOR_DATE=""
CROSS_TENANT="0"
FORCE="0"
DRY_RUN=0
FOLLOW=1

usage() {
  cat <<'USAGE'
Usage: seed-stand.sh -n <namespace> --email <address> [options]

Runs the demo-data seeder as a one-shot Job on a chart-deployed stand, using the
seeder image the release already pins.

Required:
  -n, --namespace <ns>     namespace the Insight release runs in
      --email <address>    persona the dev-lead login resolves to. A user with
                           this email must already exist in the stand's IdP —
                           the authenticator resolves people by the email claim.

Discovered from the stand (pass a flag only to override):
      --context <name>     kube context to act on         [default: the current one]
      --release <name>     helm release name                 [default: same as -n]
      --tenant <uuid>      tenant every seeded row is scoped to
      --image <ref>        seeder image to run (chart: ingestion.seedImage)
      --analytics-db <db>  database holding metric_definitions
      --identity-db <db>   database holding persons
      --db-secret <name>   Secret holding mariadb-password + clickhouse-password
      --pull-secret <name> image-pull Secret (default: the release's own)
      --idp-source-type <t> identity source_type the login rows are written under

Seed options:
      --step <step>        identity | silver | analytics | all       [default: all]
      --days <n>           activity-window length in days
      --anchor <date>      last day carrying activity (YYYY-MM-DD)
      --cross-tenant       also write the second-tenant refusal fixture
      --force              seed a tenant that already holds foreign person rows
      --deadline <secs>    pod wall-clock ceiling                  [default: 3600]

Output:
      --dry-run            print the rendered Job manifest and exit
      --no-follow          apply the Job without following its logs
  -h, --help               this text

Examples:
  seed-stand.sh -n insight --email you@example.com --dry-run
  seed-stand.sh -n insight --email you@example.com
  seed-stand.sh -n insight --email you@example.com --step identity
USAGE
}

die() {
  echo "ERROR: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required but not on PATH."
}

# Every cluster call goes through these, so the target is whatever --context
# names rather than whatever the shell was last pointed at. An ambient context
# can change between two runs of this script; the cluster it writes to should
# not.
kube() {
  if [[ -n "$CONTEXT" ]]; then kubectl --context "$CONTEXT" "$@"; else kubectl "$@"; fi
}

helm_release() {
  if [[ -n "$CONTEXT" ]]; then helm --kube-context "$CONTEXT" "$@"; else helm "$@"; fi
}

# The copy-pasteable form of the above, for the hints this script prints.
kubectl_hint() {
  if [[ -n "$CONTEXT" ]]; then echo "kubectl --context $CONTEXT"; else echo "kubectl"; fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -n|--namespace)      NAMESPACE="${2:?--namespace needs a value}"; shift 2 ;;
    --context)           CONTEXT="${2:?--context needs a value}"; shift 2 ;;
    --release)           RELEASE="${2:?--release needs a value}"; shift 2 ;;
    --email)             DEV_EMAIL="${2:?--email needs a value}"; shift 2 ;;
    --tenant)            TENANT="${2:?--tenant needs a value}"; shift 2 ;;
    --image)             IMAGE="${2:?--image needs a value}"; shift 2 ;;
    --analytics-db)      ANALYTICS_DB="${2:?--analytics-db needs a value}"; shift 2 ;;
    --identity-db)       IDENTITY_DB="${2:?--identity-db needs a value}"; shift 2 ;;
    --db-secret)         DB_SECRET="${2:?--db-secret needs a value}"; shift 2 ;;
    --pull-secret)       PULL_SECRETS="[{\"name\":\"${2:?--pull-secret needs a value}\"}]"; shift 2 ;;
    --idp-source-type)   IDP_SOURCE_TYPE="${2:?--idp-source-type needs a value}"; shift 2 ;;
    --step)              STEP="${2:?--step needs a value}"; shift 2 ;;
    --days)              WINDOW_DAYS="${2:?--days needs a value}"; shift 2 ;;
    --anchor)            ANCHOR_DATE="${2:?--anchor needs a value}"; shift 2 ;;
    --deadline)          DEADLINE_SECONDS="${2:?--deadline needs a value}"; shift 2 ;;
    --cross-tenant)      CROSS_TENANT="1"; shift ;;
    --force)             FORCE="1"; shift ;;
    --dry-run)           DRY_RUN=1; shift ;;
    --no-follow)         FOLLOW=0; shift ;;
    -h|--help)           usage; exit 0 ;;
    *)                   usage >&2; die "unknown argument: $1" ;;
  esac
done

need kubectl
need envsubst
[[ -f "$JOB_TEMPLATE" ]] || die "Job template not found at $JOB_TEMPLATE."
[[ -n "$NAMESPACE" ]] || { usage >&2; die "--namespace is required."; }
[[ -n "$DEV_EMAIL" ]] || { usage >&2; die "--email is required."; }
case "$STEP" in
  identity|silver|analytics|all) ;;
  *) die "--step must be one of identity, silver, analytics, all (got '$STEP')." ;;
esac
# Checked here rather than by the apiserver: this value is also the script's own
# polling budget, and a non-numeric one turns the wait loop's arithmetic into a
# silent zero — the run would be abandoned the moment it started.
[[ "$DEADLINE_SECONDS" =~ ^[0-9]+$ && "$DEADLINE_SECONDS" -gt 0 ]] \
  || die "--deadline must be a positive whole number of seconds (got '$DEADLINE_SECONDS')."

# The release name is the one thing a namespace cannot answer, so the common
# case (release named after its namespace) is assumed and reported, and the flag
# overrides it. Every value read below is verified to exist, so a wrong guess
# fails naming --release rather than seeding something unintended.
if [[ -z "$RELEASE" ]]; then
  RELEASE="$NAMESPACE"
fi

# Printed, always: the context is ambient unless --context says otherwise, and
# an operator should see which cluster is about to be written to before it is.
resolved_context="$CONTEXT"
[[ -n "$resolved_context" ]] || resolved_context="$(kubectl config current-context 2>/dev/null || true)"
[[ -n "$resolved_context" ]] || die "no kube context is set and --context was not given."
echo "==> stand: context=$resolved_context namespace=$NAMESPACE release=$RELEASE"

# ── Discovery ───────────────────────────────────────────────────────────────
# One ConfigMap read per value keeps each failure attributable; kubectl's
# jsonpath returns empty rather than failing on a missing key, so each result is
# checked against the flag that would supply it.
platform_cm="${RELEASE}-platform"
kube -n "$NAMESPACE" get configmap "$platform_cm" >/dev/null 2>&1 || die \
  "ConfigMap $platform_cm not found in namespace $NAMESPACE. It is generated by
       the umbrella chart and names every infrastructure coordinate this Job needs.
       Check --release (currently '$RELEASE')."

cm_value() {
  # Same reasoning as secret_value below: a failed read must reach the
  # missing-values report, not `set -e`.
  kube -n "$NAMESPACE" get configmap "$platform_cm" -o "jsonpath={.data.$1}" 2>/dev/null || true
}

secret_value() {
  # $1 = secret, $2 = key. An absent secret, an absent key, or a read this
  # caller is not allowed to make all yield an empty string, which every call
  # site treats as "not discovered" and reports through the missing-values list.
  #
  # The `|| true` is on the READ, not on the decode: under `pipefail` a failing
  # kubectl at the head of a pipeline sets the whole pipeline's status, and a
  # bare `X="$(secret_value …)"` assignment would then take `set -e` with it —
  # killing the script before it can name the flag that fixes the problem.
  local encoded
  encoded="$(kube -n "$NAMESPACE" get secret "$1" -o "jsonpath={.data.$2}" 2>/dev/null || true)"
  [[ -n "$encoded" ]] || return 0
  printf '%s' "$encoded" | base64 --decode 2>/dev/null || true
}

MARIADB_HOST="$(cm_value MARIADB_HOST)"
MARIADB_PORT="$(cm_value MARIADB_PORT)"
MARIADB_USER="$(cm_value MARIADB_USERNAME)"
CLICKHOUSE_HOST="$(cm_value CLICKHOUSE_HOST)"
CLICKHOUSE_HTTP_PORT="$(cm_value CLICKHOUSE_PORT)"
CLICKHOUSE_USER="$(cm_value CLICKHOUSE_USER)"
CLICKHOUSE_DATABASE="$(cm_value CLICKHOUSE_DATABASE)"

# The product database is where a chart-deployed stand's analytics migrations
# created the catalogue tables. Preflight verifies that inside the Job.
if [[ -z "$ANALYTICS_DB" ]]; then
  ANALYTICS_DB="$(cm_value MARIADB_DATABASE)"
fi

# The seeder speaks HTTP to ClickHouse (see config.ClickHouse.url), so a stand
# fronting it with TLS would be dialled on the wrong scheme — said now rather
# than as a connection error inside the Job.
platform_ch_url="$(cm_value CLICKHOUSE_URL)"
case "$platform_ch_url" in
  https://*)
    die "this stand's ClickHouse is $platform_ch_url, and the seeder speaks plain HTTP only.
       Point it at an HTTP endpoint with --release/--image overrides, or teach
       config.ClickHouse.url a scheme first." ;;
esac

ir_secret="insight-identity-resolution-config"
# Told apart from a missing key: an unreadable Secret is an access problem, and
# reporting it as "pass --tenant" would send the operator after the wrong thing.
if ! kube -n "$NAMESPACE" get secret "$ir_secret" -o name >/dev/null 2>&1; then
  echo "WARNING: Secret $ir_secret is absent or not readable in namespace $NAMESPACE;" >&2
  echo "         the tenant and identity database cannot be discovered from it." >&2
fi

if [[ -z "$TENANT" ]]; then
  # The gear name is spelled with hyphens on some chart versions and
  # underscores on others; both are the same value.
  TENANT="$(secret_value "$ir_secret" 'APP__gears__identity-resolution__config__tenant_default_id')"
  if [[ -z "$TENANT" ]]; then
    TENANT="$(secret_value "$ir_secret" 'APP__gears__identity_resolution__config__tenant_default_id')"
  fi
fi

if [[ -z "$IDENTITY_DB" ]]; then
  for key in 'APP__gears__identity-resolution__config__database_url' \
             'APP__gears__identity_resolution__config__database_url'; do
    url="$(secret_value "$ir_secret" "$key")"
    # Database name is the last path segment, minus any query string. The URL
    # also carries a password, so it is never echoed — only this segment leaves
    # the expansion.
    if [[ -n "$url" ]]; then
      candidate="${url##*/}"
      candidate="${candidate%%\?*}"
      # A URL with no path segment leaves the AUTHORITY here — which carries the
      # password. Only an identifier-shaped result is a database name; anything
      # else is dropped so no credential can reach the rendered manifest.
      if [[ "$candidate" =~ ^[A-Za-z0-9_]+$ ]]; then
        IDENTITY_DB="$candidate"
        break
      fi
    fi
  done
fi

if [[ -z "$IDP_SOURCE_TYPE" ]]; then
  IDP_SOURCE_TYPE="$(secret_value insight-authenticator-config \
    'APP__gears__authenticator__config__idp__source_type')"
fi

if [[ -z "$IMAGE" || -z "$PULL_SECRETS" ]]; then
  need helm
  need jq
  # `|| true`: a helm failure (no such release, no permission) must reach the
  # missing-values report below naming --image, not kill the script through
  # pipefail with nothing said.
  release_values="$(helm_release get values "$RELEASE" -n "$NAMESPACE" -a -o json 2>/dev/null || true)"
  if [[ -n "$release_values" ]]; then
    if [[ -z "$IMAGE" ]]; then
      # `ingestion.seedImage`, NOT the toolbox: the seeder ships in an image of
      # its own so the operator toolbox carries no demo data. A release that
      # never set it cannot be seeded until someone names one.
      IMAGE="$(printf '%s' "$release_values" | jq -r '.ingestion.seedImage // empty')"
    fi
    if [[ -z "$PULL_SECRETS" ]]; then
      # Rendered as a YAML flow sequence so the template needs no conditional:
      # the release's own secrets, or an empty list.
      PULL_SECRETS="$(printf '%s' "$release_values" \
        | jq -c '[(.global.imagePullSecrets // [])[] | if type == "string" then {name: .} else . end]')"
    fi
  fi
fi
[[ -n "$PULL_SECRETS" ]] || PULL_SECRETS="[]"

if [[ -z "$DB_SECRET" ]]; then
  # Discovered by looking for BOTH keys the Job references rather than by
  # assuming the name: a Secret carrying only one of them would render a Job
  # that fails at pod creation on the missing key.
  for candidate in insight-db-creds "${RELEASE}-db-creds"; do
    have_maria="$(kube -n "$NAMESPACE" get secret "$candidate" \
      -o 'jsonpath={.data.mariadb-password}' 2>/dev/null || true)"
    have_ch="$(kube -n "$NAMESPACE" get secret "$candidate" \
      -o 'jsonpath={.data.clickhouse-password}' 2>/dev/null || true)"
    if [[ -n "$have_maria" && -n "$have_ch" ]]; then
      DB_SECRET="$candidate"
      break
    fi
  done
fi

# ── Every value, or the flag that supplies it ───────────────────────────────
# Accumulated as newline-delimited text rather than an array: `${#arr[@]}` on an
# empty array is an unbound-variable error under `set -u` in bash 3.2, which is
# what /bin/bash still is on macOS.
missing=""
missing_count=0
check() {
  # $1 = value, $2 = what it is, $3 = flag that overrides it
  if [[ -z "$1" ]]; then
    missing="${missing}  - $2 — pass $3"$'\n'
    missing_count=$((missing_count + 1))
  fi
}
check "$MARIADB_HOST" "MariaDB host (ConfigMap $platform_cm, MARIADB_HOST)" "--release"
check "$MARIADB_PORT" "MariaDB port (ConfigMap $platform_cm, MARIADB_PORT)" "--release"
check "$MARIADB_USER" "MariaDB user (ConfigMap $platform_cm, MARIADB_USERNAME)" "--release"
check "$CLICKHOUSE_HOST" "ClickHouse host (ConfigMap $platform_cm)" "--release"
check "$CLICKHOUSE_HTTP_PORT" "ClickHouse HTTP port (ConfigMap $platform_cm)" "--release"
check "$CLICKHOUSE_USER" "ClickHouse user (ConfigMap $platform_cm)" "--release"
check "$CLICKHOUSE_DATABASE" "ClickHouse database (ConfigMap $platform_cm)" "--release"
check "$ANALYTICS_DB" "analytics catalogue database" "--analytics-db"
check "$IDENTITY_DB" "identity database (Secret $ir_secret, database_url)" "--identity-db"
check "$TENANT" "stand tenant (Secret $ir_secret, tenant_default_id)" "--tenant"
check "$IMAGE" \
  "the seeder's image. The chart carries it as ingestion.seedImage, empty by
    default because seeding is a test-stand activity; CI publishes the image as
    insight-seed alongside every toolbox build" \
  "--image"
check "$DB_SECRET" "database-credentials Secret" "--db-secret"
check "$IDP_SOURCE_TYPE" \
  "the identity source_type the stand's logins resolve under. Newer charts
    publish it as authenticator.oidc.sourceType; a chart that predates that field
    resolves logins by email instead, and any stable label (e.g. 'keycloak') is
    then correct as long as it matches what the authenticator will use later" \
  "--idp-source-type"

if [[ "$missing_count" -gt 0 ]]; then
  echo "ERROR: could not resolve $missing_count value(s) from namespace $NAMESPACE:" >&2
  printf '%s' "$missing" >&2
  exit 1
fi

echo "==> tenant:    $TENANT"
echo "==> databases: identity=$IDENTITY_DB analytics=$ANALYTICS_DB clickhouse=$CLICKHOUSE_DATABASE"
echo "==> image:     $IMAGE"
echo "==> idp:       source_type=$IDP_SOURCE_TYPE dev_user=$DEV_EMAIL"

# ── Render ──────────────────────────────────────────────────────────────────
# A name per run: a Job's pod spec is immutable, so reusing one name would make
# a re-run fail on an unrelated conflict instead of seeding.
job_name="insight-seed-${STEP}-$(date -u +%Y%m%d%H%M%S)"

export SEED_JOB_NAME="$job_name"
export SEED_NAMESPACE="$NAMESPACE"
export SEED_IMAGE="$IMAGE"
export SEED_STEP="$STEP"
export SEED_DEADLINE_SECONDS="$DEADLINE_SECONDS"
export SEED_DB_SECRET="$DB_SECRET"
export SEED_MARIADB_HOST="$MARIADB_HOST"
export SEED_MARIADB_PORT="$MARIADB_PORT"
export SEED_MARIADB_USER="$MARIADB_USER"
export SEED_IDENTITY_DB="$IDENTITY_DB"
export SEED_ANALYTICS_DB="$ANALYTICS_DB"
export SEED_CLICKHOUSE_HOST="$CLICKHOUSE_HOST"
export SEED_CLICKHOUSE_HTTP_PORT="$CLICKHOUSE_HTTP_PORT"
export SEED_CLICKHOUSE_USER="$CLICKHOUSE_USER"
export SEED_CLICKHOUSE_DATABASE="$CLICKHOUSE_DATABASE"
export SEED_TENANT_ID="$TENANT"
export SEED_DEV_USER_EMAIL="$DEV_EMAIL"
export SEED_IDP_SOURCE_TYPE="$IDP_SOURCE_TYPE"
export SEED_CROSS_TENANT="$CROSS_TENANT"
export SEED_FORCE="$FORCE"
# Deliberately allowed to be empty: the seeder documents its own window, and an
# empty value here means "use it" rather than a second copy of that default.
export SEED_WINDOW_DAYS="$WINDOW_DAYS"
export SEED_ANCHOR_DATE="$ANCHOR_DATE"
export SEED_PULL_SECRETS="$PULL_SECRETS"

# Only the seed variables are substituted, so a `$HOME` or `$PATH` in the
# template stays literal.
manifest="$(envsubst '
  ${SEED_JOB_NAME} ${SEED_NAMESPACE} ${SEED_IMAGE} ${SEED_STEP}
  ${SEED_DEADLINE_SECONDS} ${SEED_DB_SECRET}
  ${SEED_MARIADB_HOST} ${SEED_MARIADB_PORT} ${SEED_MARIADB_USER}
  ${SEED_IDENTITY_DB} ${SEED_ANALYTICS_DB}
  ${SEED_CLICKHOUSE_HOST} ${SEED_CLICKHOUSE_HTTP_PORT} ${SEED_CLICKHOUSE_USER}
  ${SEED_CLICKHOUSE_DATABASE}
  ${SEED_TENANT_ID} ${SEED_DEV_USER_EMAIL} ${SEED_IDP_SOURCE_TYPE}
  ${SEED_CROSS_TENANT} ${SEED_FORCE} ${SEED_WINDOW_DAYS} ${SEED_ANCHOR_DATE}
  ${SEED_PULL_SECRETS}
' < "$JOB_TEMPLATE")"

if [[ "$DRY_RUN" -eq 1 ]]; then
  printf '%s\n' "$manifest"
  exit 0
fi

printf '%s\n' "$manifest" | kube apply -f -
echo "==> applied Job $job_name"

if [[ "$FOLLOW" -eq 0 ]]; then
  echo "    follow it with: $(kubectl_hint) -n $NAMESPACE logs -f job/$job_name"
  exit 0
fi

# Wait for the pod's CONTAINER to start, not just for the pod object: `logs -f`
# against a pod still in ContainerCreating fails immediately, and the run would
# then finish with nothing streamed. A pod that cannot start at all is caught by
# the poll loop below, so this wait is bounded and never fatal.
for _ in $(seq 1 60); do
  phase="$(kube -n "$NAMESPACE" get pod -l "job-name=$job_name" \
    -o 'jsonpath={.items[0].status.phase}' 2>/dev/null || true)"
  case "$phase" in
    Running|Succeeded|Failed) break ;;
  esac
  sleep 2
done

kube -n "$NAMESPACE" logs -f "job/$job_name" || true

# The log stream ending is not the verdict — read it from the Job. Polled rather
# than `kubectl wait --for=condition=complete`, which only knows how to wait for
# success: a refused seed would sit there until the timeout instead of reporting
# in the second it failed.
deadline=$((SECONDS + DEADLINE_SECONDS))
while [[ "$SECONDS" -lt "$deadline" ]]; do
  # `|| true` on every read: a transient apiserver hiccup inside a loop that may
  # run for an hour must not kill the script through `set -e`.
  succeeded="$(kube -n "$NAMESPACE" get "job/$job_name" \
    -o 'jsonpath={.status.succeeded}' 2>/dev/null || true)"
  failed="$(kube -n "$NAMESPACE" get "job/$job_name" \
    -o 'jsonpath={.status.failed}' 2>/dev/null || true)"
  if [[ "${succeeded:-0}" -ge 1 ]]; then
    echo "==> seed complete: $job_name"
    exit 0
  fi
  if [[ "${failed:-0}" -ge 1 ]]; then
    echo "ERROR: Job $job_name failed. Its logs above hold the reason; it is not" >&2
    echo "       retried (backoffLimit 0) and survives for an hour" >&2
    echo "       (ttlSecondsAfterFinished), so it can be read again until then:" >&2
    echo "         $(kubectl_hint) -n $NAMESPACE logs job/$job_name" >&2
    exit 1
  fi

  # A pod that cannot start never becomes either, and waiting out the deadline
  # for it would be an hour of silence. The common cause is an image the cluster
  # cannot pull — including a locally built one on a remote cluster.
  waiting="$(kube -n "$NAMESPACE" get pod -l "job-name=$job_name" \
    -o 'jsonpath={.items[0].status.containerStatuses[0].state.waiting.reason}' 2>/dev/null || true)"
  case "$waiting" in
    ImagePullBackOff|ErrImagePull|InvalidImageName)
      echo "ERROR: the seed pod cannot start: $waiting for image $IMAGE." >&2
      echo "       Push the image somewhere the cluster can pull it, or pass a tag it already has." >&2
      exit 1 ;;
  esac

  sleep 3
done

echo "ERROR: Job $job_name neither completed nor failed within ${DEADLINE_SECONDS}s:" >&2
echo "         $(kubectl_hint) -n $NAMESPACE describe job/$job_name" >&2
exit 1
