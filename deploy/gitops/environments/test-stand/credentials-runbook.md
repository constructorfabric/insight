# SOP — Test-Stand CI Credentials

**Audience**: platform engineers with an admin kubeconfig for the test-stand cluster, and repo admins on `constructorfabric/insight`.
**Covers**: provisioning, verifying, storing, rotating, revoking and cleaning up the credential CI uses to deploy the Insight umbrella chart onto the test stand (issue #2244, decision D5).
**Last verified**: 2026-08-11.

> Every value in this document is a placeholder. Substitute your own and never
> paste a real one back in — this repository is public, and so are its issues,
> pull requests and workflow run logs.

---

## 0. What exists, and where it lives

| Thing | Where it lives | Who can read it |
|---|---|---|
| Admin kubeconfig for the stand | An operator's laptop / password manager | Humans only. **Never** goes into GitHub. |
| `ServiceAccount insight/ci-deployer` + its RBAC | The cluster | Anyone with cluster read access |
| The CI kubeconfig (server + CA + SA token), base64-encoded | GitHub environment `insight-test-stand`, secret `TEST_STAND_KUBECONFIG` | Jobs that declare that environment, on `main` only |
| The persona password, and optionally an explicit seed dev-lead address | Same GitHub environment | Same |
| Public stand URL | Same environment, as **variable** `TEST_STAND_BASE_URL` (not a secret) | Same, and it is public anyway |

The split is the whole point of D5: **admin kubeconfigs stay human-only.** CI gets
a credential that is namespace-scoped and destroyable, and nothing else.

Provisioning is three `make` targets in
[`deploy/gitops/Makefile`](../../Makefile) —
`provision-ci`, `verify-ci-credential`, `revoke-ci` — acting on the committed,
static RBAC manifest
[`deploy/gitops/environments/test-stand/ci-deployer-rbac.yaml`](ci-deployer-rbac.yaml).
[`deploy/gitops/scripts/provision-ci-deployer.sh`](../../scripts/provision-ci-deployer.sh)
is a thin CLI wrapper over the same targets, kept only because this runbook
names it as the entrypoint; `make` directly works identically.

One-line answer to "why a ServiceAccount token and not a copied admin
kubeconfig": a token can be revoked (delete the Secret, or the ServiceAccount
itself); a client certificate cannot be revoked without rotating the cluster
CA, which invalidates every other certificate on the cluster at once.

---

## 1. Prerequisites

1. `kubectl` ≥ 1.27, `yq` and `git` on PATH — `make doctor` from `deploy/gitops/`
   covers the wider toolchain.
2. An admin kubeconfig for the target cluster, with its **current-context**
   equal to `deploy/gitops/environments/test-stand/inventory.yaml`'s
   `kubeContext` (or pass `KUBE_CTX=<your context>` on every `make` call
   below). The Makefile's `kube-ctx` target refuses to act otherwise — the
   mechanical guard against provisioning a standing grant on the wrong stand.
   Confirm without acting on anything:

   ```bash
   kubectl config current-context
   yq -r '.kubeContext' deploy/gitops/environments/test-stand/inventory.yaml
   ```
3. The application namespace already exists (`insight` by default). See
   §7 "First install into an empty namespace" if it does not.
4. `gh` authenticated with admin rights on the repository, for §4.

---

## 2. Provision

### 2.1 Read the plan first

The manifest **is** the plan — it is committed, static YAML, so reading it
takes the place of a dry-run print. Optionally validate it against the
cluster without applying anything:

```bash
cat deploy/gitops/environments/test-stand/ci-deployer-rbac.yaml
kubectl --context "$(yq -r '.kubeContext' deploy/gitops/environments/test-stand/inventory.yaml)" \
  apply --dry-run=client -f deploy/gitops/environments/test-stand/ci-deployer-rbac.yaml
```

Read the manifest. You are looking for five things:

