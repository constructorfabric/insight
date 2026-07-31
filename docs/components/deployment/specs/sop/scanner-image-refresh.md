# SOP — Scanner Image Refresh

## Why this is manual

Security workflows run their scanners as `docker run <image>@sha256:…` inside shell
steps, not as `FROM` lines. Dependabot's `docker` ecosystem parses Dockerfiles only,
so these pins are outside its reach.

## Cadence and owner

Quarterly. Owner: Gregory91G.

## Steps

1. List the pinned digests:
   `grep -rhoE '[a-zA-Z0-9./_-]+:[a-zA-Z0-9._-]+@sha256:[0-9a-f]{64}' .github/workflows/`
   — the tag part must not be restricted to digits: images such as
   `python3.12-bookworm-slim` would be missed.
2. For each image, resolve the current digest of the same major line.
3. Update the tag, the digest and the version comment in one edit — a digest that no
   longer matches its tag is worse than no pin at all, because review reads the tag.
4. Open one pull request covering every image.
5. Confirm each security workflow completes and renders its job summary.

## Done when

The pull request is merged and one full run of every security workflow is green on
the default branch.
