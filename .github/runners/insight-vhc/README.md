# insight-vhc runner image

Source of truth for how a runner in the `insight-vhc` pool is provisioned.
`bootstrap.sh` is what a machine runs on first boot; provisioning reads this
file rather than carrying a copy of its own.

The workflows in this repository call `helm`, `gh` and `git` without installing
them, so what this script puts on a machine is part of their contract. Changing
it changes whether those lanes pass, which is why it is versioned beside them.

## Which runs land here

The default rule, carried by every x64 `runs-on` in `.github/workflows`:

```yaml
runs-on: ${{ (vars.INSIGHT_FORCE_GITHUB_HOSTED == 'true' || github.event_name == 'pull_request') && 'ubuntu-latest' || fromJSON('["self-hosted","linux","x64","insight-vhc"]') }}
```

A pull request keeps the GitHub-hosted runner it had before; `merge_group`,
`push`, `schedule`, `workflow_dispatch` and `workflow_run` come here. Merge-queue
and post-merge runs are the bulk of the machine time and carry no fork code, so
moving them off the organisation's shared 20-job ceiling is where the wait goes
away. arm64 matrix legs never route here — the pool is x86-only and they stay on
`ubuntu-24.04-arm` under every event, including when the switch below is on.

Two lanes are exempt and stay on the pool for pull requests as well, because
they have been measured here and the win is large: `ci.yml` (`Lint and test`)
and `connectors-ddl.yml`. They carry the switch without the event test.

`deploy-test-stand.yml` never routes here. It reads `TEST_STAND_KUBECONFIG` and
the persona password, and this pool is persistent and also runs pull-request
code, so a deploy job holding stand credentials does not belong on it.

## The kill switch

`INSIGHT_FORCE_GITHUB_HOSTED` is a repository variable. Set it to exactly `true`
and every x64 job that would otherwise take the pool goes to `ubuntu-latest`;
unset, empty or `false` leaves routing exactly as described above. The polarity
is deliberate — an absent variable must not change policy, least of all for pull
requests, and a variable nobody has created yet reads as empty.

It does not reach a pull request opened from a fork: Actions withholds `vars`
from those runs, so the expression sees an empty string. For the default rule
that is harmless, since a fork's pull request is hosted anyway. For the two
exempt lanes it means the switch cannot pull a fork's pull request off the pool
— the lever there is the approval setting for fork pull requests.

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

## Notes on what it installs

- **git from the git-core PPA.** Ubuntu 24.04 ships 2.43, whose partial-clone
  behaviour fails `git-cli-proxy`'s promisor tests; the GitHub-hosted images
  take the same PPA.
- **helm and gh from release tarballs**, not the vendors' apt repositories:
  `baltocdn.com` rejects the TLS handshake from this network. The helm digest is
  pinned here because the checksum published beside the tarball shares its host.
- **`ACTIONS_RUNNER_HOOK_JOB_STARTED`.** A container job runs as root over the
  mounted work tree, so one killed before its cleanup leaves root-owned paths
  that the next job's checkout cannot remove. The hook repairs ownership on the
  host before every job.
- **Docker's data root on `/srv/gha`**, the attached volume, so image layers do
  not fill the system disk.
