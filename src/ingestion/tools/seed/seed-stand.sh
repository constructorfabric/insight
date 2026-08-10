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
#   ConfigMap <realms config map>         the realm the stand applies, and with
#                                         it the dev-lead persona's address —
#                                         see "the dev-lead address" below for
#                                         why that one has to come from there
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
# --email is deliberately NOT gated here. It used to be, and it could not stay:
# the value is now discovered from the stand, and discovery needs --release
# defaulted and the cluster reachable, neither of which is true this early. An
# address that can be neither discovered nor supplied is reported by the
# missing-values block further down, in the same named-flag list as an
# undiscoverable tenant — one report, one shape, one place to look.
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

# Declared out here because two blocks read it and only one of them always runs:
# a caller passing both --image and --pull-secret skips the fetch below entirely,
# and the dev-lead discovery that follows would then reference an unset variable
# — which under `set -u` is a fatal error rather than a missing value.
release_values=""
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

# ── The dev-lead address ────────────────────────────────────────────────────
# A stand's Keycloak realm and the `identity.persons` rows this Job writes are
# two projections of ONE roster. They have to describe the same people: the
# authenticator resolves a login by its email claim, so an address present in
# the realm and absent from `persons` authenticates and then resolves to nobody.
#
# Every persona's address except one is derived deterministically from the
# roster module — `email_<team>_<NN>@company.nonpresent` — and passes through no
# input at all, so the two projections cannot disagree about them. The dev
# lead's is the exception: it is the roster's one operator-supplied slot, and
# therefore the only address that can be typed differently on the two sides.
#
# It has been. When the realm is generated at deploy time with one address and
# the seed is invoked with another, the result is the quietest failure this
# stand can produce: three of four personas sign in, the dev-lead alone cannot,
# every pod stays Ready and the release reports `deployed`. Nothing below this
# script catches it either — the seeder's own preflight only asserts that
# DEV_USER_EMAIL is SET, never that the IdP has a user by that name — so the
# first thing that notices is a login check, and only if one is run at all.
#
# So the address is discovered, exactly as the tenant, the datastore
# coordinates, the seed image and the IdP source type already are, and for the
# same stated reason: a value supplied from outside is a value that can be wrong
# while looking right. This was the last operator input still doing what the
# tenant used to do.
#
# The realm is the side that gets to be right, because it is the side that
# already exists when this script runs and the side a human logs in against.
#
# THE KEY. `insight-seed-realm` writes each realm user's `id` as the roster
# person's UUID (see keycloak_realm.py `_user()`), so the realm user whose id is
# the roster's DEV_LEAD_UUID IS the dev-lead, whatever address it happens to
# carry. That makes the lookup total and tie-break-free — no name matching, no
# guessing from group membership, no assuming a position in the array.
#
# A stand may also rewrite that `id`: keycloak-config-cli creates users through
# the admin REST API, which assigns ids of its own, so a deployment can copy the
# roster UUID into an `idp_sub` attribute instead and point the authenticator's
# externalIdClaim at it. Both are accepted below. The ConfigMap holds the
# DOCUMENT rather than what Keycloak stored, so `id` is normally still there;
# the second clause costs one `or` and covers the stand where it is not.
#
# This runs on --dry-run as well, which is deliberate: a rehearsal that did not
# resolve the address would not be rehearsing the thing most likely to be wrong.
# It stays a read — --dry-run still writes nothing — but it does now need the
# cluster to answer, so a --dry-run against a half-brought-up stand can report a
# missing address where it used to print a manifest.
DEV_EMAIL_ORIGIN="--email"
realm_dev_email=""
realms_cm=""

