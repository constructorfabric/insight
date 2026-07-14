# One-shot image for the sample-data seed script.
#
# Re-built only when requirements.txt changes — the script itself is
# bind-mounted at runtime so iterating on seed.py doesn't trigger an
# image rebuild.

# Pinned to 3.12 to match src/ingestion/tools/toolbox/Dockerfile (the
# deploy dbt-run image) — apply-ch-migrations.sh runs dbt here, and
# dbt-clickhouse is validated against 3.12 on that image.
FROM python:3.12-slim

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

COPY requirements.txt .
RUN pip install -r requirements.txt

# Source is bind-mounted at runtime (see docker-compose.yml seed-sample
# service). The COPY lines below are a fallback for `docker build &&
# docker run` without a volume — exercised only by ad-hoc inspection,
# so the `generators/` package must come along or `silver.py`'s
# `from generators import ...` fails at import time.
COPY *.py ./
COPY generators ./generators/

ENTRYPOINT ["python", "seed.py"]
CMD ["all"]
