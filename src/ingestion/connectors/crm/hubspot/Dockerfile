# Rebuild marker (2026-06-09): GHCR packages did not follow the cyberfabric ->
# constructorfabric org rename; this change fires a CI rebuild so the image is
# republished (and attested) under ghcr.io/constructorfabric.
FROM airbyte/python-connector-base:2.0.0

COPY . /airbyte/integration_code
RUN pip install --no-cache-dir /airbyte/integration_code

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="source-hubspot-insight"
ENTRYPOINT ["source-hubspot-insight"]