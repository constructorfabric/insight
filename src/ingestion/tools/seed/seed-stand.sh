#!/usr/bin/env bash
# Seed a Kubernetes stand with the demo organisation and its activity. An
# `all`/`silver` run finishes by resolving identity (persons-seed, which
# publishes its own log) and rebuilding gold, so the stand serves real metrics rather than a null for
# every person. Every coordinate is discovered from the cluster, never copied in
# by an operator; credentials are never read. Nothing is defaulted.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JOB_TEMPLATE="$SCRIPT_DIR/seed-job.yaml.tpl"

# RULE-DEFAULTS-OK: the seeder's own documented default; every step is
# idempotent, so seeding everything is the safe reading of "seed this stand".
STEP="all"
# RULE-DEFAULTS-OK: wall-clock ceiling for a pod, not a config input.
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
Usage: seed-stand.sh -n <namespace> [options]

Runs the demo-data seeder as a one-shot Job on a chart-deployed stand, using the
seeder image the release already pins.

Required:
  -n, --namespace <ns>     namespace the Insight release runs in

Discovered from the stand (pass a flag only to override):
      --context <name>     kube context to act on         [default: the current one]
      --release <name>     helm release name                 [default: same as -n]
      --email <address>    persona the dev-lead login resolves to. A user with
                           this email must already exist in the stand's IdP —
                           the authenticator resolves people by the email claim —
                           which is precisely why it is read back out of the realm
                           the stand applies rather than supplied: the realm and
                           the rows this Job writes are two projections of one
                           roster, and the realm is the one that already exists.
                           Pass it for a stand whose realm came from somewhere
                           else; a supplied address that disagrees with the realm
                           is used, loudly.
      --tenant <uuid>      tenant every seeded row is scoped to
      --image <ref>        seeder image to run (chart: ingestion.seedImage)
      --analytics-db <db>  database holding metric_definitions
      --identity-db <db>   database holding persons
      --db-secret <name>   Secret holding mariadb-password + clickhouse-password
      --pull-secret <name> image-pull Secret (default: the release's own)
      --idp-source-type <t> identity source_type the login rows are written under

Seed options:
      --step <step>        identity | silver | analytics | gold | all [default: all]
                           `all`/`silver` finish by running the identity
                           projection (persons-seed, which publishes) and rebuilding
                           gold, so the stand serves resolved metrics. `gold`
                           alone just rebuilds over the map as it stands now.
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
  seed-stand.sh -n insight --dry-run
  seed-stand.sh -n insight
  seed-stand.sh -n insight --step identity
  seed-stand.sh -n insight --email you@example.com   # realm provisioned elsewhere
USAGE
}

die() {
  echo "ERROR: $*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required but not on PATH."
}

# Target is --context, never whatever the shell was last pointed at.
kube() {
  if [[ -n "$CONTEXT" ]]; then kubectl --context "$CONTEXT" "$@"; else kubectl "$@"; fi
}

helm_release() {
  if [[ -n "$CONTEXT" ]]; then helm --kube-context "$CONTEXT" "$@"; else helm "$@"; fi
}

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
# --email is NOT gated here: discovery below needs the cluster first. `all` and
# `silver` finish by running the projection + gold rebuild themselves (see tail).
case "$STEP" in
  identity|silver|analytics|gold|all) ;;
  *) die "--step must be one of identity, silver, analytics, gold, all (got '$STEP')." ;;
esac
# Also the poll loop's own budget; a non-numeric value would silently zero it.
[[ "$DEADLINE_SECONDS" =~ ^[0-9]+$ && "$DEADLINE_SECONDS" -gt 0 ]] \
  || die "--deadline must be a positive whole number of seconds (got '$DEADLINE_SECONDS')."

# Assumed from the namespace; every value below is verified, so a wrong
# guess fails naming --release.
if [[ -z "$RELEASE" ]]; then
  RELEASE="$NAMESPACE"
fi

resolved_context="$CONTEXT"
[[ -n "$resolved_context" ]] || resolved_context="$(kubectl config current-context 2>/dev/null || true)"
[[ -n "$resolved_context" ]] || die "no kube context is set and --context was not given."
echo "==> stand: context=$resolved_context namespace=$NAMESPACE release=$RELEASE"

