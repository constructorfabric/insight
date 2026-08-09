#!/usr/bin/env bash
#
# provision-ci-deployer.sh — mint, rotate, verify and revoke the Kubernetes
# credential that CI uses to deploy the Insight umbrella chart onto the test
# stand (constructorfabric/insight#2244, decision D5).
#
# ═══════════════════════════════════════════════════════════════════════════
#  WHAT IT CREATES
# ═══════════════════════════════════════════════════════════════════════════
#   ServiceAccount   <ns>/ci-deployer
#   RoleBinding      <ns>/ci-deployer-admin              → ClusterRole/admin
#   Role             <ns>/ci-deployer-crd-supplement
#   RoleBinding      <ns>/ci-deployer-crd-supplement     → the Role above
#   Secret           <ns>/ci-deployer-token              (service-account-token)
#   <--out>          a kubeconfig carrying server + CA + that token, mode 0600
#
# Every object is namespaced. Nothing cluster-scoped is created and nothing
# cluster-scoped is granted. The verification section at the end of a run
# PROVES that with `kubectl auth can-i` instead of asserting it in prose —
# a reviewer should be able to read the assertions rather than trust the
# author's reading of the RBAC bootstrap policy.
#
# ═══════════════════════════════════════════════════════════════════════════
#  WHY A SERVICEACCOUNT TOKEN AND NOT A COPY OF SOMEONE'S ADMIN KUBECONFIG
# ═══════════════════════════════════════════════════════════════════════════
# 1. REVOCABILITY — the decisive reason.
#    A ServiceAccount token can be destroyed. `--revoke` deletes the token
#    Secret and the credential stops authenticating on the next request, no
#    control-plane restart and no coordination with anyone. Deleting the
#    ServiceAccount itself is even more final: the SA's UID is embedded in
#    every token it ever issued and is re-checked on every request, so a new
#    ServiceAccount of the same name does not resurrect an old token. (Both
#    checks depend on the API server running with --service-account-lookup,
#    which has been the default since 1.7 and which `--verify-only` will show
#    you the truth about after a revoke: the credential either authenticates
#    or it does not.)
#
#    An admin kubeconfig is, in almost every distribution, a CLIENT
#    CERTIFICATE. Kubernetes has no certificate revocation at all: the API
#    server consults no CRL and no OCSP responder. Once such a kubeconfig
#    leaks, the ONLY way to invalidate it is to rotate the cluster CA — which
#    invalidates every other client certificate on the cluster at the same
#    time and requires restarting the control plane. "Rotate the CI
#    credential" must not be a cluster outage.
#
# 2. BLAST RADIUS. An admin kubeconfig authenticates as a cluster-admin
#    subject: every namespace, every CRD, every node. This credential is
#    bound with a RoleBinding, so even though the roleRef names a
#    ClusterRole, the grant stops dead at the namespace edge.
#
# 3. ATTRIBUTION. Audit entries read `system:serviceaccount:<ns>:ci-deployer`
#    — a subject that exists for exactly one purpose. A shared human
#    kubeconfig makes every automated action look like a person, and makes
#    "who deleted the release" unanswerable.
#
# ─── Alternatives considered and rejected ─────────────────────────────────
#   * `kubectl create token ci-deployer --duration=…` (the TokenRequest API,
#     and the modern recommendation). Rejected HERE only because a GitHub
#     Actions environment secret is a static string: a projected/bound token
#     expires (one hour by default, and the API server caps the ceiling with
#     --service-account-max-token-expiration), and nothing in the merge path
#     would refresh it. The genuinely correct long-term answer is federation
#     — GitHub's OIDC id_token exchanged for a short-lived cluster credential
#     — which needs the API server to trust GitHub as an OIDC issuer, or a
#     cloud IAM broker in front of it. That is a follow-up, not a
#     prerequisite for #2244. A long-lived token is the deliberate trade, and
#     it is affordable precisely because it is namespace-scoped and instantly
#     revocable.
#   * ClusterRoleBinding → admin. Rejected: identical permissions in EVERY
#     namespace, which is the whole thing we are trying not to do.
#   * RoleBinding → cluster-admin. Works, and a RoleBinding does scope it to
#     one namespace — but `cluster-admin` carries `escalate` and `bind` and
#     matches every API group that will ever exist on the cluster. `admin` is
#     the intended "owns this namespace" role and deliberately lacks both.
#   * A hand-written Role enumerating every kind the umbrella renders.
#     Rejected after writing it out: the rendered set changes whenever a
#     subchart adds a kind, and a stale bespoke list fails closed during a
#     deploy rather than during review. `admin` plus a small, explicitly
#     justified supplement for custom resources is the maintainable middle.
#
# ═══════════════════════════════════════════════════════════════════════════
#  WHY THE SUPPLEMENTAL ROLE IS NOT OPTIONAL
# ═══════════════════════════════════════════════════════════════════════════
# The built-in `admin` ClusterRole is an AGGREGATE. It picks up rules only
# from ClusterRoles labelled `rbac.authorization.k8s.io/aggregate-to-admin`,
# so a custom resource is covered only if its provider ships such a
# ClusterRole and its chart has that switch enabled. The upstream Gateway API
# CRD bundle ships no aggregation ClusterRoles at all; Argo Workflows and
# cert-manager ship them but behind chart values. Relying on that is how you
# get a credential that works on one cluster and 403s on the next.
#
# There is a second, sharper reason. The umbrella chart itself renders a Role
# (charts/insight/templates/ingestion/reconcile-rbac.yaml, gated on
# `ingestion.templates.enabled`) that grants `argoproj.io` and
# `onepassword.com` verbs. RBAC escalation prevention refuses to let a
# subject CREATE a Role containing permissions the subject does not itself
# hold, unless it holds the `escalate` verb — which `admin` does not. Without
# the supplement, `helm upgrade` fails on that one manifest with
# "attempt to grant extra privileges", after having already applied half the
# release. The supplement is therefore a superset of what the chart's own
# Roles grant, on purpose.
#
# ═══════════════════════════════════════════════════════════════════════════
#  KNOWN LIMITATION — helm's --create-namespace
# ═══════════════════════════════════════════════════════════════════════════
# `helm upgrade --install … --create-namespace` (which deploy/gitops/Makefile
# hardcodes) POSTs a Namespace object at CLUSTER scope when, and only when,
# the release does not yet exist. This credential cannot create namespaces —
# that is the point — so a FIRST install into an empty namespace fails with a
# 403 before any chart resource is applied. Upgrades of an existing release
# never reach that code path, so day-2 CI is unaffected.
#   Disposition: the target namespace is created once, by a human, with a
#   human credential. If CI ever has to bootstrap a namespace from nothing,
#   fix it by pre-creating the namespace in the same human step — NOT by
#   granting namespace-create to CI.
#
# ═══════════════════════════════════════════════════════════════════════════
#  WHAT THIS DELIBERATELY DOES NOT GRANT
# ═══════════════════════════════════════════════════════════════════════════
#   * Anything cluster-scoped: namespaces, nodes, CRDs, ClusterRoles,
#     ClusterRoleBindings, PersistentVolumes, StorageClasses.
#   * Any other namespace. In particular the datastore namespaces, whose
#     Secrets are the stand's crown jewels, stay out of reach.
#   * The chart's `airbyte-auth-rbac.yaml` renders a Role into
#     `airbyte.namespace` when that value is non-empty. A namespace-scoped
#     credential cannot create it. Keep `airbyte.namespace: ""` (same
#     namespace as the release) in the CI-driven environment's values, or
#     provision a second, equally narrow supplement there by hand.
#
# ═══════════════════════════════════════════════════════════════════════════
#  SAFETY PROPERTIES OF THE SCRIPT ITSELF
# ═══════════════════════════════════════════════════════════════════════════
#   * DRY-RUN BY DEFAULT. Nothing is created, deleted or written without an
#     explicit `--apply`. The default run prints the exact manifests and the
#     exact verification commands and exits 0.
#   * EXPLICIT TARGET. `--kubeconfig` and `--expect-cluster` are both
#     required, and the script refuses loudly when the kubeconfig's context
#     resolves to a different cluster. There is no "current context" default
#     and no way to omit the expectation: a deploy credential minted on the
#     wrong cluster is a security incident, not a typo.
#   * NO SECRET EVER REACHES STDOUT. The token is read into a shell variable
#     (never an argv, which `ps` exposes to every local user), written into
#     the output kubeconfig under `umask 077`, and unset. The assembled
#     kubeconfig is never cat'ed, diffed or echoed by this script.
#   * NO INFRA HOSTNAME REACHES STDOUT. The API server URL is printed with
#     its host redacted unless `--show-server` is passed. Operator terminals
#     get pasted into pull requests, and this repository is public.
#
# ═══════════════════════════════════════════════════════════════════════════
#  USAGE
# ═══════════════════════════════════════════════════════════════════════════
#   Plan (default — reads the cluster, writes nothing):
#     ./provision-ci-deployer.sh --kubeconfig ~/.kube/stand.yaml \
#         --expect-cluster <cluster-name>
#
#   Apply, then assemble the CI kubeconfig:
#     ./provision-ci-deployer.sh --kubeconfig ~/.kube/stand.yaml \
#         --expect-cluster <cluster-name> --apply
#
#   Re-check the scope of a kubeconfig you already have:
#     ./provision-ci-deployer.sh --kubeconfig ~/.kube/stand.yaml \
#         --expect-cluster <cluster-name> --verify-only
#
#   Rotate / revoke: add --rotate or --revoke (still needs --apply).
#
# The operator runbook — GitHub environment creation, secret names, rotation
# cadence, cleanup — lives at
# docs/components/deployment/specs/sop/credentials-runbook.md.