# The UUID is read out of the roster module rather than copied into this file.
# A copy would be a second projection of one constant maintained in two places,
# which is structurally the identical bug being fixed one layer down — and a
# stale copy would fail OPEN: it would match no realm user, discovery would go
# quiet, and the script would fall back to demanding --email with nothing said
# about why. Reading it means a renamed or restructured constant yields EMPTY
# and is reported by name. The anchor is deliberately column-0 `DEV_LEAD_UUID`
# so it cannot match `profiles.DEV_LEAD_UUID` references elsewhere, and it
# demands a 36-character UUID-shaped literal so a changed shape is a miss rather
# than a wrong value.
#
# Not read from the seed image: this value is an INPUT to rendering the Job, so
# taking it from the image would mean pulling and running a pod before the
# script has decided it can seed at all — and would make --dry-run, which today
# touches nothing, need a running container to print a manifest.
#
# Required only on this path. If --email was supplied and the module is
# unreadable, the run proceeds exactly as it did before this block existed.
roster_module="$SCRIPT_DIR/insight_seed/profiles.py"
dev_lead_uuid=""
if [[ -r "$roster_module" ]]; then
  dev_lead_uuid="$(sed -n \
    's/^DEV_LEAD_UUID[[:space:]]*=[[:space:]]*"\([0-9a-fA-F-]\{36\}\)".*/\1/p' \
    "$roster_module" 2>/dev/null | head -n1 || true)"
fi

# The release values carry the realm ConfigMap's name, so top them up if the
# image/pull-secret block above never fetched them.
#
# `command -v` rather than `need`: `need` DIES, and making helm and jq hard
# requirements of every run would break a caller who passes --image and
# --pull-secret and has neither installed. Tooling that is absent means
# "discovered nothing", which is this file's existing convention for an empty
# string — not a new failure.
if [[ -z "$release_values" ]] \
   && command -v helm >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  release_values="$(helm_release get values "$RELEASE" -n "$NAMESPACE" -a -o json 2>/dev/null || true)"
fi

