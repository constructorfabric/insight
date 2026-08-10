# SOP — Test-Stand CI Credentials

**Audience**: platform engineers with an admin kubeconfig for the test-stand cluster, and repo admins on `constructorfabric/insight`.
**Covers**: provisioning, verifying, storing, rotating, revoking and cleaning up the credential CI uses to deploy the Insight umbrella chart onto the test stand (issue #2244, decision D5).
**Last verified**: 2026-08-10.

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
| Persona email / password (and, in `override` mode only, the bootstrap principal's credentials) | Same GitHub environment | Same |
| Public stand URL, and the smoke login mode | Same environment, as **variables** `TEST_STAND_BASE_URL` and `TEST_STAND_SMOKE_LOGIN_MODE` (not secrets) | Same, and both are public anyway |

The split is the whole point of D5: **admin kubeconfigs stay human-only.** CI gets
a credential that is namespace-scoped and destroyable, and nothing else.

The provisioning tool is
[`deploy/gitops/scripts/provision-ci-deployer.sh`](../../../../../deploy/gitops/scripts/provision-ci-deployer.sh).
Its header explains *why* a ServiceAccount token beats a copied admin kubeconfig
(one-line answer: a token can be revoked; a client certificate cannot be revoked
without rotating the cluster CA). Read it once before your first run.

---

## 1. Prerequisites

1. `kubectl` ≥ 1.27 and `git` on PATH — `make doctor` from `deploy/gitops/` covers
   the wider toolchain.
2. An admin kubeconfig for the target cluster, readable at a path you know.
   This runbook writes it as `~/.kube/<stand>-admin.kubeconfig`.
3. The **exact cluster name** that kubeconfig's context resolves to. Find it
   without acting on anything:

   ```bash
   kubectl --kubeconfig ~/.kube/<stand>-admin.kubeconfig config current-context
   kubectl --kubeconfig ~/.kube/<stand>-admin.kubeconfig config view -o json \
     | jq -r '.contexts[] | "\(.name) -> \(.context.cluster)"'
   ```

   You will pass that cluster name as `--expect-cluster`. The script refuses to
   do anything if the kubeconfig resolves to a different cluster, which is the
   mechanical guard against provisioning a standing grant on the wrong stand.
4. The application namespace already exists (`insight` by default). See
   §7 "First install into an empty namespace" if it does not.
5. `gh` authenticated with admin rights on the repository, for §4.

---

## 2. Provision

### 2.1 Read the plan first

The script is **dry-run by default**. It reads the cluster, prints the exact
manifests it would apply and the exact assertions it would run, and exits
without writing anything:

```bash
cd deploy/gitops
./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>'
```

Read the printed manifests. You are looking for five things:

- the binding is a `RoleBinding`, not a `ClusterRoleBinding`;
- its `roleRef` is `ClusterRole/admin` — a cluster role bound namespace-wide, which
  is not the same as a cluster-wide grant;
- the supplemental `Role` lists only `gateway.networking.k8s.io`, `argoproj.io`,
  `onepassword.com` and `cert-manager.io`;
- there is a **second, much smaller** `Role` + `RoleBinding` in the `airbyte`
  namespace. The umbrella renders its `insight-airbyte-auth-reader` Role and
  RoleBinding *there* rather than in `insight` (that is what `airbyte.namespace`
  in the values file decides), so a credential scoped only to `insight` fails
  every upgrade. That supplement grants exactly `create`/`update` on Roles and
  RoleBindings in `airbyte`, plus `get` on the single Secret `airbyte-auth-secrets`
  by name — it deliberately cannot read anything else in that namespace;
- the token `Secret` carries no `data:` block (the token controller fills it).

If the cluster name does not match, the script prints a refusal banner and exits
2 having touched nothing. That is the expected outcome of a typo — do not "fix"
it by changing `--expect-cluster` until you are certain which cluster you mean.

### 2.2 Apply

```bash
./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>' \
  --out ~/.kube/insight-test-stand-ci-deployer.kubeconfig \
  --apply
```

What happens, in order:

1. the manifests are applied (idempotent — re-running is safe and is the
   supported way to repair drift);
2. the script waits for the token controller to populate the Secret;
3. the kubeconfig is assembled at `--out` with **mode 0600** and its contents are
   never printed;
4. the scope assertions run, and a single failure aborts with a non-zero exit and
   a "do not put this kubeconfig in a GitHub environment" message.

`--out` defaults to `~/.kube/insight-test-stand-ci-deployer.kubeconfig`. The
script refuses any path inside a git work tree unless that path is gitignored —
a bearer token one `git add -A` away from a public repository is not a risk worth
carrying.

### 2.3 Context name

The generated kubeconfig's context is named `insight-test-stand` by default,
matching the gitops convention `insight-<env>` documented in
`deploy/gitops/README.md`. That matters because the Makefile's `kube-ctx` target
refuses to act unless `kubectl config current-context` equals the environment
inventory's `kubeContext`. If your inventory says something else, either fix the
inventory or pass `--context-name <name>` when provisioning.

---

## 3. Verify the scope

The apply run verifies automatically. Re-verify at any time — after a cluster
upgrade, after someone edits the Role, before you trust a run — without
re-provisioning:

```bash
./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>' \
  --out ~/.kube/insight-test-stand-ci-deployer.kubeconfig \
  --verify-only
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
| `TEST_STAND_SEED_EMAIL` | the address `seed-stand.sh --email` is given, e.g. `email_development_lead@company.nonpresent` | kept as a secret rather than a variable purely so it is masked in run logs |
| `TEST_STAND_PERSONA_PASSWORD` | the password every seeded persona signs in with — on a seeded-realm stand this is the realm generator's `DEV_PASSWORD` constant (`insight_seed.keycloak_realm`), which every generated user shares | required in `password` mode. The workflow exports it to pytest as **`SMOKE_PERSONA_PASSWORD`** — not `INSIGHT_STAND_PERSONA_PASSWORD`, which belongs to a different helper the smoke suite deliberately does not use |
| `TEST_STAND_BOOTSTRAP_EMAIL` | the one principal that really authenticates in `override` mode | **`override` mode only.** Not needed on a stand whose realm serves a password form — leave both unset there rather than provisioning credentials nothing reads |
| `TEST_STAND_BOOTSTRAP_PASSWORD` | that principal's IdP password | as above |
| `TEST_STAND_OIDC_CLIENT_SECRET` | the confidential OIDC client secret | **normally unnecessary** — the deploy workflow reads the value out of the cluster instead. See §4.3 |

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
gh secret set TEST_STAND_SEED_EMAIL       --env "$ENVIRONMENT" --repo "$REPO"
gh secret set TEST_STAND_PERSONA_PASSWORD --env "$ENVIRONMENT" --repo "$REPO"

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

# How the smoke stage logs in. Set it EXPLICITLY, even though unset falls
# through to the suite's own default of `password`: a value left over from an
# earlier configuration keeps the gate in impersonation mode silently, and an
# environment whose variable list states the mode is one a reader can check.
gh variable set TEST_STAND_SMOKE_LOGIN_MODE --env "$ENVIRONMENT" --repo "$REPO" \
  --body 'password'

gh variable list --env "$ENVIRONMENT" --repo "$REPO"
```

`password` means every persona authenticates as themselves, which is what a
stand whose realm carries a local user per persona can serve, and it is the mode
to use on the test stand. `override` means one principal authenticates and every
persona session is minted from it through the product's own view-as path; it is
the fallback for a stand whose realm federates login to an external provider and
therefore serves no password form at all. The mode decides which secrets above
are required — `password` needs `TEST_STAND_PERSONA_PASSWORD`, `override` needs
the two `TEST_STAND_BOOTSTRAP_*` values — and the workflow checks that before it
touches the cluster rather than twenty minutes later.

### 4.3 When `TEST_STAND_OIDC_CLIENT_SECRET` is needed

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

### 4.4 Why the secrets may look missing to a job

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

# 2. Plan, then rotate.
./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>' \
  --out ~/.kube/insight-test-stand-ci-deployer.kubeconfig \
  --rotate                     # prints the plan and exits

./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>' \
  --out ~/.kube/insight-test-stand-ci-deployer.kubeconfig \
  --rotate --apply             # deletes the old token, mints a new one,
                               # rewrites --out, re-runs the assertions

# 3. Upload the new one (base64, single line — see §4.2).
base64 < ~/.kube/insight-test-stand-ci-deployer.kubeconfig | tr -d '\n' \
  | gh secret set TEST_STAND_KUBECONFIG --env "$ENVIRONMENT" --repo "$REPO"

# 4. Prove it end to end before you walk away.
gh workflow run build-images.yml --repo "$REPO" --ref main
```

Then do §8 (cleanup).

Rotating the seed email is just `gh secret set` again — no cluster action. The
persona password is not, and it is worth being precise about why. On a stand
whose realm is generated from the seed roster, the IdP's copy of that password
is a constant in the realm generator (`insight_seed.keycloak_realm`), shared by
every user it emits: changing the GitHub secret alone changes nothing on the
stand and breaks the gate on the next merge. Moving that value means changing
the generator, regenerating and re-applying the realm, and setting the secret —
in that order, in one sitting. Until the generator learns to take a password
instead of embedding one, treat this secret as *masking* a published constant in
a public log rather than as a rotatable credential, and see the stand
environment's README ("Known gaps") for the follow-up that fixes it properly.

---

## 6. Revoke

Two levels. Both are immediate: the API server re-validates a ServiceAccount
token against the Secret and against the ServiceAccount's UID on every request,
so the credential stops working as soon as the delete propagates. (That
re-validation depends on `--service-account-lookup`, on by default since 1.7 —
the confirmation step below is what actually proves it on *your* cluster, so do
not skip it.)

**Level 1 — kill the token, keep the identity.** Use when the credential leaked
but you still want CI to work after re-issuing.

```bash
./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>' \
  --revoke --apply
```

**Level 2 — remove the identity entirely.** Use when decommissioning the stand
or the CI integration.

```bash
./scripts/provision-ci-deployer.sh \
  --kubeconfig ~/.kube/<stand>-admin.kubeconfig \
  --expect-cluster '<cluster-name>' \
  --purge --apply
```

Either way, clear the stored copy too — a dead credential in a secret store is a
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
its own reconcile Role with `argoproj.io` and `onepassword.com` rules. You almost
certainly ran with `--no-supplement`, or someone trimmed the supplemental Role.
Re-run provisioning without `--no-supplement`.

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
The generated kubeconfig's context name and the environment inventory's
`kubeContext` disagree. Re-provision with `--context-name <inventory value>`, or
pass `KUBE_CTX=<generated name>` on the make invocation.

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
See §4.4.

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
  re-verify the scope with `--verify-only`.
- **Delete it** once it is in the GitHub environment. Re-issuing is one
  `--rotate --apply` away, and a credential that does not exist on your laptop
  cannot leak from it. This is the recommended default.

Finally, confirm nothing sensitive reached the repository. The gitops tree ships
`.gitleaks.toml` for the pre-commit hook; run it explicitly if you have it:

```bash
gitleaks detect --source <path-to-repo> --config <path-to-repo>/deploy/gitops/.gitleaks.toml
```

---

## 9. Done when

- `--verify-only` passes every assertion against the credential CI will use.
- `gh api "repos/$REPO/environments/$ENVIRONMENT/deployment-branch-policies"`
  lists exactly `main`.
- `gh secret list --env "$ENVIRONMENT"` shows the kubeconfig, the seed email and
  the persona password — and nothing else, unless you consciously chose the
  `override` login mode (the two `TEST_STAND_BOOTSTRAP_*` values) or the
  injected client secret of §4.3.
- `gh variable list --env "$ENVIRONMENT"` shows `TEST_STAND_BASE_URL` and
  `TEST_STAND_SMOKE_LOGIN_MODE`, the latter reading `password` on a stand whose
  realm serves a password form.
- One full deploy run on `main` is green.
- No kubeconfig, token or password exists anywhere in the repository work tree,
  and every local kubeconfig is mode 0600.
