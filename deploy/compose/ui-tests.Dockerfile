# INVARIANT: The base tag must match the playwright version in tests/uv.lock.
FROM mcr.microsoft.com/playwright/python:v1.64.1-noble@sha256:330d401ae23c905896a0c26c66c36fc971b621519e7b11b435a8328d7bf6e031

COPY --from=ghcr.io/astral-sh/uv:0.12.0@sha256:606e70c71c852d03f611b1e56a195d08648507018a7057fab82c4974c4eae105 /uv /uvx /bin/

WORKDIR /tests
COPY tests/pyproject.toml tests/uv.lock /tests/
COPY tests/lib /tests/lib
COPY tests/stand /tests/stand

ENV UV_LINK_MODE=copy \
    UV_PYTHON_INSTALL_DIR=/opt/uv-python \
    PLAYWRIGHT_BROWSERS_PATH=/ms-playwright \
    INSIGHT_STAND_ARTIFACT_DIR=/tests/.artifacts
RUN uv sync --locked --no-dev

ENV HOME=/tmp \
    XDG_CACHE_HOME=/tmp/.cache
RUN mkdir -p /tests/.artifacts /tests/test-results \
 && chown -R pwuser:pwuser /tests /opt/uv-python \
 && chmod -R a+rX /tests /opt/uv-python
USER pwuser

RUN uv run --frozen --no-dev python -c "\
import sys, insight_stand; \
assert sys.version_info >= (3, 13), f'interpreter {sys.version.split()[0]} is below the >=3.13 floor'; \
print('insight_stand imports on', sys.version.split()[0])" \
 && uv run --frozen --no-dev python -c "\
from playwright.sync_api import sync_playwright; \
p = sync_playwright().start(); \
b = p.chromium.launch(); \
print('chromium starts:', b.version); \
b.close(); p.stop()"

ENTRYPOINT ["uv", "run", "--frozen", "--no-dev", "pytest"]
CMD ["/tests/stand", "--browser", "chromium"]
