ARG BASE_IMAGE=airbyte/destination-postgres:latest
FROM ${BASE_IMAGE}
ARG AIRBYTE_COMMAND=/airbyte/bin/destination-postgres
ENV AIRBYTE_COMMAND=${AIRBYTE_COMMAND}

COPY entrypoint.sh /usr/local/bin/airbyte_entrypoint.sh
ENTRYPOINT ["/bin/sh", "/usr/local/bin/airbyte_entrypoint.sh"]