- the binding is a `RoleBinding`, not a `ClusterRoleBinding`;
- its `roleRef` is `ClusterRole/admin` — a cluster role bound namespace-wide, which
  is not the same as a cluster-wide grant;
- the supplemental `Role` lists only `gateway.networking.k8s.io`, `argoproj.io`,
  `onepassword.com` and `cert-manager.io`;
- the manifest does **not** touch the `airbyte` namespace. The umbrella renders
  its `insight-airbyte-auth-reader` Role and RoleBinding *there* rather than in
  `insight` (that is what `airbyte.namespace` in the values file decides), so a
  credential scoped only to `insight` fails every upgrade — but authorising a
  deployment identity over a namespace this repository does not own is
  infrastructure policy, owned by whoever installs Airbyte. `make
  verify-ci-credential` (§3) asserts the grant exists there; it never creates it;
- the token `Secret` carries no `data:` block (the token controller fills it).

If your kube-context does not resolve to the inventory's `kubeContext`, the
Makefile's `kube-ctx` precondition refuses with a clear error and nothing is
touched. That is the expected outcome of a typo — do not "fix" it by overriding
`KUBE_CTX` until you are certain which cluster you mean.

### 2.2 Apply

```bash
make -C deploy/gitops provision-ci ENV=test-stand
```

What happens, in order:

1. the manifest is applied (idempotent — re-running is safe and is the
   supported way to repair drift);
2. `make` waits for the token controller to populate the Secret;
3. the kubeconfig is assembled at `CI_DEPLOYER_OUT` with **mode 0600** and its
   contents are never printed;
4. `verify-ci-credential` (§3) runs, and a single failing assertion aborts with
   a non-zero exit and a "do not put this kubeconfig in a GitHub environment"
   message.

`CI_DEPLOYER_OUT` defaults to `~/.kube/insight-test-stand-ci-deployer.kubeconfig`
(override with `CI_DEPLOYER_OUT=<path> make …`). `provision-ci` refuses any path
inside a git work tree unless that path is gitignored — a bearer token one
`git add -A` away from a public repository is not a risk worth carrying.

### 2.3 Context name