set -euo pipefail

# ─── Presentation ─────────────────────────────────────────────────────────
# Same tput-with-fallback shape as scripts/doctor.sh so the two read alike
# when they scroll past each other in one terminal.
C_RED=$(tput setaf 1 2>/dev/null || echo "")
C_GRN=$(tput setaf 2 2>/dev/null || echo "")
C_YEL=$(tput setaf 3 2>/dev/null || echo "")
C_CYA=$(tput setaf 6 2>/dev/null || echo "")
C_RST=$(tput sgr0 2>/dev/null || echo "")

note() { printf '%s\n' "$*"; }
ok() { printf '%sOK%s      %s\n' "$C_GRN" "$C_RST" "$*"; }
warn() { printf '%sNOTE%s    %s\n' "$C_YEL" "$C_RST" "$*"; }
bad() { printf '%sFAIL%s    %s\n' "$C_RED" "$C_RST" "$*"; }
hdr() { printf '\n%s══ %s ══%s\n' "$C_CYA" "$*" "$C_RST"; }
die() {
  printf '%sERROR%s: %s\n' "$C_RED" "$C_RST" "$*" >&2
  exit 1
}

# ─── Defaults ─────────────────────────────────────────────────────────────
KUBECONFIG_IN=""
EXPECT_CLUSTER=""
EXPECT_SERVER=""
SRC_CONTEXT=""
NAMESPACE="insight"
SA_NAME="ci-deployer"
TOKEN_SECRET=""
# Written into the GENERATED kubeconfig as the context name. The gitops
# Makefile's `kube-ctx` target refuses to act unless
# `kubectl config current-context` equals inventory `.kubeContext`, and
# deploy/gitops/README.md documents the convention `insight-<env>`. Naming
# the generated context after the environment therefore makes
# `KUBECONFIG=<out> make deploy ENV=test-stand` work with no KUBE_CTX
# override. Change it with --context-name if the inventory disagrees.
OUT_CONTEXT="insight-test-stand"
OUT_PATH="${HOME}/.kube/insight-test-stand-ci-deployer.kubeconfig"
CA_FILE=""
APPLY=0
MODE="provision" # provision | rotate | revoke | purge | verify
SHOW_SERVER=0
WITH_SUPPLEMENT=1
TOKEN_WAIT_S=60

