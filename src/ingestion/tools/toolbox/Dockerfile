FROM python:3.12-slim@sha256:aa943ce411423c7582288f18707ff66eddcbaeb510502d747a72aeb34f30d159

# System tools
RUN apt-get update && apt-get upgrade -y --no-install-recommends && \
    apt-get install -y --no-install-recommends \
    curl gettext-base jq gnupg lsb-release && \
    rm -rf /var/lib/apt/lists/*

# Node.js (for JWT token generation). Nothing in this image installs a package,
# and npm's own bundled dependency tree is what the image scan reports against.
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    npm cache clean --force && \
    rm -rf /usr/lib/node_modules/npm /usr/bin/npm /usr/bin/npx && \
    rm -rf /var/lib/apt/lists/*

# yq (YAML processor)
RUN curl -fsSL "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_$(dpkg --print-architecture)" \
    -o /usr/local/bin/yq && chmod +x /usr/local/bin/yq

# kubectl
RUN curl -fsSL "https://dl.k8s.io/release/$(curl -fsSL https://dl.k8s.io/release/stable.txt)/bin/linux/$(dpkg --print-architecture)/kubectl" \
    -o /usr/local/bin/kubectl && chmod +x /usr/local/bin/kubectl

# dbt at the pin every dbt step reads (scripts/bootstrap-db/pins.env), so the
# adapter the cluster rig proves is the one the migrate hook runs. certifi is
# pinned beside it: this layer is served from the registry build cache until
# the pins or the line change, so a certifi advisory (GHSA-6q48-6c7w-9gw2)
# only reaches the image through an explicit pin.
COPY scripts/bootstrap-db/pins.env /tmp/pins.env
RUN . /tmp/pins.env && \
    pip install --no-cache-dir "dbt-clickhouse==${DBT_CLICKHOUSE_VERSION}" "dbt-core==${DBT_CORE_VERSION}" "certifi==2026.10.19" && \
    rm /tmp/pins.env

# Copy the ingestion project, minus the sample-data seeder: this image runs
# migrations against real stands, and a demo-data generator does not belong on
# one. The exclusion lives in Dockerfile.dockerignore beside this file, so it
# applies to THIS image only — the seeder's own image (tools/seed/Dockerfile)
# builds from the same context and does carry it.
COPY . /ingestion
WORKDIR /ingestion

# dbt writes logs/ and target/ under the project root, so the tree must belong
# to the runtime user.
RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /ingestion
USER 1000
