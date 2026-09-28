# insight-vhc runner image

Source of truth for how a runner in the `insight-vhc` pool is provisioned.
`bootstrap.sh` is what a machine runs on first boot; provisioning reads this
file rather than carrying a copy of its own.

The workflows in this repository call `helm`, `gh` and `git` without installing
them, so what this script puts on a machine is part of their contract. Changing
it changes whether those lanes pass, which is why it is versioned beside them.

## Which runs land here

Every x64 `runs-on` in `.github/workflows` carries one rule:

```yaml
runs-on: ${{ vars.INSIGHT_FORCE_GITHUB_HOSTED == 'true' && 'ubuntu-latest' || (github.event_name == 'pull_request' && (github.event.pull_request.head.repo.full_name != github.repository && 'ubuntu-latest' || 'insight-vhc-arc') || fromJSON('["self-hosted","linux","x64","insight-vhc"]')) }}
```

| Event | Runner |
|---|---|
| pull request from a branch of this repository | `insight-vhc-arc` — the ARC scale set, one ephemeral pod per job |
| pull request from a fork | `ubuntu-latest` |
| `push`, `merge_group`, `schedule`, `workflow_dispatch`, `workflow_run` | the shared `[self-hosted, linux, x64, insight-vhc]` pool — these five machines **and** the scale set, whichever takes the job |
| any of the above with the kill switch on | `ubuntu-latest` |

**A pull request runs on an ephemeral pod, not on these machines.** A machine
here is persistent, its job user has passwordless sudo and sits in the docker
group, and what one job leaves behind the next one finds. Pull-request code is
the one workload where that matters, so it goes to a pod that is destroyed
afterwards. These machines keep the work that reaches them only after review or
merge.

The scale set answers to its own name here rather than to the pool's labels. It
also carries `[self-hosted, linux, x64, insight-vhc]`, so it takes a share of
the non-PR work alongside these five; naming it directly is what pins a pull
request to it.

**A fork's pull request stays on a GitHub-hosted runner.** Approval for a fork
workflow is an admission gate, not a sandbox — the code still executes once
approved. The scale set runs docker-in-docker on a shared node, which is not a
boundary to put arbitrary external code behind, so that work goes to a runner
GitHub throws away.

**arm64 never routes here.** The pool is x86-only; the matrix legs stay on
`ubuntu-24.04-arm` under every event and with the kill switch on.

Left on hosted runners deliberately:

- `deploy-test-stand.yml` and `run-stand-suite.yml` — both read
  `TEST_STAND_KUBECONFIG` and the persona password against the
  `insight-test-stand` environment.
- `previews-helm.yml` — its render-contract test asserts an issuer URL that
  embeds the namespace helm resolved, and a runner pod supplies its own, so the
  lane renders `arc-runners` where the test expects `default`. Pinning the
  namespace in the test is the fix.

## The kill switch

`INSIGHT_FORCE_GITHUB_HOSTED` is a repository variable. Set it to exactly `true`
and every x64 job that would otherwise take a self-hosted runner, pool or scale
set, goes to `ubuntu-latest`; unset, empty or `false` leaves routing as
described above. The polarity is deliberate — an absent variable must not change
policy, and a variable nobody has created yet reads as empty.

Actions withholds `vars` from a pull request opened from a fork, so in those
runs the expression reads an empty string and the switch cannot speak. That is
why the rule compares the head repository against this one rather than relying
on the variable: a fork's pull request is hosted by the shape of the expression,
under every value the variable could have held.

Routing is not a security boundary either. A pull request runs the workflow from
its own merge commit, so a fork can rewrite any of these lines, and the runner is
picked before the first step executes.

## No secrets here

The registration token, the cloud credentials and any kubeconfig are injected at
provisioning time — the token arrives in `/etc/gha-runner/register.env`, which
this script consumes and deletes. Nothing in this directory is a secret and
nothing in it should become one.

## The five machines were levelled by hand

`insight-gha-runner-1` … `-5` were brought up before parts of this script
existed, and git 2.55, the job-started hook, helm and gh were installed on them
in place. They match this file today, but only because someone made them match.
A rebuilt machine must be provisioned from this script, or it will come up
without those and the chart-contract and API-calling lanes will fail on a
missing binary.

## What a job inherits from the one before it

These machines are persistent: nothing is torn down between jobs, and the work
tree, the local repository objects and the Docker cache are all still there when
the next job starts. That reuse is the point — it is most of why a job here
starts faster than a fresh hosted image.

It also means a job can read state no step of its own produced, and two kinds of
that are repaired by `ACTIONS_RUNNER_HOOK_JOB_STARTED` before the first step
runs:

- **Ownership.** A container job runs as root over the mounted work tree, so one
  killed before its cleanup leaves root-owned paths the next job's checkout
  cannot remove. The hook reclaims them.
- **Remote-tracking refs.** `actions/checkout` puts the requested ref in the
  work tree, but it does not remove the `refs/remotes/*` an earlier job fetched.
  A lane that resolves a remote ref would get whichever one happened to be left
  on that machine — a different answer per machine, and a stale one. The hook
  deletes them, so a job sees only the remote refs its own checkout or fetch
  created.

The hook deletes nothing else: local branches, tags, `HEAD`, the object store,
the index and the working tree all survive, and it never fetches. A job that
needs a remote ref is the thing that must fetch it.

It also cleans only a checkout that physically sits where the path says, with a
real `.git` directory of its own. The previous job could leave a symlink or a
`gitdir:` redirect behind, and following one would take the deletes into a
repository outside the work tree, so any redirected or otherwise non-standard
git directory is skipped rather than resolved.

## Notes on what it installs

- **git from the git-core PPA.** Ubuntu 24.04 ships 2.43, whose partial-clone
  behaviour fails `git-cli-proxy`'s promisor tests; the GitHub-hosted images
  take the same PPA.
- **helm and gh from release tarballs**, not the vendors' apt repositories:
  `baltocdn.com` rejects the TLS handshake from this network. The helm digest is
  pinned here because the checksum published beside the tarball shares its host.
- **`ACTIONS_RUNNER_HOOK_JOB_STARTED`.** Runs on the host before every job,
  container jobs included, and repairs the two things a previous job leaves
  behind — see the section above.
- **Docker's data root on `/srv/gha`**, the attached volume, so image layers do
  not fill the system disk.
