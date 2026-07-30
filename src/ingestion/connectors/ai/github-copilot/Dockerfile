FROM airbyte/python-connector-base:4.1.1

COPY . /airbyte/integration_code
RUN pip install /airbyte/integration_code

# airbyte-cdk pins nltk==3.9.1 (fixable CVE-2025-14009, CVE-2026-0846,
# CVE-2026-33231, CVE-2026-54293) but only uses it in the file-based
# unstructured parser, which this connector never loads; defusedxml is
# nltk 3.10's added runtime dependency. poetry/dulwich are base-image build
# tooling unused at runtime (CVE-2026-34591, CVE-2026-42305).
RUN pip install --no-cache-dir --no-deps nltk==3.10.0 defusedxml==0.7.1 \
    && pip uninstall -y poetry dulwich

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="source-github-copilot"
ENTRYPOINT ["source-github-copilot"]