The generated kubeconfig's context is named after `KUBE_CTX` itself — the
cluster context you just provisioned against — so it always matches the
environment inventory's `kubeContext`, and `KUBECONFIG=<out> make deploy
ENV=test-stand` works with no further `KUBE_CTX` override.

---

## 3. Verify the scope

The apply run verifies automatically. Re-verify at any time — after a cluster
upgrade, after someone edits the Role, before you trust a run — without
re-provisioning:

```bash
make -C deploy/gitops verify-ci-credential ENV=test-stand
```

### 3.1 What it asserts

**Must be `yes`** — the deploy and seed stages genuinely need these:

| Check | Why the deploy needs it |
|---|---|
| `create jobs -n insight` | the seed stage renders and applies a Job |
| `get pods/log -n insight` | seed follows the Job's log; diagnostics tail failed containers |
| `create secrets -n insight` | helm stores release state as Secrets in the namespace |
| `update deployments.apps -n insight` | `helm upgrade`, and the post-upgrade rollout restart |
| `create roles.rbac.authorization.k8s.io -n insight` | the chart renders its own reconcile RBAC |
| `create workflowtemplates.argoproj.io -n insight` | the chart renders WorkflowTemplates and CronWorkflows |
| `update httproutes.gateway.networking.k8s.io -n insight` | the umbrella renders both edge routes from `gateway.route` / `keycloak.route`, so `helm upgrade` writes them on every run |
| `get persistentvolumeclaims -n insight` | the git-cli-proxy renders a claim for its clone cache, and `helm upgrade` reads it before patching |
| `update persistentvolumeclaims -n insight` | its chart and version labels change on every bump |
| `create roles.rbac.authorization.k8s.io -n airbyte` | the chart renders `insight-airbyte-auth-reader` into the Airbyte namespace, not the release one |
| `create rolebindings.rbac.authorization.k8s.io -n airbyte` | same object, the binding half |

**Must be `no`** — this is the containment claim:

| Check | |
|---|---|
| `'*' '*' --all-namespaces` | the blanket check |
| `get pods -n airbyte` | the cross-namespace grant is RBAC-only; it reads one Secret by name and nothing else |
| `list namespaces --all-namespaces` | cannot enumerate the cluster |
| `create namespaces --all-namespaces` | see §7 |
| `get secrets --all-namespaces` | the datastore namespaces' credentials stay out of reach |
| `list nodes --all-namespaces` | no cluster-scoped reads |
| `create clusterrolebindings… --all-namespaces` | cannot widen its own grant |
| `get pods -n kube-system` | no unrelated namespace, control plane included |
| `delete persistentvolumeclaims -n insight` | the clone cache is annotated `helm.sh/resource-policy: keep` and outlives the release; a credential that could delete it could discard every synced repository |

### 3.2 One RBAC subtlety worth knowing

`kubectl get namespace insight` may succeed while `kubectl get namespaces`
(the list) is forbidden, and both are correct. A request naming a single
namespace is evaluated *as a request in that namespace*, so a RoleBinding there
can satisfy it; a list is a cluster-scoped request that only a
ClusterRoleBinding can satisfy. Every "must be `no`" assertion above passes
`--all-namespaces`, which sends an empty namespace, so the assertions test what
they claim to test. Do not "fix" a `yes` from `kubectl get ns insight` — it is
not a scope leak.

### 3.3 Manual spot check

```bash
export KUBECONFIG=~/.kube/insight-test-stand-ci-deployer.kubeconfig
kubectl auth whoami                 # system:serviceaccount:insight:ci-deployer
kubectl get pods                    # works — the context defaults to the namespace
kubectl get ns                      # Error from server (Forbidden)
kubectl -n kube-system get pods      # Error from server (Forbidden)
unset KUBECONFIG
```

---

## 4. Create the GitHub environment and load the secrets

Set the repo once for the commands below:

```bash
REPO=constructorfabric/insight
ENVIRONMENT=insight-test-stand
```

### 4.1 Create the environment, restricted to `main`

```bash
# Create it, and declare that it uses a custom branch allow-list.
gh api --method PUT "repos/$REPO/environments/$ENVIRONMENT" --input - <<'JSON'
{
  "deployment_branch_policy": {
    "protected_branches": false,
    "custom_branch_policies": true
  }
}
JSON

# Then allow exactly one branch.
gh api --method POST \
  "repos/$REPO/environments/$ENVIRONMENT/deployment-branch-policies" \
  -f name='main'
```

`protected_branches: true` would have been shorter, but it allows *every*
protected branch — this repository also protects `release-**`, and a
release-branch build must not reach the test stand. The custom allow-list says
`main` and means `main`.

Confirm:

```bash
gh api "repos/$REPO/environments/$ENVIRONMENT/deployment-branch-policies" \
  --jq '.branch_policies[].name'     # -> main
