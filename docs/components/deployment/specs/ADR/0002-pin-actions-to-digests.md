# ADR-0002 — Pin third-party actions to commit digests, refresh with Dependabot

## Status

Accepted

## Context

A workflow step written as `uses: owner/action@v3` resolves the tag at run time. A
tag is mutable: whoever controls the action's repository can repoint it, which is
normal release practice and also the exact capability an attacker gains by taking
over that account. The repointed code then runs on our runners with the job's
token and secrets, and nothing in our repository changes.

A commit digest cannot be repointed. It also cannot move forward on its own, so a
digest that nobody refreshes silently ages past security fixes — trading one risk
for another unless refresh is automated.

## Decision

Reference third-party actions by 40-character commit digest, with the human-readable
version kept in a trailing comment. Actions published under `actions/*` stay on tags:
they live in GitHub's own organisation, and the residual risk does not justify the
extra churn.

Refresh through Dependabot (`.github/dependabot.yml`), weekly, in two groups:
`actions` and `base-images`. They are separated because a base-image bump is a
rebuild that can change runtime behaviour, while an action bump cannot — one failing
build must not hold back the other.

`npm` and `cargo` are deliberately out of scope for now; they are not the source of
the current findings and would add volume without addressing them.

## Consequences

- A compromised upstream tag no longer reaches our runners.
- Up to two dependency pull requests per week per repository, assigned for triage.
- Digests pinned inside shell steps (scanner images) are invisible to Dependabot and
  need the manual routine in `sop/scanner-image-refresh.md`.
- `dtolnay/rust-toolchain@stable` tracks a branch by design; pinning it would freeze
  the toolchain, so it is treated as an exception and left on the moving reference.
