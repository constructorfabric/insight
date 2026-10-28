# Runners

Every x64 job in `.github/workflows` runs on one of three self-hosted labels,
or on a GitHub-hosted runner. Each self-hosted runner is ephemeral: it takes
one job and is destroyed afterwards, so nothing carries over between jobs.

| Label | Machine | A job belongs here when |
|---|---|---|
| `insight-small` | a container: 1 CPU, 2-4 GiB, no Docker, no Rust toolchain | it starts no container, uses no secret beyond `GITHUB_TOKEN`, and its token has no `contents`, `packages`, `id-token` or `attestations` write. Short. |
| `insight-small-vm` | a small VM with Docker | short, but holds a secret, needs a write permission, or uses Docker lightly |
| `insight-large` | a large VM with Docker | it compiles, tests, builds images, or runs compose / ClickHouse / k3d |

## The rule

```yaml
runs-on: ${{ (vars.INSIGHT_FORCE_GITHUB_HOSTED == 'true' || (github.event_name == 'pull_request' && !(contains(fromJSON('["OWNER","MEMBER","COLLABORATOR"]'), github.event.pull_request.author_association) || contains(fromJSON(vars.INSIGHT_ARC_PR_AUTHORS || '[]'), github.event.pull_request.user.login)))) && 'ubuntu-latest' || 'insight-large' }}
```

| Event | Runner |
|---|---|
| pull request by a project member, from a branch or a fork | the job's `insight-*` label |
| pull request by anyone else, Dependabot included | `ubuntu-latest` |
| `push`, `merge_group`, `schedule`, `workflow_dispatch`, `workflow_run` | the job's `insight-*` label |
| any of the above with `INSIGHT_FORCE_GITHUB_HOSTED=true` (the kill switch) | `ubuntu-latest` |

**Only project members reach these runners.** A project member is an author
the event payload reports as `OWNER`, `MEMBER` or `COLLABORATOR`, or whose
login is in the `INSIGHT_ARC_PR_AUTHORS` repository variable, a JSON array such
as `["octocat"]`. The payload reports org membership only when it is public, so
a member whose membership is private shows as `CONTRIBUTOR` and needs either
the variable entry or public membership. Approval for an outside contributor's
workflow is an admission gate, not a sandbox, so their pull requests stay on
GitHub-hosted runners.

**Routing is not a security boundary.** A pull request runs the workflow from
its own merge commit, so its author can rewrite `runs-on`; the runner is picked
before any step executes.

Jobs that publish (`build-images.yml`, `gateway.yml`, `ghcr-cleanup.yml` and a
few others) also require the pull request to come from a branch of this
repository, not a fork.

## Exceptions

- **arm64 legs** stay on `ubuntu-24.04-arm` under every event and with the kill
  switch on.
- **`deploy-test-stand.yml`** uses `insight-large` for candidate validation
  on `push` and `workflow_dispatch`, with the GitHub-hosted kill switch.
  Published-release deployments and **`run-stand-suite.yml`** stay on
  GitHub-hosted runners. All retain the `insight-test-stand` environment.

## Caches

`.github/actions/ci-cache` wires a job to the shared build caches: sccache for
Rust, `runs-on/cache` for downloads, BuildKit's S3 cache for images. Only a
`push` to `main` writes, through the `ci-trusted` environment; every other run
reads. On a runner without the caches the action prints a notice and does
nothing.