```

#### The allow-list is the security control, not a tidiness preference

Worth being explicit, because it is easy to read the list above as bookkeeping.
No secret store protects a value from the people who can run jobs that read it:
an environment secret is readable by **any** job that declares
`environment: insight-test-stand` on an allowed branch, and someone with write
access can add a step that prints it. Encryption at rest does nothing about
that.

So what actually bounds this credential is the pair "allow-list says `main`" and
"`main` is branch-protected". Together they mean a change that reads the secret
has to survive review before it can run. Widen the allow-list and that property
is gone — not degraded, gone — which is why:

* a pull request from a fork cannot read it (GitHub withholds environment
  secrets from fork PRs, and the allow-list would refuse anyway);
* `secrets: inherit` was removed from the calling job. Inheriting would have
  handed this job every repository secret, including the App key that bypasses
  branch protection on `main` — a worse exposure than the kubeconfig, and one
  that would have undone the control described here;
* a **rehearsal** environment on a personal fork may legitimately allow a
  feature branch, and that widening is exactly why it must stay on the fork.
  Never copy a rehearsal allow-list into `constructorfabric/insight`.

#### This credential is a deliberate stopgap

A GitHub environment secret is a static string with no refresh path, so this is
a long-lived, non-expiring token by necessity rather than by preference. That is
an accepted trade, not an oversight: the token is namespace-scoped, its blast
radius is asserted on every provisioning run, and revoking it is one command
with no cluster-wide consequence.

The property it cannot have is expiry. Treat the 90-day cadence in §5 as a real
obligation rather than a suggestion — nothing else ages this credential out.
Kubernetes' legacy-token cleanup only reaps tokens unused for a year, and a
token used on every merge never qualifies.

The durable answer is a credential that is never stored: Kubernetes 1.30+
structured authentication config can trust GitHub's OIDC issuer directly, so the
workflow presents a short-lived token minted per run and the cluster maps its
claims (`repository`, `ref`, `environment`) onto the same namespace-scoped RBAC
this script already creates. Trust then reads "runs from this repository, on this
ref, in this environment" instead of "whoever holds this string", and there is
nothing in GitHub left to leak. It is a cluster-operator change rather than a
repository one, which is the only reason it is not the starting point.

### 4.2 Load the secrets

| Secret | Value | Notes |
|---|---|---|
| `TEST_STAND_KUBECONFIG` | **base64 of** the file from §2.2 | the only credential CI needs for the cluster |
| `TEST_STAND_SEED_EMAIL` | an explicit dev-lead address for `seed-stand.sh --email`, e.g. `email_development_lead@company.nonpresent` | **normally unset** — the seeder reads that address back out of the realm the deploy just applied. Kept as a secret rather than a variable purely so it is masked in run logs. See §4.3 |
| `TEST_STAND_PERSONA_PASSWORD` | the password every seeded persona signs in with — the value `INSIGHT_SEED_PERSONA_PASSWORD` carried when the stand's realm was generated (default: the committed `insight-dev` constant, only acceptable for stands unreachable from outside) | required for the smoke stage. The workflow exports it to pytest as `INSIGHT_STAND_PERSONA_PASSWORD` |
| `TEST_STAND_OIDC_CLIENT_SECRET` | the confidential OIDC client secret | **normally unnecessary** — the deploy workflow reads the value out of the cluster instead. See §4.4 |

The kubeconfig goes in **base64-encoded**, single-line. The workflow decodes it
with `base64 -d` before writing it to the runner's temp directory. A raw
multi-line YAML secret survives `gh secret set` but is fragile in transit
(trailing-newline handling, and GitHub's log masker only masks the value as a
whole, so a multi-line secret is effectively unmasked line by line).

```bash
# From a file, no shell history exposure. `tr -d '\n'` makes the line-wrapping
# behaviour identical on macOS and Linux — GNU's `base64 -w0` is not portable.
base64 < ~/.kube/insight-test-stand-ci-deployer.kubeconfig | tr -d '\n' \
  | gh secret set TEST_STAND_KUBECONFIG --env "$ENVIRONMENT" --repo "$REPO"

# Typed values: omit --body and let gh prompt, so the value never enters
# the shell history or the process table.
gh secret set TEST_STAND_PERSONA_PASSWORD --env "$ENVIRONMENT" --repo "$REPO"

# TEST_STAND_SEED_EMAIL is deliberately absent from this block. Leave it unset
# and the seed stage reads the dev-lead address out of the realm the deploy
# stage just applied, which is the only way the two can be guaranteed to agree.
# §4.3 is when you would set it anyway, and what setting it means.

gh secret list --env "$ENVIRONMENT" --repo "$REPO"
```

Never `echo '<value>' | gh secret set …` and never `--body '<value>'`: both put
the cleartext into shell history and into `ps` output for the duration of the
call.

Two values are **variables**, not secrets. Neither is sensitive, and a masked
value in a log is harder to debug and buys nothing:

```bash
# The stand's public HTTPS origin — scheme and host, no trailing slash. The
# workflow refuses to start without it: nothing else aims the smoke suite.
gh variable set TEST_STAND_BASE_URL --env "$ENVIRONMENT" --repo "$REPO" \
  --body 'https://<public-stand-url>'