usage() {
  cat <<'USAGE'
provision-ci-deployer.sh — provision the namespace-scoped CI deploy credential.

Required:
  --kubeconfig PATH        Admin kubeconfig for the target cluster. Read only;
                           this script never modifies it.
  --expect-cluster NAME    The cluster name the kubeconfig's context MUST
                           resolve to. The script refuses if it does not.

Target selection:
  --context NAME           Context inside --kubeconfig (default: its
                           current-context).
  --expect-server URL      Optional second guard: the API server URL must
                           match this string exactly.
  --namespace NAME         Namespace to scope the credential to
                           (default: insight).
  --serviceaccount NAME    ServiceAccount name (default: ci-deployer).
  --token-secret NAME      Token Secret name (default: <sa>-token).

Output:
  --out PATH               Where to write the assembled kubeconfig
                           (default: ~/.kube/insight-test-stand-ci-deployer.kubeconfig).
                           Its parent directory must already exist. Refused if
                           the path is inside a git work tree and not ignored.
  --context-name NAME      Context/cluster name INSIDE the generated
                           kubeconfig (default: insight-test-stand).
  --ca-file PATH           PEM CA bundle to embed, if neither the token Secret
                           nor the admin kubeconfig carries one.

Modes (at most one; default provisions):
  --rotate                 Replace ONLY the token Secret and rewrite --out.
  --revoke                 Delete the token Secret. The credential stops
                           authenticating; the ServiceAccount and RBAC stay.
  --purge                  Delete token Secret, both RoleBindings, the Role
                           and the ServiceAccount.
  --verify-only            Run the scope assertions against an existing --out.

Behaviour:
  --apply                  Actually mutate. WITHOUT THIS THE SCRIPT ONLY
                           PRINTS THE PLAN.
  --no-supplement          Skip the Gateway API / Argo / cert-manager
                           supplemental Role. Only for a cluster whose CRD
                           providers ship aggregate-to-admin ClusterRoles.
  --show-server            Print the API server URL unredacted.
  --token-wait SECONDS     How long to wait for the token controller to fill
                           the Secret (default: 60).
  -h, --help               This text.
USAGE
}

# ─── Argument parsing ─────────────────────────────────────────────────────
set_mode() {
  [ "$MODE" = "provision" ] || die "modes are mutually exclusive: already in '$MODE', cannot also do '$1'"
  MODE="$1"
}

while [ $# -gt 0 ]; do
  case "$1" in
  --kubeconfig)
    KUBECONFIG_IN="${2:?--kubeconfig needs a value}"
    shift 2
    ;;
  --expect-cluster)
    EXPECT_CLUSTER="${2:?--expect-cluster needs a value}"
    shift 2
    ;;
  --expect-server)
    EXPECT_SERVER="${2:?--expect-server needs a value}"
    shift 2
    ;;
  --context)
    SRC_CONTEXT="${2:?--context needs a value}"
    shift 2
    ;;
  --namespace)
    NAMESPACE="${2:?--namespace needs a value}"
    shift 2
    ;;
  --serviceaccount)
    SA_NAME="${2:?--serviceaccount needs a value}"
    shift 2
    ;;
  --token-secret)
    TOKEN_SECRET="${2:?--token-secret needs a value}"
    shift 2
    ;;
  --out)
    OUT_PATH="${2:?--out needs a value}"
    shift 2
    ;;
  --context-name)
    OUT_CONTEXT="${2:?--context-name needs a value}"
    shift 2
    ;;
  --ca-file)
    CA_FILE="${2:?--ca-file needs a value}"
    shift 2
    ;;
  --token-wait)
    TOKEN_WAIT_S="${2:?--token-wait needs a value}"
    shift 2
    ;;
  --rotate)
    set_mode rotate
    shift
    ;;
  --revoke)
    set_mode revoke
    shift
    ;;
  --purge)
    set_mode purge
    shift
    ;;
  --verify-only)
    set_mode verify
    shift
    ;;
  --apply)
    APPLY=1
    shift
    ;;
  --no-supplement)
    WITH_SUPPLEMENT=0
    shift
    ;;
  --show-server)
    SHOW_SERVER=1
    shift
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  *) die "unknown argument: $1 (try --help)" ;;
  esac
done

