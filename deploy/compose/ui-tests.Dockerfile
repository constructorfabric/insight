# Browser runner for the deployed-stand journeys (tests/stand).
#
# Built ad hoc and run as a one-shot container against a running stand —
# deliberately NOT a docker-compose service, so it can never become part of the
# stack it is testing.
#
#   docker build -f deploy/compose/ui-tests.Dockerfile -t insight-ui-tests:dev .
#   docker run --rm --network container:insight-gateway \
#     -e INSIGHT_STAND_BASE_URL=http://localhost:8080 \
#     -e INSIGHT_STAND_PERSONA_PASSWORD=... \
#     -v "$PWD/src/ingestion/tools/seed/manifest.json:/stand/manifest.json:ro" \
#     -e INSIGHT_STAND_MANIFEST=/stand/manifest.json \
#     insight-ui-tests:dev
#
# The network namespace is shared with the gateway rather than joining the
# `insight` network as a peer, so ONE url — http://localhost:<gateway port> —
# serves both the HTTP clients and the browser. `localhost` is the only host
# name a browser trusts over plain http, and the session cookie is
# `__Host-`-prefixed, so it is dropped on any other origin. Compose service
# names still resolve: the namespace is the gateway's, and the gateway is on
# the `insight` network.
#
# Python only. No Node, no package.json, no npm — the Playwright *Python*
# bindings drive the same browsers, and a JS toolchain here would mean a second
# dependency graph to keep in step for no gain.
#
# Dependencies come from tests/pyproject.toml + tests/uv.lock, the SAME pair the
# host runner syncs from, so the two cannot drift. uv also provisions the
# interpreter: this base image ships Python 3.12, below the suite's >=3.13
# floor, and rather than lowering the floor or skipping the metadata check, uv
# fetches a managed 3.13 and builds the venv on it.

# Version policy: pinned by tag AND digest, so a rebuild can never silently
# pick up a different image. Resolved 2026-07-31 as the newest stable tag in
# the registry list (v1.61.0, cross-checked against the
# microsoft/playwright-python v1.61.0 release and PyPI). -noble is the Ubuntu
# 24.04 LTS base; a -resolute variant of the same version also exists.
FROM mcr.microsoft.com/playwright/python:v1.61.0-noble@sha256:a9731514f24121d1dcd25d58d0a38146646d290a5998fd80d3e533e7b5e21c69

# uv, pinned the same way (0.12.0).
COPY --from=ghcr.io/astral-sh/uv:0.12.0@sha256:606e70c71c852d03f611b1e56a195d08648507018a7057fab82c4974c4eae105 /uv /uvx /bin/

WORKDIR /tests
COPY tests/pyproject.toml tests/uv.lock /tests/
COPY tests/lib /tests/lib
COPY tests/stand /tests/stand

# --frozen: install exactly what the committed lock says, and fail rather than
# re-resolving. --no-dev: ruff and mypy are not needed to run a journey.
# Browsers are already baked into the base image, so there is deliberately no
# `playwright install` step here or in any entrypoint.
ENV UV_LINK_MODE=copy \
    UV_PYTHON_INSTALL_DIR=/opt/uv-python \
    PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
RUN uv sync --frozen --no-dev

# Fail the BUILD, not a journey, if any of this is wrong: the interpreter must
# satisfy the suite's floor, the library must import on it, and the pip
# Playwright must match the browsers baked into the image — a mismatch there
# means the package looks for a browser build the image does not contain.
RUN uv run --frozen --no-dev python -c "\
import sys, insight_stand; \
assert sys.version_info >= (3, 13), f'interpreter {sys.version.split()[0]} is below the >=3.13 floor'; \
print('insight_stand imports on', sys.version.split()[0])" \
 && uv run --frozen --no-dev python -c "\
import subprocess; \
out = subprocess.run(['playwright', '--version'], capture_output=True, text=True).stdout.strip(); \
assert '1.61.0' in out, f'playwright pip pin != image: {out!r}'; \
print('playwright matches the image:', out)"

# Drop root before anything runs. A browser rendering pages from the stand is
# the least privileged thing in this repo and had the most privilege — and the
# root-owned `.artifacts/` a failed run left behind was the visible half of
# that: the invoking user could not delete its own trace files.
#
# `pwuser` is the base image's own non-root account, already owning
# /ms-playwright, so the browser gains nothing here. Only what the run WRITES
# changes hands: the venv the entrypoint executes from, and the artifact
# directory pytest reports into.
#
# HOME is set to a world-writable path on purpose. `dev-compose.sh` runs this
# image with `--user $(id -u):$(id -g)` so bind-mounted artifacts land owned by
# the INVOKING user rather than by whatever uid the image declares — which
# means the runtime uid is usually neither root nor pwuser, and has no home of
# its own. uv needs somewhere to put a cache even with `--frozen`.
ENV HOME=/tmp \
    XDG_CACHE_HOME=/tmp/.cache
RUN mkdir -p /tests/.artifacts \
 && chown -R pwuser:pwuser /tests /opt/uv-python \
 && chmod -R a+rX /tests /opt/uv-python \
 && chmod 1777 /tests/.artifacts
USER pwuser

# Headless Chromium only. Firefox and WebKit ship in the base image but are
# never launched by this suite.
ENTRYPOINT ["uv", "run", "--frozen", "--no-dev", "pytest"]
CMD ["/tests/stand", "--browser", "chromium"]