gh variable list --env "$ENVIRONMENT" --repo "$REPO"
```

Every persona authenticates as themselves with `TEST_STAND_PERSONA_PASSWORD`,
which is what a realm carrying a local user per persona serves — and the
workflow checks the secret is present before it touches the cluster rather than
twenty minutes later.

### 4.3 When `TEST_STAND_SEED_EMAIL` is needed

**On a stand whose realm was generated from this repository's roster module, it
is not.** Leave it unset. `seed-stand.sh` reads the dev-lead address out of the
realm the stand actually applied — the `<release>-keycloak-config-realms`
ConfigMap, the user whose `id` is the roster's dev-lead UUID — and seeds that
address. An explicit `--email` still wins where one is passed, so this remains
an escape hatch rather than a capability that was taken away.

Why it stopped being part of the required set is worth a paragraph, because the
failure it used to enable is one of the expensive ones. A stand's realm and its
seeded `identity.persons` rows are two projections of **one** roster: the realm
decides who can authenticate, the rows decide who a login resolves to. Every
persona address except the dev-lead's is derived from the roster and passes
through no input at all — which is precisely why the dev-lead is the only one
that can drift, and it did drift, because the realm was generated at deploy time
with one address while the seed was handed a different one out of this secret.
The result is not an error anybody sees: some personas sign in, the dev-lead
authenticates and resolves to nobody, every pod stays Ready and the release
reports `deployed`. Discovery removes the second input, so there is one writer
and nothing left for two copies to disagree about.

Note the mechanism, because "just blank the secret" is not it: an unset secret
expands to an empty string, and `--email ""` is an argument error rather than a
fallback. The workflow therefore **omits the flag entirely** when the secret is
empty. Deleting the secret and passing it through anyway are not the same thing.

Two shapes of stand still want the override, and they are the reason the secret
was made optional rather than deleted:

* **A realm provisioned some other way.** The realm ConfigMap is written by the
  bring-up outside this repository, not by the Helm release — its name, its key
  and its contents are not this repo's to guarantee. A stand that packs a broker
  realm, or no roster realm at all, offers nothing to discover; there the seeder
  has to be told, and this secret is how CI tells it. Discovery that finds
  nothing falls back to demanding `--email` by name, so such a stand fails
  loudly at the seed stage instead of seeding the wrong person.
* **Deliberately seeding a persona other than the realm's dev-lead.** Rare, and
  worth being explicit about the cost: the seeded roster and the realm then
  describe different people, which is the drift above, reintroduced on purpose.
  Expect the smoke gate's dev-lead login to fail, and say why in the change that
  sets it.

Setting it is not silent, but it is also not fatal. The seed stage annotates the
run with a warning naming the secret (never its value — the redactor masks
addresses either way), and where the seeder *can* read the realm it prints a
multi-line warning giving both addresses and states that it is using yours,
because an explicit flag wins. Nothing fails. Nothing downstream compares the
two either: the seed Job's preflight asserts only that an address is set, and
the sole detector is the smoke gate, which the workflow's `stages` input can
legitimately skip. That is the whole argument for keeping this out of the
required set — a required secret is a required opportunity to be wrong, and this
one has no check under it, only a louder log.

Removing it from an environment that already carries it:

```bash
gh secret delete TEST_STAND_SEED_EMAIL --env "$ENVIRONMENT" --repo "$REPO"
```

### 4.4 When `TEST_STAND_OIDC_CLIENT_SECRET` is needed

**On the test stand, it is not.** The deploy workflow reads the value out of the
cluster at deploy time — `kubectl -n insight get secret insight-oidc -o
jsonpath='{.data.client-secret}'` — and refuses to run the upgrade if that key
is empty, so there is exactly one copy of the value and it is the one the realm
was configured with. A second copy in GitHub would be a second thing to rotate
and a second way for the two to disagree.

It matters only for a deploy path that *composes* the authenticator's config
Secret itself. The gitops Makefile's `compose-app-secrets.sh` is such a path,
and it reads a different key name — `oidc-client-secret` — from the same Secret;
a stand whose Secret spells the key `client-secret` therefore has the value
present but invisible to that lookup, and the composed config lands with an
**empty** client secret. Pods stay Ready, the release reports `deployed`, and
the confidential-client token exchange fails for every login. That is one of the
reasons this environment does not use `make deploy` at all.

So: leave this GitHub secret unset unless you consciously choose to inject the
value from CI, and record that choice in the workflow header if you do. If you
are chasing the empty-client-secret symptom on some other path, the fix is to
add the key that path expects to the existing in-cluster Secret, once, by hand,
with a human credential — not to introduce a second copy in GitHub.

### 4.5 Why the secrets may look missing to a job

Environment secrets are readable only by a job that declares
`environment: <name>`. GitHub Actions does **not** allow `environment:` on a job
that calls a reusable workflow with `uses:` — the declaration has to go on the
jobs *inside* the called workflow. A job whose `${{ secrets.TEST_STAND_* }}` is
empty is almost always this, not a missing secret. Branch policies still
evaluate against the caller's ref, so the `main`-only restriction holds either
way.

---

## 5. Rotate

**Cadence**: every 90 days, and immediately on any of — an operator with a copy
leaves the team, the value is pasted anywhere it should not be, a runner or
laptop that held it is compromised, or the cluster CA is rotated (which
invalidates the embedded CA bundle even though the token itself survives).

Rotation deletes the old token first, so there is a window in which the secret
stored in GitHub is dead. Do it when no deploy is in flight, and re-upload
immediately.

```bash
# 1. Confirm nothing is deploying: the workflow's concurrency group is
#    `test-stand-deploy`, and it coalesces rather than cancels.
gh run list --repo "$REPO" --workflow build-images.yml --limit 5

