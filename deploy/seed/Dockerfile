# One-shot image for the sample-data seed script.
#
# Installs deps + the seeder from pyproject.toml (single source of truth).
# The script is bind-mounted at runtime (see docker-compose.yml seed-sample)
# so iterating on seed.py doesn't require a rebuild; rebuild when
# pyproject.toml or the packaged sources change.

# Python 3.13 to match this seeder's pyproject (ruff target-version /
# mypy python_version = 3.13).
FROM python:3.13-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1

WORKDIR /app

# curl: needed by src/ingestion/scripts/{create-bronze-placeholders,
# apply-ch-migrations}.sh (bind-mounted at /ingestion), which silver.py runs
# to create the placeholder tables + build the gold layer — the same scripts
# the k8s clickhouse-migrate Hook Job runs.
# bash: the scripts use bashisms (process substitution, [[ ]]); slim ships dash.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl bash \
    && rm -rf /var/lib/apt/lists/*

# Source is copied before the install so the build backend can package the
# flat modules + generators/. At runtime the source is bind-mounted (see
# docker-compose.yml seed-sample service); these COPY lines are also the
# fallback for `docker build && docker run` without a volume — exercised only
# by ad-hoc inspection, so the `generators/` package must come along or
# `silver.py`'s `from generators import ...` fails at import time.
COPY pyproject.toml .
COPY *.py ./
COPY generators ./generators/
RUN pip install .

ENTRYPOINT ["python", "seed.py"]
CMD ["all"]