[ -n "$KUBECONFIG_IN" ] || {
  usage >&2
  die "--kubeconfig is required. There is no current-context default on purpose."
}
[ -n "$EXPECT_CLUSTER" ] || {
  usage >&2
  die "--expect-cluster is required. Minting a deploy credential on the wrong cluster is not a recoverable typo."
}
[ -r "$KUBECONFIG_IN" ] || die "cannot read kubeconfig at $KUBECONFIG_IN"
[ -n "$TOKEN_SECRET" ] || TOKEN_SECRET="${SA_NAME}-token"
case "$TOKEN_WAIT_S" in
'' | *[!0-9]*) die "--token-wait must be an integer number of seconds" ;;
esac

command -v kubectl >/dev/null 2>&1 || die "kubectl is required (brew install kubectl)"

ROLE_NAME="${SA_NAME}-crd-supplement"
RB_ADMIN="${SA_NAME}-admin"
RB_SUPPLEMENT="${SA_NAME}-crd-supplement"

# `kubectl` against the ADMIN kubeconfig. Every call in this script that uses
# it is either read-only or gated behind $APPLY.
kadm() { kubectl --kubeconfig "$KUBECONFIG_IN" --context "$SRC_CONTEXT" "$@"; }

# ─── Output path guard ────────────────────────────────────────────────────
# Checked before anything touches the network, so a bad --out costs a second
# rather than a round trip. The assembled kubeconfig contains a bearer token:
# if it lands inside the work tree of this PUBLIC repository and is not
# ignored, one `git add -A` publishes it. Refuse rather than rely on review
# catching it.
OUT_DIR="$(dirname "$OUT_PATH")"
[ -d "$OUT_DIR" ] || die "the parent directory of --out does not exist: $OUT_DIR (mkdir -p it first)"
OUT_ABS="$(cd "$OUT_DIR" && pwd)/$(basename "$OUT_PATH")"

if git -C "$OUT_DIR" rev-parse --show-toplevel >/dev/null 2>&1; then
  if ! git -C "$OUT_DIR" check-ignore -q "$OUT_ABS" 2>/dev/null; then
    die "refusing to write a bearer token to '$OUT_ABS': it is inside a git work tree and not gitignored. Choose a path outside the repo (e.g. ~/.kube/…)."
  fi
  warn "--out is inside a git work tree but gitignored — allowed, still prefer a path outside the repo"
fi

# ─── Target guard ─────────────────────────────────────────────────────────
# The single most important thing this script does. `kubectl` happily follows
# whatever current-context it finds; a credential provisioned against the
# wrong cluster is a silent, standing grant nobody is looking for.
hdr "target"

if [ -z "$SRC_CONTEXT" ]; then
  SRC_CONTEXT="$(kubectl --kubeconfig "$KUBECONFIG_IN" config current-context 2>/dev/null || true)"
  [ -n "$SRC_CONTEXT" ] || die "no current-context in $KUBECONFIG_IN and --context was not given"
fi