# 2. Rotate: delete the old token, mint a new one, rewrite CI_DEPLOYER_OUT,
#    re-run the assertions. There is no separate "kill token, keep identity"
#    step any more — revoke-ci removes the whole ServiceAccount + RBAC, and
#    provision-ci re-creates all of it from the same committed manifest, so
#    a rotation is just running both in order.
make -C deploy/gitops revoke-ci    ENV=test-stand
make -C deploy/gitops provision-ci ENV=test-stand

# 3. Upload the new one (base64, single line — see §4.2).
base64 < ~/.kube/insight-test-stand-ci-deployer.kubeconfig | tr -d '\n' \
  | gh secret set TEST_STAND_KUBECONFIG --env "$ENVIRONMENT" --repo "$REPO"

# 4. Prove it end to end before you walk away.
gh workflow run build-images.yml --repo "$REPO" --ref main
```

Then do §8 (cleanup).

The seed email has no rotation, because on a stand that discovers it there is no
stored copy to rotate: the address moves when the realm is regenerated, and the
next seed follows it. If the environment carries an explicit
`TEST_STAND_SEED_EMAIL` (§4.3), changing it is `gh secret set` again with no
cluster action — but changing it *alone* re-opens the drift, because the realm
still names whoever it named. Move both, or delete the secret and let the realm
be the one writer. The persona password is a third case, and it is worth being
precise about why. On a stand whose realm is generated from the seed roster,
the IdP's copy of that password is whatever the generator
(`insight_seed.keycloak_realm`) was given in `INSIGHT_SEED_PERSONA_PASSWORD`,
shared by every user it emits — and a constant documented for local stands when
it was given nothing. So changing the GitHub secret alone changes nothing on the
stand and breaks the gate on the next merge. Rotating means setting the
generator's input, regenerating and re-applying the realm, and setting the
secret — in that order, in one sitting. A realm generated without that input
carries a value published in this repository, and the secret masks it in a
public log rather than making it private; the stand environment's README
("Known gaps") states what that costs.

---

## 6. Revoke

`revoke-ci` deletes the ServiceAccount, both RoleBindings, the Role and the
token Secret in one `kubectl delete -f` of the committed manifest — there is
no partial "kill the token, keep the identity" mode. Immediate: the API
server re-validates a ServiceAccount token against the Secret and against the
ServiceAccount's UID on every request, so the credential stops working as
soon as the delete propagates. (That re-validation depends on
`--service-account-lookup`, on by default since 1.7 — the confirmation step
below is what actually proves it on *your* cluster, so do not skip it.)
Re-issuing afterward is one `make provision-ci` away, from the same manifest.

```bash
make -C deploy/gitops revoke-ci ENV=test-stand
```

Then clear the stored copy too — a dead credential in a secret store is a
false sense of coverage and an obstacle to the next person debugging:

```bash
gh secret delete TEST_STAND_KUBECONFIG --env "$ENVIRONMENT" --repo "$REPO"
# Decommissioning the whole integration:
gh api --method DELETE "repos/$REPO/environments/$ENVIRONMENT"
```

Confirm the credential is dead:

```bash
KUBECONFIG=~/.kube/insight-test-stand-ci-deployer.kubeconfig kubectl get pods
# -> error: You must be logged in to the server (Unauthorized)
```

> This is exactly the operation that is **impossible** with a copied admin
> kubeconfig. Those authenticate with a client certificate, and Kubernetes
> consults no CRL and no OCSP responder — the only revocation is rotating the
> cluster CA, which invalidates every other client certificate at the same time
> and needs a control-plane restart. That asymmetry is the reason this whole
> runbook exists.

---

## 7. Troubleshooting

**`the token controller never populated '<name>' within Ns`**
The cluster runs without the legacy token controller (some hardened
distributions disable it). There is no long-lived token to mint. Either enable
it, or accept a bound token from `kubectl create token` plus a refresh mechanism
— and note that a static GitHub secret cannot hold an expiring token, so this
becomes a workflow change, not a secret change.

**`attempt to grant extra privileges` during `helm upgrade`**
RBAC escalation prevention: a subject cannot create a Role granting permissions
it does not itself hold, and `admin` does not carry `escalate`. The chart renders
its own reconcile Role with `argoproj.io` and `onepassword.com` rules. Someone
trimmed the supplemental `Role` in `ci-deployer-rbac.yaml` — diff it against
git history and re-run `make provision-ci ENV=test-stand` to repair drift.

**`could not get information about the resource <kind> … is forbidden`**
The chart grew a kind the Role does not enumerate — the guard working, not a
broken credential. Helm reads *every* rendered object before it plans the
patch, so one unreadable kind fails the whole upgrade. Either add the kind to
`ci-deployer-rbac.yaml` and re-apply (`make -C deploy/gitops provision-ci
ENV=test-stand`, which does not rotate the token), or turn the subchart off in
this environment's `values.yaml` when the stand does not need it —
`gitCliProxy.deploy: false` is there for that reason.

**`namespaces is forbidden` on a first install**
`helm upgrade --install --create-namespace` POSTs a Namespace at cluster scope,
but only when the release does not yet exist. This credential cannot create
namespaces, by design. Create the namespace once, by hand, with a human
credential:

```bash
kubectl --kubeconfig ~/.kube/<stand>-admin.kubeconfig create namespace insight
```

Do **not** grant namespace-create to CI to make this go away. Upgrades of an
existing release never reach that code path.

**`Error: context "…" does not exist` from the gitops Makefile**
Should not happen by construction — `provision-ci` names the generated
kubeconfig's context after `KUBE_CTX` itself, so it always matches the
environment inventory's `kubeContext`. If it does, the kubeconfig was hand-
edited or copied from a different environment; re-run `make provision-ci
ENV=test-stand`.

**A Gateway or an object in another namespace is unreadable**
Expected, and it does not block the deploy. The gateway controller's own
namespace is out of scope for this credential, and the only exception anywhere
is the narrow RBAC-plus-one-Secret grant in `airbyte` described in §2.1. The
release's two `HTTPRoute`s attach to a `Gateway` in that controller namespace by
reference, and the `Accepted` / `ResolvedRefs` conditions the deploy checks are
written back onto the routes **in `insight`** — so the post-upgrade route check
works without ever reading the `Gateway`. A preflight that wants to inspect the
`Gateway` object itself needs either a human credential or a separate, equally
narrow read-only grant in that namespace; file it as a follow-up rather than
widening this one.

**The workflow reports empty secrets**
See §4.5.

**The seed stage fails naming `--email`**
Discovery found no dev-lead address in the realm, and no `--email` was supplied.
Read it as a statement about the stand, not about the seeder: either the realm
ConfigMap is absent, or it holds no user carrying the roster's dev-lead UUID
(a broker realm does not), or `keycloakConfig` is disabled so nothing applies it.
Confirm which before reaching for `TEST_STAND_SEED_EMAIL` — setting the secret
makes the seed run, and on a stand whose realm genuinely lacks that person it
makes it run into a roster the realm cannot authenticate. §4.3 has the two cases
where setting it is the right answer.

---

## 8. Cleanup — do this every time

The local copy of the CI kubeconfig is a live credential. Treat it like one.

```bash
# 1. Nothing sensitive is sitting in the work tree.
git -C <path-to-repo> status --porcelain

