ARG BASE_IMAGE=airbyte/source-declarative-manifest:latest
FROM ${BASE_IMAGE}
ARG AIRBYTE_COMMAND=source-declarative-manifest
ENV AIRBYTE_COMMAND=${AIRBYTE_COMMAND}

COPY entrypoint.sh /usr/local/bin/airbyte_entrypoint.sh
ENTRYPOINT ["/bin/sh", "/usr/local/bin/airbyte_entrypoint.sh"]