# One ConfigMap read per value keeps each failure attributable; jsonpath
# returns empty rather than failing on a missing key.
platform_cm="${RELEASE}-platform"
kube -n "$NAMESPACE" get configmap "$platform_cm" >/dev/null 2>&1 || die \
  "ConfigMap $platform_cm not found in namespace $NAMESPACE. It is generated by
       the umbrella chart and names every infrastructure coordinate this Job needs.
       Check --release (currently '$RELEASE')."

cm_value() {
  # Failed read reaches the missing-values report, not `set -e`.
  kube -n "$NAMESPACE" get configmap "$platform_cm" -o "jsonpath={.data.$1}" 2>/dev/null || true
}

secret_value() {
  # $1 = secret, $2 = key; `|| true` on the READ, or `set -e` kills the
  # script under `pipefail` before it can name the flag that fixes it.
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

# Where analytics migrations created the catalogue tables; the Job's own
# preflight verifies it.
if [[ -z "$ANALYTICS_DB" ]]; then
  ANALYTICS_DB="$(cm_value MARIADB_DATABASE)"
fi

# The seeder speaks HTTP to ClickHouse only.
platform_ch_url="$(cm_value CLICKHOUSE_URL)"
case "$platform_ch_url" in
  https://*)
    die "this stand's ClickHouse is $platform_ch_url, and the seeder speaks plain HTTP only.
       Point it at an HTTP endpoint with --release/--image overrides, or teach
       config.ClickHouse.url a scheme first." ;;
esac

ir_secret="insight-identity-resolution-config"
# Unreadable is an access problem, not "pass --tenant".
if ! kube -n "$NAMESPACE" get secret "$ir_secret" -o name >/dev/null 2>&1; then
  echo "WARNING: Secret $ir_secret is absent or not readable in namespace $NAMESPACE;" >&2
  echo "         the tenant and identity database cannot be discovered from it." >&2
fi

if [[ -z "$TENANT" ]]; then
  # Hyphens on some chart versions, underscores on others; same value.
  TENANT="$(secret_value "$ir_secret" 'APP__gears__identity-resolution__config__tenant_default_id')"
  if [[ -z "$TENANT" ]]; then
    TENANT="$(secret_value "$ir_secret" 'APP__gears__identity_resolution__config__tenant_default_id')"
  fi
fi

if [[ -z "$IDENTITY_DB" ]]; then
  for key in 'APP__gears__identity-resolution__config__database_url' \
             'APP__gears__identity_resolution__config__database_url'; do
    url="$(secret_value "$ir_secret" "$key")"
    # Last path segment only; the URL also carries a password.
    if [[ -n "$url" ]]; then
      candidate="${url##*/}"
      candidate="${candidate%%\?*}"
      # A path-less URL leaves the password-bearing AUTHORITY here; only an
      # identifier-shaped result is accepted.
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

# Declared out here: skipping the fetch below (both --image and
# --pull-secret given) must not leave an unset variable under `set -u`.
release_values=""
if [[ -z "$IMAGE" || -z "$PULL_SECRETS" ]]; then
  need helm
  need jq
  # `|| true`: a helm failure must reach the missing-values report, not
  # kill the script through pipefail with nothing said.
  release_values="$(helm_release get values "$RELEASE" -n "$NAMESPACE" -a -o json 2>/dev/null || true)"
  if [[ -n "$release_values" ]]; then
    if [[ -z "$IMAGE" ]]; then
      # ingestion.seedImage, NOT the toolbox: the seeder ships in its own
      # image so the operator toolbox carries no demo data.
      IMAGE="$(printf '%s' "$release_values" | jq -r '.ingestion.seedImage // empty')"
    fi
    if [[ -z "$PULL_SECRETS" ]]; then
      PULL_SECRETS="$(printf '%s' "$release_values" \
        | jq -c '[(.global.imagePullSecrets // [])[] | if type == "string" then {name: .} else . end]')"
    fi
  fi
fi
[[ -n "$PULL_SECRETS" ]] || PULL_SECRETS="[]"

if [[ -z "$DB_SECRET" ]]; then
  # Looks for BOTH keys the Job references, not just the name.
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

# Realm and identity.persons are two projections of ONE roster (INFRA.md,
# "TEST_STAND_SEED_EMAIL drift"). Dev-lead = roster UUID, never a name match.
DEV_EMAIL_ORIGIN="--email"
realm_dev_email=""
realms_cm=""

# Scraped, not copied: fail-open, a renamed or reformatted DEV_LEAD_UUID
# scrapes nothing, the realm read-back is skipped and --email is demanded.
roster_module="$SCRIPT_DIR/insight_seed/profiles.py"
dev_lead_uuid=""
if [[ -r "$roster_module" ]]; then
  dev_lead_uuid="$(sed -n \
    's/^DEV_LEAD_UUID[[:space:]]*=[[:space:]]*"\([0-9a-fA-F-]\{36\}\)".*/\1/p' \
    "$roster_module" 2>/dev/null | head -n1 || true)"
fi

# Top up release_values if the image/pull-secret block never fetched them.
# `command -v`, not `need`: an absent tool means "discovered nothing" here.
if [[ -z "$release_values" ]] \
   && command -v helm >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  release_values="$(helm_release get values "$RELEASE" -n "$NAMESPACE" -a -o json 2>/dev/null || true)"
fi

# Explicit override first, then the chart's fullname helper — NOT platform_cm's
# bare .Release.Name. Fail-open: a wrong guess only reaches the warning below.
if [[ -n "$release_values" ]] && command -v jq >/dev/null 2>&1; then
  # Gated on keycloakConfig.enabled: the ConfigMap has no owner reference, so
  # a disabled stand can still carry a stale one from a previous bring-up.
  if [[ "$(printf '%s' "$release_values" | jq -r '.keycloakConfig.enabled // false')" == "true" ]]; then
    realms_cm="$(printf '%s' "$release_values" | jq -r '.keycloakConfig.realmsConfigMap // empty')"
    if [[ -z "$realms_cm" ]]; then
      realm_fullname="$(printf '%s' "$release_values" | jq -r '.fullnameOverride // empty')"
      [[ -n "$realm_fullname" ]] || realm_fullname="$RELEASE"
      realm_fullname="${realm_fullname:0:63}"
      realm_fullname="${realm_fullname%-}"
      realms_cm="${realm_fullname}-keycloak-config-realms"
    fi
  fi
fi

if [[ -n "$realms_cm" && -n "$dev_lead_uuid" ]] && command -v jq >/dev/null 2>&1; then
  # Absent/unreadable is a bring-up or access problem, told apart from "no
  # match" so the warning does not send the operator after the wrong thing.
  if ! kube -n "$NAMESPACE" get configmap "$realms_cm" -o name >/dev/null 2>&1; then
    echo "WARNING: ConfigMap $realms_cm is absent or not readable in namespace $NAMESPACE;" >&2
    echo "         the dev-lead address cannot be read from the realm this stand applies." >&2
    echo "         Deploy the stand's realm first, or pass --email to name the persona." >&2
  else
    # Only the matched address leaves this expansion; `realm_json` also carries
    # the client secret and is never echoed. `ascii_downcase` matches the seeder.
    realm_json="$(kube -n "$NAMESPACE" get configmap "$realms_cm" -o json 2>/dev/null || true)"
    realm_matches="$(printf '%s' "$realm_json" | jq -r --arg id "$dev_lead_uuid" '
      [ (.data // {})[]
        | (fromjson? | .users? // empty)
        | .[]?
        | select(
            ((.id? | strings | ascii_downcase) == ($id | ascii_downcase))
            or (([.attributes.idp_sub?] | flatten | map(strings) | map(ascii_downcase))
                 | index($id | ascii_downcase))
          )
        | (.email? // .username?) | strings | ascii_downcase
      ] | unique | .[]' 2>/dev/null || true)"

    match_count=0
    if [[ -n "$realm_matches" ]]; then
      match_count="$(printf '%s\n' "$realm_matches" | wc -l | tr -d '[:space:]')"
    fi

    if [[ "$match_count" -gt 1 ]]; then
      # Named, not resolved: jq's key order is stable but arbitrary, so
      # picking the first would hand the stand a dev-lead nobody chose.
      echo "WARNING: ConfigMap $realms_cm names more than one dev-lead address" >&2
      echo "         (users carrying the roster UUID $dev_lead_uuid):" >&2
      while IFS= read -r realm_addr; do
        if [[ -n "$realm_addr" ]]; then
          echo "           $realm_addr" >&2
        fi
      done <<< "$realm_matches"
      echo "         That is more than one roster packed into one realm ConfigMap." >&2
      echo "         Pack a single roster realm, or pass --email to say which persona" >&2
      echo "         this seed is for." >&2
    elif [[ "$match_count" -eq 1 ]]; then
      # Shape-asserted before use: substituted into the Job template, so a
      # newline or quote would render a broken manifest, not a bad address.
      realm_email_shape='^[^[:space:]"]+@[^[:space:]"]+$'
      if [[ "$realm_matches" =~ $realm_email_shape ]]; then
        realm_dev_email="$realm_matches"
      else
        echo "WARNING: the user carrying the roster UUID $dev_lead_uuid in ConfigMap" >&2
        echo "         $realms_cm has no address-shaped email; ignoring it." >&2
        echo "         Fix the realm, or pass --email to name the persona." >&2
      fi
    fi
  fi
fi

if [[ -z "$DEV_EMAIL" ]]; then
  DEV_EMAIL="$realm_dev_email"
  [[ -z "$DEV_EMAIL" ]] || DEV_EMAIL_ORIGIN="ConfigMap $realms_cm"
elif [[ -n "$realm_dev_email" ]]; then
  # bash 3.2 (macOS /bin/bash) has no ${var,,}, hence `tr` for case-folding.
  supplied_folded="$(printf '%s' "$DEV_EMAIL" | tr '[:upper:]' '[:lower:]')"
  if [[ "$supplied_folded" != "$realm_dev_email" ]]; then
    # A warning, not a refusal: an explicit --email wins, but this is the one
    # disagreement that fails silently downstream (the Job still SUCCEEDS).
    echo "WARNING: --email $DEV_EMAIL disagrees with the realm this stand applies." >&2
    echo "         ConfigMap $realms_cm names the dev-lead as $realm_dev_email" >&2
    echo "         (the realm user whose id is the roster's DEV_LEAD_UUID $dev_lead_uuid)." >&2
    echo "         Seeding with the supplied address, $DEV_EMAIL, because an" >&2
    echo "         explicit flag wins — but the realm and identity.persons are two" >&2
    echo "         projections of ONE roster and they now describe different people." >&2
    echo "         The consequence is silent: the dev-lead login authenticates" >&2
    echo "         against the realm and resolves to nobody, every other persona" >&2
    echo "         signs in normally, every pod stays Ready and the release still" >&2
    echo "         reports 'deployed'." >&2
    echo "         Drop --email to seed the address the realm already carries." >&2
  fi
fi

# Newline-delimited text, not an array: `${#arr[@]}` on an empty array is an
# unbound-variable error under `set -u` in bash 3.2 (still /bin/bash on macOS).
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
realms_cm_reported="$realms_cm"
[[ -n "$realms_cm_reported" ]] || realms_cm_reported="none — this release does not enable keycloakConfig, or its values could not be read"
check "$DEV_EMAIL" \
  "the dev-lead persona's address. It is read from the realm the stand applies
    (ConfigMap: $realms_cm_reported) — the user whose id is the roster's
    DEV_LEAD_UUID — so the realm and the seeded persons table stay one roster.
    A stand whose realm is provisioned some other way has to name the persona
    itself, and the address must already exist as a user in that stand's IdP" \
  "--email"
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
# Origin printed, not just the value: shows whether the seed follows the
# realm or overrides it, before anything is written.
echo "==> idp:       source_type=$IDP_SOURCE_TYPE dev_user=$DEV_EMAIL (from $DEV_EMAIL_ORIGIN)"

# The seeder Job's shared environment. SEED_STEP and SEED_JOB_NAME are the only
# per-step values, so run_seed_step sets them for each Job it applies.
export SEED_NAMESPACE="$NAMESPACE"
export SEED_IMAGE="$IMAGE"
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
# Allowed to be empty: the seeder has its own default window.
export SEED_WINDOW_DAYS="$WINDOW_DAYS"
export SEED_ANCHOR_DATE="$ANCHOR_DATE"
export SEED_PULL_SECRETS="$PULL_SECRETS"

# Renders the Job manifest for the step in $SEED_STEP / name in $SEED_JOB_NAME.
# Only the seed variables are substituted; a `$HOME`/`$PATH` stays literal.
render_seed_manifest() {
  envsubst '
    ${SEED_JOB_NAME} ${SEED_NAMESPACE} ${SEED_IMAGE} ${SEED_STEP}
    ${SEED_DEADLINE_SECONDS} ${SEED_DB_SECRET}
    ${SEED_MARIADB_HOST} ${SEED_MARIADB_PORT} ${SEED_MARIADB_USER}
    ${SEED_IDENTITY_DB} ${SEED_ANALYTICS_DB}
    ${SEED_CLICKHOUSE_HOST} ${SEED_CLICKHOUSE_HTTP_PORT} ${SEED_CLICKHOUSE_USER}
    ${SEED_CLICKHOUSE_DATABASE}
    ${SEED_TENANT_ID} ${SEED_DEV_USER_EMAIL} ${SEED_IDP_SOURCE_TYPE}
    ${SEED_CROSS_TENANT} ${SEED_FORCE} ${SEED_WINDOW_DAYS} ${SEED_ANCHOR_DATE}
    ${SEED_PULL_SECRETS}
  ' < "$JOB_TEMPLATE"
}

# print_manifest_sentinel NAME — re-read the finished Job's log and print the
# one manifest line, or nothing for a step that writes no manifest. Read from
# the completed Job rather than taken from the stream below, because the
# seeder prints it last and `logs -f` can end without the container's final
# lines; a consumer that lost it cannot tell a seeded stand from an attempt.
print_manifest_sentinel() {
  kube -n "$NAMESPACE" logs "job/$1" --tail=-1 2>/dev/null \
    | grep -m1 '^SEED_MANIFEST_JSON: ' || true
}

# wait_for_job NAME — follow the pod's logs, then read the verdict from the Job
# object (the log stream ending is not the verdict). 0 on success, 1 otherwise;
# never fatal on a transient apiserver read.
wait_for_job() {
  local job_name="$1" phase succeeded failed waiting deadline

  # Waits for the CONTAINER, not just the pod object: `logs -f` against a pod
  # still in ContainerCreating fails immediately. Bounded, and never fatal.
  for _ in $(seq 1 60); do
    phase="$(kube -n "$NAMESPACE" get pod -l "job-name=$job_name" \
      -o 'jsonpath={.items[0].status.phase}' 2>/dev/null || true)"
    case "$phase" in Running|Succeeded|Failed) break ;; esac
    sleep 2
  done

  # The sentinel is filtered out here and re-emitted from the finished Job on
  # success, so the log carries it exactly once and never a truncated copy.
  kube -n "$NAMESPACE" logs -f "job/$job_name" | grep -v '^SEED_MANIFEST_JSON: ' || true

  # Polled, not `kubectl wait --for=condition=complete`, which only waits for
  # success. `|| true`: a transient apiserver hiccup must not kill the loop.
  deadline=$((SECONDS + DEADLINE_SECONDS))
  while [[ "$SECONDS" -lt "$deadline" ]]; do
    succeeded="$(kube -n "$NAMESPACE" get "job/$job_name" \
      -o 'jsonpath={.status.succeeded}' 2>/dev/null || true)"
    failed="$(kube -n "$NAMESPACE" get "job/$job_name" \
      -o 'jsonpath={.status.failed}' 2>/dev/null || true)"
    if [[ "${succeeded:-0}" -ge 1 ]]; then
      print_manifest_sentinel "$job_name"
      echo "==> job complete: $job_name"
      return 0
    fi
    if [[ "${failed:-0}" -ge 1 ]]; then
      # CI publishes only `==>`/`ERROR:` lines from this script's output, so
      # re-emit the preflight refusal from the failed Job's log with an
      # ERROR: prefix — otherwise the reason never reaches the CI log.
      # Bounded on purpose: this is the one place pod output crosses into the
      # allowlisted channel of a public repository, and a refusal is a handful
      # of lines. Anything past the cap stays where the rest of the log is.
      kube -n "$NAMESPACE" logs "job/$job_name" --tail=-1 2>/dev/null \
        | sed -n '/PreflightError: /,$p' | head -40 | sed 's/^/ERROR: /' >&2 || true
      echo "ERROR: Job $job_name failed. Its full log holds the reason; it is not" >&2
      echo "       retried (backoffLimit 0) and survives for an hour" >&2
      echo "       (ttlSecondsAfterFinished), so it can be read again until then:" >&2
      echo "         $(kubectl_hint) -n $NAMESPACE logs job/$job_name" >&2
      return 1
    fi

    # A pod that cannot start never becomes either; the common cause is an
    # image the cluster cannot pull.
    waiting="$(kube -n "$NAMESPACE" get pod -l "job-name=$job_name" \
      -o 'jsonpath={.items[0].status.containerStatuses[0].state.waiting.reason}' 2>/dev/null || true)"
    case "$waiting" in
      ImagePullBackOff|ErrImagePull|InvalidImageName)
        echo "ERROR: the pod cannot start: $waiting for image $IMAGE." >&2
        echo "       Push the image somewhere the cluster can pull it, or pass a tag it already has." >&2
        return 1 ;;
    esac

    sleep 3
  done

  echo "ERROR: Job $job_name neither completed nor failed within ${DEADLINE_SECONDS}s:" >&2
  echo "         $(kubectl_hint) -n $NAMESPACE describe job/$job_name" >&2
  return 1
}

# run_seed_step STEP — apply one seeder Job for STEP and wait for its verdict.
run_seed_step() {
  local step="$1" job_name
  # A name per run: a Job's pod spec is immutable.
  job_name="insight-seed-${step}-$(date -u +%Y%m%d%H%M%S)"
  export SEED_STEP="$step"
  export SEED_JOB_NAME="$job_name"
  render_seed_manifest | kube apply -f -
  echo "==> applied Job $SEED_JOB_NAME (step: $step)"
  wait_for_job "$SEED_JOB_NAME"
}

# project_identity — force the persons-seed CronJob to run now and wait for it:
# a fresh CI stand cannot wait for the next scheduled tick. The run LINKS each connector
# account to the seeded roster person by e-mail (resolve_assignments'
# LinkedByEmail), APPENDS to `persons` (INSERT IGNORE — it never rewrites the
# seeder's login rows), and publishes to ClickHouse identity_persons, where
# gold's resolve_person_id() reads it. Without this, gold resolves nothing and
# the API answers 200 with a null for every person metric. Discovered by
# component label, never named, so a chart rename fails loudly here rather than
# silently skipping the projection.
project_identity() {
  local component=persons-seed cronjob job_name
  cronjob="$(kube -n "$NAMESPACE" get cronjob \
    -l "app.kubernetes.io/component=$component" \
    -o 'jsonpath={.items[0].metadata.name}' 2>/dev/null || true)"
  if [[ -z "$cronjob" ]]; then
    echo "ERROR: no $component CronJob in namespace $NAMESPACE — the identity" >&2
    echo "       projection cannot run, so gold would resolve to a null for every" >&2
    echo "       person. Enable identityResolution.seed.enabled on this stand," >&2
    echo "       or seed --step silver and run the projection yourself." >&2
    return 1
  fi
  job_name="${cronjob}-ci-$(date -u +%Y%m%d%H%M%S)"
  echo "==> identity projection: $component (job/$job_name from cronjob/$cronjob)"
  kube -n "$NAMESPACE" create job --from="cronjob/$cronjob" "$job_name"
  wait_for_job "$job_name" || {
    echo "ERROR: $component did not complete; gold would stay unresolved." >&2
    return 1
  }
}

# --dry-run: print the requested step's manifest and exit, writing nothing.
if [[ "$DRY_RUN" -eq 1 ]]; then
  dry_job_name="insight-seed-${STEP}-$(date -u +%Y%m%d%H%M%S)"
  export SEED_STEP="$STEP"
  export SEED_JOB_NAME="$dry_job_name"
  render_seed_manifest
  exit 0
fi

# --no-follow: apply the requested step and return without waiting. The identity
# projection and gold rebuild need the seed to finish first, so they are skipped
# here — a no-follow caller has asked for the raw apply only.
if [[ "$FOLLOW" -eq 0 ]]; then
  nofollow_job_name="insight-seed-${STEP}-$(date -u +%Y%m%d%H%M%S)"
  export SEED_STEP="$STEP"
  export SEED_JOB_NAME="$nofollow_job_name"
  render_seed_manifest | kube apply -f -
  echo "==> applied Job $SEED_JOB_NAME (step: $STEP)"
  echo "    follow it with: $(kubectl_hint) -n $NAMESPACE logs -f job/$SEED_JOB_NAME"
  case "$STEP" in
    all|silver)
      echo "    NOTE: gold is UNRESOLVED until the persons-seed CronJob"
      echo "          runs and '--step gold' rebuilds over it. Without --no-follow"
      echo "          this script does all three for you." ;;
  esac
  exit 0
fi

# Run the requested step. For the steps that build gold (all, silver), refresh
# the identity projection and rebuild gold, so the stand serves resolved metrics
# instead of a null for every person — the k8s analogue of dev-compose.sh's
# cmd_seed, which does the same for the compose stand.
run_seed_step "$STEP" || exit 1

case "$STEP" in
  all|silver)
    project_identity || exit 1
    echo "==> rebuilding gold over the refreshed identity map"
    run_seed_step gold || exit 1
    ;;
esac

echo "==> seed complete: namespace=$NAMESPACE step=$STEP"
exit 0
