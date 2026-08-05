ARG BASE_IMAGE=airbyte/source-declarative-manifest:latest
FROM ${BASE_IMAGE}
ARG AIRBYTE_COMMAND=source-declarative-manifest
ENV AIRBYTE_COMMAND=${AIRBYTE_COMMAND}

COPY entrypoint.sh /usr/local/bin/airbyte_entrypoint.sh

# Every mount this image is given is read-only and its output goes to stdout,
# so it needs no writable path of its own.
USER 1000

ENTRYPOINT ["/bin/sh", "/usr/local/bin/airbyte_entrypoint.sh"]