ACTUAL_CLUSTER="$(kubectl --kubeconfig "$KUBECONFIG_IN" config view \
  -o "jsonpath={.contexts[?(@.name==\"${SRC_CONTEXT}\")].context.cluster}" 2>/dev/null || true)"
[ -n "$ACTUAL_CLUSTER" ] || die "context '$SRC_CONTEXT' is not present in $KUBECONFIG_IN"

if [ "$ACTUAL_CLUSTER" != "$EXPECT_CLUSTER" ]; then
  printf '%s\n' "$C_RED" >&2
  printf '  ┌──────────────────────────────────────────────────────────────┐\n' >&2
  printf '  │  REFUSING TO ACT — CLUSTER MISMATCH                          │\n' >&2
  printf '  └──────────────────────────────────────────────────────────────┘%s\n' "$C_RST" >&2
  printf '    kubeconfig : %s\n' "$KUBECONFIG_IN" >&2
  printf '    context    : %s\n' "$SRC_CONTEXT" >&2
  printf '    resolves to: %s\n' "$ACTUAL_CLUSTER" >&2
  printf '    --expect-cluster says: %s\n\n' "$EXPECT_CLUSTER" >&2
  printf '  Nothing was created, deleted or written. Re-run with the right\n' >&2
  printf '  --kubeconfig/--context, or correct --expect-cluster if you are\n' >&2
  printf '  certain which cluster you mean.\n' >&2
  exit 2
fi

SERVER="$(kubectl --kubeconfig "$KUBECONFIG_IN" config view \
  -o "jsonpath={.clusters[?(@.name==\"${ACTUAL_CLUSTER}\")].cluster.server}" 2>/dev/null || true)"
[ -n "$SERVER" ] || die "cluster '$ACTUAL_CLUSTER' has no server URL in $KUBECONFIG_IN"

if [ -n "$EXPECT_SERVER" ] && [ "$SERVER" != "$EXPECT_SERVER" ]; then
  die "API server does not match --expect-server (compared without printing either; pass --show-server to see them)"
fi

# Redact the host by default: this repo is public and operator terminals end
# up in pull requests and issue comments. The scheme and port are enough to
# tell "yes, that's an API server endpoint" apart from "that's the public
# ingress URL".
redact_url() {
  if [ "$SHOW_SERVER" = "1" ]; then
    printf '%s' "$1"
  else
    printf '%s' "$1" | sed -E 's#^([a-zA-Z][a-zA-Z0-9+.-]*://)[^/:]+#\1<redacted-host>#'
  fi
}

ok "cluster '$ACTUAL_CLUSTER' matches --expect-cluster"
note "        context    : $SRC_CONTEXT"
note "        api server : $(redact_url "$SERVER")"
note "        namespace  : $NAMESPACE"

# Liveness + "is this really a cluster I can act on" check. Read-only.
kadm version --request-timeout=15s -o json >/dev/null 2>&1 ||
  die "cannot reach the API server for context '$SRC_CONTEXT' (VPN down? kubeconfig expired?)"
ok "API server reachable"

if kadm get namespace "$NAMESPACE" >/dev/null 2>&1; then
  ok "namespace '$NAMESPACE' exists"
else
  warn "namespace '$NAMESPACE' does not exist yet — create it with a human credential first"
  warn "        (see the 'helm --create-namespace' limitation in this script's header)"
fi

# ═══════════════════════════════════════════════════════════════════════════
#  MANIFESTS
# ═══════════════════════════════════════════════════════════════════════════
# Built as one document so the plan the operator reads and the bytes that get
# applied are literally the same string.
render_manifests() {
  cat <<EOF
---
# The CI identity. Never mounted into a pod — it exists purely as an
# authentication subject for GitHub Actions — so automount is off.
apiVersion: v1
kind: ServiceAccount
metadata:
  name: ${SA_NAME}
  namespace: ${NAMESPACE}
  labels:
    app.kubernetes.io/name: ${SA_NAME}
    app.kubernetes.io/component: ci-credential
    app.kubernetes.io/managed-by: provision-ci-deployer.sh
  annotations:
    insight.constructor.tech/purpose: >-
      GitHub Actions deploy credential for the Insight test stand.
      Provisioned by deploy/gitops/scripts/provision-ci-deployer.sh.
      Rotate or revoke with that script; see
      docs/components/deployment/specs/sop/credentials-runbook.md.
automountServiceAccountToken: false
---
# RoleBinding — NOT ClusterRoleBinding. roleRef names the built-in cluster
# role \`admin\`, but a RoleBinding applies its rules only inside this
# namespace. That is the whole containment story in four lines of YAML.
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: ${RB_ADMIN}
  namespace: ${NAMESPACE}
  labels:
    app.kubernetes.io/name: ${SA_NAME}
    app.kubernetes.io/component: ci-credential
    app.kubernetes.io/managed-by: provision-ci-deployer.sh
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: admin
subjects:
  - kind: ServiceAccount
    name: ${SA_NAME}
    namespace: ${NAMESPACE}
EOF

  if [ "$WITH_SUPPLEMENT" = "1" ]; then
    cat <<EOF
---
# Supplement for custom resources the umbrella chart touches. \`admin\` is an
# aggregate and only covers CRDs whose provider ships a ClusterRole labelled
# rbac.authorization.k8s.io/aggregate-to-admin. Listing them here removes a
# whole class of "worked on the last cluster, 403 on this one" failures, and
# — see the script header — is what lets the deploy create the chart's own
# reconcile Role without tripping RBAC escalation prevention.
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: ${ROLE_NAME}
  namespace: ${NAMESPACE}
  labels:
    app.kubernetes.io/name: ${SA_NAME}
    app.kubernetes.io/component: ci-credential
    app.kubernetes.io/managed-by: provision-ci-deployer.sh
rules:
  # Gateway API. The umbrella chart renders NO HTTPRoute — the stand's edge
  # routes are applied alongside the release, and CI has to be able to apply
  # and re-read them (and to read \`.status\` for the Accepted/ResolvedRefs
  # check before the smoke stage). \`referencegrants\` is the companion object
  # the moment any HTTPRoute backendRef crosses a namespace; granting it in a
  # namespaced Role costs nothing and avoids a second provisioning round when
  # that day comes. \`gateways\` is read-only here: it is namespaced, but the
  # Gateway itself normally lives in the gateway controller's namespace,
  # which this credential cannot see at all — see the runbook.
  - apiGroups: ["gateway.networking.k8s.io"]
    resources: [httproutes, referencegrants]
    verbs: [get, list, watch, create, update, patch, delete]
  - apiGroups: ["gateway.networking.k8s.io"]
    resources: [gateways]
    verbs: [get, list, watch]
  # Argo Workflows. charts/insight/templates/ingestion/ renders
  # WorkflowTemplate and CronWorkflow objects, and reconcile-rbac.yaml
  # renders a Role granting workflows / cronworkflows / workflows/status /
  # workflowtaskresults. This block is a superset of that Role on purpose:
  # a subject cannot create a Role granting more than it holds.
  - apiGroups: ["argoproj.io"]
    resources: [workflows, workflowtemplates, cronworkflows, workfloweventbindings]
    verbs: [get, list, watch, create, update, patch, delete]
  - apiGroups: ["argoproj.io"]
    resources: [workflows/status, workflowtaskresults]
    verbs: [get, list, watch, create, update, patch, delete]
  # 1Password operator CRs, for the same escalation-prevention reason: the
  # chart's reconcile Role grants read on them.
  - apiGroups: ["onepassword.com"]
    resources: [onepassworditems]
    verbs: [get, list, watch]
  # cert-manager. A subchart renders a Certificate. cert-manager DOES ship
  # aggregate-to-admin ClusterRoles, but only when its chart's rbac switch is
  # on — which is a property of how someone else installed it.
  - apiGroups: ["cert-manager.io"]
    resources: [certificates, certificaterequests, issuers]
    verbs: [get, list, watch, create, update, patch, delete]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: ${RB_SUPPLEMENT}
  namespace: ${NAMESPACE}
  labels:
    app.kubernetes.io/name: ${SA_NAME}
    app.kubernetes.io/component: ci-credential
    app.kubernetes.io/managed-by: provision-ci-deployer.sh
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: ${ROLE_NAME}
subjects:
  - kind: ServiceAccount
    name: ${SA_NAME}
    namespace: ${NAMESPACE}
EOF
  fi

  cat <<EOF
---
# Long-lived ServiceAccount token. The token controller fills .data.token and
# .data['ca.crt'] shortly after this Secret appears; this manifest carries no
# data of its own, which is why it is safe to print.
#
# This is the legacy (non-expiring) token mechanism, chosen deliberately: a
# GitHub Actions environment secret is a static string with no refresh path.
# See the header for why TokenRequest was rejected and what replaces this
# later. Kubernetes' LegacyServiceAccountTokenCleanUp only reaps tokens that
# have gone a year without being used; a token used on every merge is never a
# candidate.
apiVersion: v1
kind: Secret
metadata:
  name: ${TOKEN_SECRET}
  namespace: ${NAMESPACE}
  labels:
    app.kubernetes.io/name: ${SA_NAME}
    app.kubernetes.io/component: ci-credential
    app.kubernetes.io/managed-by: provision-ci-deployer.sh
  annotations:
    kubernetes.io/service-account.name: ${SA_NAME}
type: kubernetes.io/service-account-token
EOF
}

# ═══════════════════════════════════════════════════════════════════════════
#  VERIFICATION
# ═══════════════════════════════════════════════════════════════════════════
# Assertions, not a report. Every mismatch is a failure and the script exits
# non-zero, because "the credential is scoped" is a claim that has to survive
# the next person editing the Role.
#
# One subtlety worth knowing before reading the table: RBAC evaluates a
# request for a CLUSTER-SCOPED resource differently depending on whether the
# request carries a namespace. `kubectl get namespace insight` is a
# namespaced request (the API server treats the name as the namespace) and a
# RoleBinding in that namespace can satisfy it. `kubectl get namespaces` —
# the list — is a cluster-scoped request that only ClusterRoleBindings can
# satisfy. All the "must be no" assertions below therefore pass
# --all-namespaces, which sends an empty namespace, so they test what they
# claim to test.
SCOPE_FAILURES=0

expect_can() {
  local want="$1" desc="$2"
  shift 2
  local got
  got="$(kubectl --kubeconfig "$OUT_PATH" auth can-i "$@" 2>/dev/null || true)"
  got="${got%%$'\n'*}"
  got="${got%% *}"
  [ -n "$got" ] || got="(no answer)"
  if [ "$got" = "$want" ]; then
    ok "$(printf '%-4s %s' "$got" "$desc")"
  else
    bad "$(printf 'want %-3s got %-3s %s' "$want" "$got" "$desc")"
    SCOPE_FAILURES=$((SCOPE_FAILURES + 1))
  fi
}

print_verification_plan() {
  cat <<PLAN
  # must answer yes — the deploy/seed stages genuinely need these
  kubectl auth can-i create jobs                                  -n ${NAMESPACE}
  kubectl auth can-i get    pods/log                              -n ${NAMESPACE}
  kubectl auth can-i create secrets                               -n ${NAMESPACE}
  kubectl auth can-i update deployments.apps                      -n ${NAMESPACE}
  kubectl auth can-i create roles.rbac.authorization.k8s.io       -n ${NAMESPACE}
PLAN
  if [ "$WITH_SUPPLEMENT" = "1" ]; then
    cat <<PLAN
  kubectl auth can-i create workflowtemplates.argoproj.io         -n ${NAMESPACE}
  kubectl auth can-i update httproutes.gateway.networking.k8s.io  -n ${NAMESPACE}
PLAN
  fi
  cat <<PLAN

  # must answer no — this is the containment claim
  kubectl auth can-i '*' '*'                                      --all-namespaces
  kubectl auth can-i list   namespaces                            --all-namespaces
  kubectl auth can-i create namespaces                            --all-namespaces
  kubectl auth can-i get    secrets                               --all-namespaces
  kubectl auth can-i list   nodes                                 --all-namespaces
  kubectl auth can-i create clusterrolebindings.rbac.authorization.k8s.io --all-namespaces
  kubectl auth can-i get    pods                                  -n kube-system
PLAN
}

run_verification() {
  hdr "verification — scope of the assembled credential"
  [ -r "$OUT_PATH" ] || die "no kubeconfig at $OUT_PATH — run without --verify-only first"

  local who
  who="$(kubectl --kubeconfig "$OUT_PATH" auth whoami -o jsonpath='{.status.userInfo.username}' 2>/dev/null || true)"
  if [ -n "$who" ]; then
    if [ "$who" = "system:serviceaccount:${NAMESPACE}:${SA_NAME}" ]; then
      ok "authenticates as $who"
    else
      bad "authenticates as '$who', expected system:serviceaccount:${NAMESPACE}:${SA_NAME}"
      SCOPE_FAILURES=$((SCOPE_FAILURES + 1))
    fi
  else
    warn "\`kubectl auth whoami\` unavailable (needs kubectl 1.27+ / apiserver 1.28+) — identity not asserted"
  fi

  expect_can yes "create jobs in ${NAMESPACE} (the seed stage renders a Job)" create jobs -n "$NAMESPACE"
  expect_can yes "get pods/log in ${NAMESPACE} (seed + curated diagnostics)" get pods/log -n "$NAMESPACE"
  expect_can yes "create secrets in ${NAMESPACE} (helm release storage)" create secrets -n "$NAMESPACE"
  expect_can yes "update deployments in ${NAMESPACE} (helm upgrade, rollout restart)" update deployments.apps -n "$NAMESPACE"
  expect_can yes "create Roles in ${NAMESPACE} (the chart renders its own RBAC)" create roles.rbac.authorization.k8s.io -n "$NAMESPACE"

  if [ "$WITH_SUPPLEMENT" = "1" ]; then
    expect_can yes "create WorkflowTemplates in ${NAMESPACE}" create workflowtemplates.argoproj.io -n "$NAMESPACE"
    expect_can yes "update HTTPRoutes in ${NAMESPACE}" update httproutes.gateway.networking.k8s.io -n "$NAMESPACE"
  fi

  expect_can no "anything, anywhere (the blanket check)" '*' '*' --all-namespaces
  expect_can no "list namespaces at cluster scope" list namespaces --all-namespaces
  expect_can no "create namespaces (hence the --create-namespace limitation)" create namespaces --all-namespaces
  expect_can no "read Secrets across all namespaces" get secrets --all-namespaces
  expect_can no "list nodes" list nodes --all-namespaces
  expect_can no "create ClusterRoleBindings" create clusterrolebindings.rbac.authorization.k8s.io --all-namespaces
  expect_can no "read pods in kube-system" get pods -n kube-system

  if [ "$SCOPE_FAILURES" -gt 0 ]; then
    die "$SCOPE_FAILURES scope assertion(s) failed — do NOT put this kubeconfig in a GitHub environment"
  fi
  ok "all scope assertions hold"
}

# ═══════════════════════════════════════════════════════════════════════════
#  KUBECONFIG ASSEMBLY
# ═══════════════════════════════════════════════════════════════════════════
# Written by hand rather than with `kubectl config set-credentials --token=…`
# on purpose: anything passed as an argv is visible in `ps` to every local
# user for the lifetime of the call. The token only ever exists in a shell
# variable and in the 0600 file.
assemble_kubeconfig() {
  local token ca_b64

  # The go-template form decodes in-process; `base64 -d` vs `-D` differs
  # between GNU and BSD and is not worth the portability shim.
  token="$(kadm -n "$NAMESPACE" get secret "$TOKEN_SECRET" \
    -o go-template='{{ if .data.token }}{{ .data.token | base64decode }}{{ end }}' 2>/dev/null || true)"
  [ -n "$token" ] || die "the token Secret '$TOKEN_SECRET' carries no token"

  # certificate-authority-data in a kubeconfig is base64(PEM), and the token
  # Secret's ca.crt field is already base64(PEM) — copy it across verbatim.
  ca_b64="$(kadm -n "$NAMESPACE" get secret "$TOKEN_SECRET" \
    -o "jsonpath={.data.ca\.crt}" 2>/dev/null || true)"

  if [ -z "$ca_b64" ] && [ -n "$CA_FILE" ]; then
    [ -r "$CA_FILE" ] || die "cannot read --ca-file at $CA_FILE"
    ca_b64="$(base64 <"$CA_FILE" | tr -d '\n')"
  fi
  if [ -z "$ca_b64" ]; then
    ca_b64="$(kubectl --kubeconfig "$KUBECONFIG_IN" config view --raw \
      -o "jsonpath={.clusters[?(@.name==\"${ACTUAL_CLUSTER}\")].cluster.certificate-authority-data}" 2>/dev/null || true)"
  fi
  if [ -z "$ca_b64" ]; then
    local ca_path
    ca_path="$(kubectl --kubeconfig "$KUBECONFIG_IN" config view --raw \
      -o "jsonpath={.clusters[?(@.name==\"${ACTUAL_CLUSTER}\")].cluster.certificate-authority}" 2>/dev/null || true)"
    if [ -n "$ca_path" ] && [ -r "$ca_path" ]; then
      ca_b64="$(base64 <"$ca_path" | tr -d '\n')"
    fi
  fi
  [ -n "$ca_b64" ] || die "could not resolve a cluster CA — pass --ca-file <pem>. (Refusing to emit insecure-skip-tls-verify: a CI credential that does not pin its server can be handed to anything that answers on that address.)"

  # umask BEFORE creating the file: a chmod after the write leaves a window
  # in which the token is world-readable.
  local old_umask
  old_umask="$(umask)"
  umask 077
  # The cluster entry is named after the generated CONTEXT, not after the
  # real cluster, so nothing about the target cluster's naming leaks into a
  # file that might get copied around.
  cat >"$OUT_PATH" <<EOF
# Insight test-stand CI deploy credential.
#
# Subject : system:serviceaccount:${NAMESPACE}:${SA_NAME}
# Scope   : namespace '${NAMESPACE}' only (RoleBinding, not ClusterRoleBinding)
# Source  : deploy/gitops/scripts/provision-ci-deployer.sh
# Revoke  : provision-ci-deployer.sh --revoke --apply … (deletes the token
#           Secret; the credential stops authenticating immediately)
#
# This file is a bearer credential. Keep it at mode 0600, never commit it,
# never paste it into an issue. It belongs in the GitHub environment
# 'insight-test-stand', nowhere else.
apiVersion: v1
kind: Config
current-context: ${OUT_CONTEXT}
clusters:
  - name: ${OUT_CONTEXT}
    cluster:
      server: ${SERVER}
      certificate-authority-data: ${ca_b64}
contexts:
  - name: ${OUT_CONTEXT}
    context:
      cluster: ${OUT_CONTEXT}
      namespace: ${NAMESPACE}
      user: ${SA_NAME}
users:
  - name: ${SA_NAME}
    user:
      token: '${token}'
EOF
  umask "$old_umask"
  chmod 600 "$OUT_PATH"

  # Belt and braces: the variable dies with the process anyway, but an
  # explicit unset keeps it out of anything that inspects the environment
  # later in a longer script.
  unset token
  ok "wrote $OUT_PATH (mode $(ls -l "$OUT_PATH" | cut -c1-10))"
  note "        contents deliberately not printed"
}

wait_for_token() {
  local i
  for i in $(seq 1 "$TOKEN_WAIT_S"); do
    if [ -n "$(kadm -n "$NAMESPACE" get secret "$TOKEN_SECRET" \
      -o go-template='{{ if .data.token }}filled{{ end }}' 2>/dev/null || true)" ]; then
      ok "token controller populated $TOKEN_SECRET after ${i}s"
      return 0
    fi
    sleep 1
  done
  die "the token controller never populated '$TOKEN_SECRET' within ${TOKEN_WAIT_S}s. On clusters that disable the legacy token controller, mint with TokenRequest instead and accept the expiry — see the runbook."
}

# ═══════════════════════════════════════════════════════════════════════════
#  MODES
# ═══════════════════════════════════════════════════════════════════════════
if [ "$MODE" = "verify" ]; then
  run_verification
  exit 0
fi

hdr "plan (mode: $MODE)"

case "$MODE" in
provision | rotate)
  if [ "$MODE" = "rotate" ]; then
    note "delete  secret/${TOKEN_SECRET} -n ${NAMESPACE}   (invalidates the current CI token)"
    note "re-apply the manifests below, then rewrite ${OUT_ABS}"
  else
    note "apply the manifests below, then write ${OUT_ABS} (mode 0600)"
  fi
  note ""
  render_manifests
  note ""
  note "then, against the assembled kubeconfig:"
  print_verification_plan
  ;;
revoke)
  note "delete  secret/${TOKEN_SECRET} -n ${NAMESPACE}"
  note ""
  note "The ServiceAccount, Role and both RoleBindings are LEFT IN PLACE: they"
  note "grant nothing without a token, and keeping them makes re-issuing a"
  note "credential a one-command operation. Use --purge to remove them too."
  ;;
purge)
  note "delete  secret/${TOKEN_SECRET}          -n ${NAMESPACE}"
  note "delete  rolebinding/${RB_ADMIN}         -n ${NAMESPACE}"
  note "delete  rolebinding/${RB_SUPPLEMENT}    -n ${NAMESPACE}"
  note "delete  role/${ROLE_NAME}               -n ${NAMESPACE}"
  note "delete  serviceaccount/${SA_NAME}       -n ${NAMESPACE}"
  note ""
  note "Deleting the ServiceAccount is the definitive revocation: its UID is"
  note "embedded in every token it ever issued and re-checked on every request."
  ;;
esac

if [ "$APPLY" != "1" ]; then
  hdr "dry run"
  note "Nothing was created, deleted or written."
  note "Re-run with ${C_CYA}--apply${C_RST} to execute the plan above."
  exit 0
fi

# ─── Execute ──────────────────────────────────────────────────────────────
hdr "applying"

case "$MODE" in
provision | rotate)
  if [ "$MODE" = "rotate" ]; then
    kadm -n "$NAMESPACE" delete secret "$TOKEN_SECRET" --ignore-not-found
    ok "deleted the previous token Secret — the old CI token is now dead"
  fi
  render_manifests | kadm apply -f -
  wait_for_token
  assemble_kubeconfig
  run_verification
  hdr "next"
  note "1. Load it into the GitHub environment, base64-encoded on a single"
  note "   line (that is the form the deploy workflow decodes), never into"
  note "   the repo:"
  note "     base64 < \"$OUT_ABS\" | tr -d '\\n' \\"
  note "       | gh secret set TEST_STAND_KUBECONFIG --env insight-test-stand"
  note "2. Then remove the local copy, or keep it at 0600 in ~/.kube only."
  note "   Full procedure: docs/components/deployment/specs/sop/credentials-runbook.md"
  ;;
revoke)
  kadm -n "$NAMESPACE" delete secret "$TOKEN_SECRET" --ignore-not-found
  ok "token Secret deleted — the credential no longer authenticates"
  warn "the GitHub environment still holds the dead kubeconfig; replace or delete it"
  ;;
purge)
  kadm -n "$NAMESPACE" delete secret "$TOKEN_SECRET" --ignore-not-found
  kadm -n "$NAMESPACE" delete rolebinding "$RB_ADMIN" --ignore-not-found
  kadm -n "$NAMESPACE" delete rolebinding "$RB_SUPPLEMENT" --ignore-not-found
  kadm -n "$NAMESPACE" delete role "$ROLE_NAME" --ignore-not-found
  kadm -n "$NAMESPACE" delete serviceaccount "$SA_NAME" --ignore-not-found
  ok "ServiceAccount and RBAC removed"
  warn "the GitHub environment still holds the dead kubeconfig; replace or delete it"
  ;;
esac
