FROM airbyte/python-connector-base:4.1.1@sha256:a268b44c733ae699a60f5fbc06a324945dba98945c6e2ab7f8609f2f895b0d28

# The base digest is pinned and ships no apt layer, so bookworm-security updates
# only reach the image if this stage applies them. Runs before the USER switch.
# Bump to re-run the upgrade when a fix lands without the base digest moving.
ARG SECURITY_EPOCH=2026-10-06
RUN echo "security epoch ${SECURITY_EPOCH}" \
    && apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

COPY . /airbyte/integration_code
RUN pip install --no-cache-dir /airbyte/integration_code

# airbyte-cdk pins nltk==3.9.1 (fixable CVE-2025-14009, CVE-2026-0846,
# CVE-2026-33231, CVE-2026-54293, CVE-2026-79675) but only uses it in the
# file-based unstructured parser, which this connector never loads; defusedxml is
# nltk 3.10's added runtime dependency. poetry/dulwich are base-image build
# tooling unused at runtime (CVE-2026-34591, CVE-2026-42305). cryptography
# arrives with the base image's keyring stack (SecretStorage); its wheel
# bundles OpenSSL, and 50.0.1 is the first built against 4.0.2 (CVE-2026-63072).
RUN pip install --no-cache-dir --no-deps nltk==3.10.3 defusedxml==0.7.1 cryptography==50.0.1 \
    && pip uninstall -y poetry dulwich

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="source-hubspot-insight"
ENTRYPOINT ["source-hubspot-insight"]
