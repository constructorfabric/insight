# Default pin must match the CDK bundled with the deployed Airbyte platform —
# see the BASE_IMAGE comment in source.sh. Bump together with AIRBYTE_VERSION.
ARG BASE_IMAGE=airbyte/source-declarative-manifest:6.60.9
FROM ${BASE_IMAGE}
ARG AIRBYTE_COMMAND=source-declarative-manifest
ENV AIRBYTE_COMMAND=${AIRBYTE_COMMAND}

COPY entrypoint.sh /usr/local/bin/airbyte_entrypoint.sh
ENTRYPOINT ["/bin/sh", "/usr/local/bin/airbyte_entrypoint.sh"]
