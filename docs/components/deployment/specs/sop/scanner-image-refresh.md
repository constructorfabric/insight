# SOP — Scanner Image Refresh

## Why this is manual

Security workflows run their scanners as `docker run <image>@sha256:…` inside shell
steps, not as `FROM` lines. Dependabot's `docker` ecosystem parses Dockerfiles only,
so these pins are outside its reach.

## Cadence and owner

Quarterly. Owner: Gregory91G.

## Steps

1. List the pinned digests:
   `grep -rhoE '[a-z0-9./-]+:[0-9.]+@sha256:[0-9a-f]{64}' .github/workflows/`
2. For each image, resolve the current digest of the same major line.
3. Update the digest and its version comment together — a digest without a matching
   comment is unreadable at review time.
4. Open one pull request covering every image.
5. Confirm each security workflow completes and renders its job summary.

## Done when

The pull request is merged and one full run of every security workflow is green on
the default branch.
