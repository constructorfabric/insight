# The sample-data seeder's own image — the only one that carries it.
#
# Deliberately NOT the toolbox: that image runs migrations on real stands, and
# a demo-data generator has no business being installed there. This one carries
# the seeder plus exactly what its silver step shells out to — the ingestion
# tree's DDL and gold-build scripts and the dbt project they run — and none of
# the toolbox's operator tooling (node, kubectl, yq).
#
# Build context is the INGESTION TREE, not this directory:
#
#   docker build -f src/ingestion/tools/seed/Dockerfile src/ingestion
#
# Both callers use it: compose builds it locally (see docker-compose.yml
# seed-sample), and CI publishes it as `insight-seed` for `seed-stand.sh` to
# run as a Job.

# 3.12 to match the package's floor and the rest of the ingestion tree; dbt's
# dependency chain does not import on 3.14 yet.
FROM python:3.12-slim

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    PIP_NO_CACHE_DIR=1 \
    HOME=/tmp

# curl + bash: scripts/create-bronze-placeholders.sh and
# scripts/apply-ch-migrations.sh use both, and slim ships dash and no curl.
RUN apt-get update \
    && apt-get install -y --no-install-recommends curl bash \
    && rm -rf /var/lib/apt/lists/*

# The slow layer, before any source is copied so it survives every edit below.
# dbt is the silver step's environment rather than an import of the package,
# which is why the package declares it as an extra and it is installed here.
RUN pip install --no-cache-dir dbt-clickhouse

WORKDIR /ingestion

# Only what the seed actually runs. `dbt_project.yml` reads its models from
# ../silver, ../gold and ../connectors, so those come along; the connector
# runtimes, reconcile tooling, tests and secrets do not.
COPY scripts ./scripts/
COPY dbt ./dbt/
COPY silver ./silver/
COPY gold ./gold/
COPY connectors ./connectors/
COPY tools/seed ./tools/seed/

# Editable, so the compose service can mount the working tree over /ingestion
# and have its edits take effect without a rebuild. The console script it
# installs (`insight-seed`) is the entry point for both callers.
RUN pip install --no-cache-dir -e ./tools/seed

# dbt writes target/ and logs/ under the project root and the seeder writes its
# manifest beside the package, so the tree belongs to the runtime user.
RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /ingestion
USER 1000

# The manifest lands in the working directory (see config.parse_manifest_path),
# which is the seeder's own directory — the path compose bind-mounts and the
# stand test suite reads.
WORKDIR /ingestion/tools/seed

ENTRYPOINT ["insight-seed"]
CMD ["all"]