# NOT "${RELEASE}-keycloak-config-realms". The chart names this volume
# `default (printf "%s-keycloak-config-realms" (include "insight.fullname" .))
# .Values.keycloakConfig.realmsConfigMap` — an explicit override first, then the
# fullname helper, which honours fullnameOverride and truncates to 63. That is a
# DIFFERENT rule from the platform ConfigMap above, which the chart names from a
# bare .Release.Name; copying `platform_cm`'s pattern here would read the wrong
# object on any stand that sets either value.
if [[ -n "$release_values" ]] && command -v jq >/dev/null 2>&1; then
  # Gated on keycloakConfig.enabled: with the hook off the chart renders no
  # config Job, so nothing applies a realm at all. The ConfigMap is created
  # outside the Helm release and carries no owner reference, so Helm never
  # garbage-collects it — a disabled stand can still be carrying one from a
  # previous bring-up, and letting a dead object have an opinion about who the
  # dev-lead is would be worse than having none.
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
  # Told apart from an empty result, exactly as the identity-resolution Secret
  # is: a ConfigMap that is absent or unreadable is a bring-up or access
  # problem, and reporting it only as "pass --email" would send the operator
  # after the wrong thing.
  if ! kube -n "$NAMESPACE" get configmap "$realms_cm" -o name >/dev/null 2>&1; then
    echo "WARNING: ConfigMap $realms_cm is absent or not readable in namespace $NAMESPACE;" >&2
    echo "         the dev-lead address cannot be read from the realm this stand applies." >&2
    echo "         Deploy the stand's realm first, or pass --email to name the persona." >&2
  else
    # The whole document is fetched because the key name is not a contract —
    # but ONLY the matched user's address ever leaves this expansion. The realm
    # document also carries the realm's client secret and the shared persona
    # password, so `realm_json` is never echoed, in the same spirit as the
    # database_url handling above that refuses to let an authority string out.
    realm_json="$(kube -n "$NAMESPACE" get configmap "$realms_cm" -o json 2>/dev/null || true)"
    # Every clause here is load-bearing:
    #   (.data // {})[]  — iterate EVERY key. The packer decides the filename
    #                      (the gitops target packs a whole directory) and the
    #                      chart's filesLocations glob decides which are applied,
    #                      so `realm-insight.json` is a per-stand fact, not a
    #                      contract worth hardcoding.
    #   fromjson?        — silently drop a key that is not JSON. A federated
    #                      stand legitimately packs a YAML broker realm under
    #                      this same ConfigMap name; that is a stand shape, not
    #                      an error, and it must fall through to "pass --email".
    #   .users? // empty
    #   .[]?             — a realm with no users, or a key whose JSON is a
    #                      scalar, yields nothing rather than a jq error.
    #   | strings        — never call ascii_downcase on a non-string.
    #   .email? // .username?
    #                    — the generator writes both and they are equal; a
    #                      document carrying only the username still resolves.
    #   ascii_downcase   — the seeder lowercases DEV_USER_EMAIL before it writes
    #                      a single row, while the realm generator does not
    #                      lowercase what it is given. Normalising here is what
    #                      stops a mixed-case realm from "disagreeing" with an
    #                      address the seeder would have folded to the same
    #                      string anyway.
    #   unique | .[]     — two keys naming the same person are ONE answer; two
    #                      keys naming different people are two, which is the
    #                      ambiguity refused below rather than resolved by
    #                      whichever key jq happened to visit first.
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
      # Named, not resolved. jq's key order is stable but arbitrary, so picking
      # the first would hand the stand a dev-lead nobody chose — and the whole
      # point of this block is that nobody should be choosing.
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
      # Shape-asserted before it is accepted, exactly as the identity database
      # name is: this value is substituted into `value: "${SEED_DEV_USER_EMAIL}"`
      # in the Job template, so a newline or a double quote reaching it would
      # not produce a bad address — it would produce a broken manifest applied
      # to a live cluster. Held in a variable because the pattern contains a
      # quote, which is unreliable to write inline in a [[ =~ ]] test.
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
  # Case-folded on both sides. bash 3.2 — still /bin/bash on macOS, which this
  # file already accommodates elsewhere — has no ${var,,}, hence `tr`.
  supplied_folded="$(printf '%s' "$DEV_EMAIL" | tr '[:upper:]' '[:lower:]')"
  if [[ "$supplied_folded" != "$realm_dev_email" ]]; then
    # WHY A WARNING AND NOT A REFUSAL, since a refusal is defensible and was
    # weighed. Every other discovered value in this script lets the flag win
    # silently, and can afford to: a wrong --tenant or --identity-db fails
    # loudly inside the Job within seconds. This one cannot — a wrong address
    # produces a Job that SUCCEEDS and a stand that looks seeded — so there is a
    # real argument for making a disagreement fatal here.
    #
    # It is not made fatal today for one reason: refusing would break every
    # caller that currently passes --email, at the moment this lands and before
    # those callers have been changed. `--email` was REQUIRED until this commit,
    # so every existing invocation supplies one, and a stand whose realm was
    # provisioned some other way is a caller with a legitimate answer that
    # disagrees with nothing. Refusing belongs in the change that also stops the
    # callers passing the flag — not in the one that merely makes the flag
    # optional. Until then this says the whole thing out loud, on stderr, naming
    # both addresses and which one is being used, and the seed still runs.
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
# Named the same way as the rest, so an address that could be neither read nor
# supplied lands in one list with the tenant and the image rather than in an
# early exit of its own. The description carries the two things an operator has
# to know to act: which object was consulted, and that the answer lives in the
# realm rather than in whatever they were about to type.
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
# The origin is printed, not just the value: an address alone cannot be checked
# by eye, but "which side said so" can. This is the line that shows, before
# anything is written, whether the seed is following the realm or overriding it.
echo "==> idp:       source_type=$IDP_SOURCE_TYPE dev_user=$DEV_EMAIL (from $DEV_EMAIL_ORIGIN)"

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