# 2. Every local kubeconfig is owner-only. Do this for the admin one too.
chmod 600 ~/.kube/insight-test-stand-ci-deployer.kubeconfig
chmod 600 ~/.kube/<stand>-admin.kubeconfig
chmod 700 ~/.kube
ls -l ~/.kube/*.kubeconfig        # expect -rw------- on each

# 3. Remove throwaway copies. Prefer an overwriting delete where you have one.
#    macOS:  rm -P <file>          Linux (coreutils): shred -u <file>
rm -P /tmp/ci-deployer.* 2>/dev/null || true

# 4. If you typed a secret inline despite §4.2, drop it from history now.
history | tail -40                # find the offending entries
#   zsh:  fc -W after editing ~/.zsh_history   bash: edit ~/.bash_history
```

Decide deliberately whether to keep the local CI kubeconfig at all:

- **Keep it** (mode 0600, in `~/.kube`, never in the repo) if you expect to
  re-verify the scope with `make verify-ci-credential`.
- **Delete it** once it is in the GitHub environment. Re-issuing is one
  `make provision-ci` away, and a credential that does not exist on your
  laptop cannot leak from it. This is the recommended default.

Finally, confirm nothing sensitive reached the repository. The gitops tree ships
`.gitleaks.toml` for the pre-commit hook; run it explicitly if you have it:

```bash
gitleaks detect --source <path-to-repo> --config <path-to-repo>/deploy/gitops/.gitleaks.toml
```

---

## 9. Done when

- `make -C deploy/gitops verify-ci-credential ENV=test-stand` passes every
  assertion against the credential CI will use.
- `gh api "repos/$REPO/environments/$ENVIRONMENT/deployment-branch-policies"`
  lists exactly `main`.
- `gh secret list --env "$ENVIRONMENT"` shows the kubeconfig and the persona
  password — and nothing else, unless you consciously set an explicit dev-lead
  address (§4.3) or the injected client secret of §4.4. A `TEST_STAND_SEED_EMAIL`
  nobody meant to set is not inert: it silently outranks the realm.
- `gh variable list --env "$ENVIRONMENT"` shows `TEST_STAND_BASE_URL`.
- One full deploy run on `main` is green.
- No kubeconfig, token or password exists anywhere in the repository work tree,
  and every local kubeconfig is mode 0600.
