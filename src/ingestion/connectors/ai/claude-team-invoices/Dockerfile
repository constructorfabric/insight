FROM airbyte/python-connector-base:4.1.1@sha256:a268b44c733ae699a60f5fbc06a324945dba98945c6e2ab7f8609f2f895b0d28

# The base digest is pinned and ships no apt layer, so bookworm-security updates
# only reach the image if this stage applies them. Runs before the USER switch.
# Bump to re-run the upgrade when a fix lands without the base digest moving.
ARG SECURITY_EPOCH=2026-11-02
RUN echo "security epoch ${SECURITY_EPOCH}" \
    && apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

COPY . /airbyte/integration_code
RUN pip install /airbyte/integration_code

# airbyte-cdk 7.48 pins nltk==3.9.4, which OSV still flags (e.g. CVE-2026-54293),
# but imports it only in the file-based unstructured parser, which this connector
# never loads; 3.10.4 has no advisories, and defusedxml is nltk 3.10's added
# runtime dependency. poetry 1.8.4 and dulwich 0.21.7 are base-image build tooling
# unused at runtime (CVE-2026-34591, CVE-2026-42305); so are the packages only poetry
# needs: virtualenv (CVE-2026-113718 and 10 more), filelock (CVE-2026-112940; the
# CDK's pyrate-limiter wants it only as an extra) and msgpack (CVE-2026-114613),
# with CacheControl and poetry-plugin-export that hold them. The base image's keyring stack
# brings cryptography 49.0.0 (CVE-2026-69247, fixed in 50.0.0); its wheel bundles
# OpenSSL, and 50.0.1 is the first built against 4.0.2 (CVE-2026-63072).
# pip keeps the base image's urllib3 2.7.0 (CVE-2026-97687, CVE-2026-97689)
# because it already satisfies requests' range, so it is pinned here.
# certifi needs no pin: airbyte-cdk requires >=2026.10.23, above the base's
# 2026.7.22, so the install above already upgrades it.
RUN pip install --no-cache-dir --no-deps nltk==3.10.4 defusedxml==0.7.1 cryptography==50.0.1 \
    urllib3==2.8.0 \
    && pip uninstall -y poetry dulwich poetry-plugin-export cachecontrol virtualenv filelock msgpack

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="source-claude-team-invoices"
ENTRYPOINT ["source-claude-team-invoices"]
