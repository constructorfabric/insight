FROM python:3.12-slim

# System tools
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl gettext-base jq gnupg lsb-release && \
    rm -rf /var/lib/apt/lists/*

# Node.js (for JWT token generation). npm is upgraded in place: the one the
# distro package carries lags its own release line, and its bundled dependency
# tree is what the image scan reports.
RUN curl -fsSL https://deb.nodesource.com/setup_24.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    npm install -g npm@latest && \
    npm cache clean --force && \
    rm -rf /var/lib/apt/lists/*

# yq (YAML processor)
RUN curl -fsSL "https://github.com/mikefarah/yq/releases/latest/download/yq_linux_$(dpkg --print-architecture)" \
    -o /usr/local/bin/yq && chmod +x /usr/local/bin/yq

# kubectl
RUN curl -fsSL "https://dl.k8s.io/release/$(curl -fsSL https://dl.k8s.io/release/stable.txt)/bin/linux/$(dpkg --print-architecture)/kubectl" \
    -o /usr/local/bin/kubectl && chmod +x /usr/local/bin/kubectl

# dbt-clickhouse
RUN pip install --no-cache-dir dbt-clickhouse

# Copy entire ingestion project
COPY . /ingestion
WORKDIR /ingestion

# dbt writes logs/ and target/ under the project root, so the tree must belong
# to the runtime user.
RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /ingestion
USER 1000
