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
runs-on: ${{ (vars.INSIGHT_FORCE_GITHUB_HOSTED == 'true' || github.repository != 'constructorfabric/insight' || (github.event_name == 'pull_request' && github.event.pull_request.head.repo.full_name != github.repository)) && 'ubuntu-latest' || 'insight-large' }}
```

| Event | Runner |
|---|---|
| pull request from a branch of this repository, Dependabot included | the job's `insight-*` label |
| pull request from a fork | `ubuntu-latest` |
| `push`, `merge_group`, `schedule`, `workflow_dispatch`, `workflow_run` | the job's `insight-*` label |
| any run inside a fork of this repository | `ubuntu-latest` |
| any of the above with `INSIGHT_FORCE_GITHUB_HOSTED=true` (the kill switch) | `ubuntu-latest` |

**Whoever wants the self-hosted runners pushes a branch to this repository;
forks never reach them.** Pushing a branch takes write access, so a pull
request from a branch here is trusted by where it comes from, not by who opened
it. Contributors with write access push branches here instead of opening pull
requests from their forks. Approval for an outside contributor's workflow is an
admission gate, not a sandbox, so fork pull requests stay on GitHub-hosted
runners.

A fork with Actions enabled runs its own pushes and pull requests; no runner
there serves the `insight-*` labels, so those runs go to GitHub-hosted runners
instead of queueing until they time out.

**Routing is not a security boundary.** A pull request runs the workflow from
its own merge commit, so its author can rewrite `runs-on`; the runner is picked
before any step executes.

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
