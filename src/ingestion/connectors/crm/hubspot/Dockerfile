FROM airbyte/python-connector-base:2.0.0

COPY . /airbyte/integration_code
RUN pip install --no-cache-dir /airbyte/integration_code

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="source-hubspot-insight"
ENTRYPOINT ["source-hubspot-insight"]
