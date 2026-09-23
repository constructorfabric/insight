# insight-vhc runner image

Source of truth for how a runner in the `insight-vhc` pool is provisioned.
`bootstrap.sh` is what a machine runs on first boot; provisioning reads this
file rather than carrying a copy of its own.

The workflows in this repository call `helm`, `gh` and `git` without installing
them, so what this script puts on a machine is part of their contract. Changing
it changes whether those lanes pass, which is why it is versioned beside them.

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
