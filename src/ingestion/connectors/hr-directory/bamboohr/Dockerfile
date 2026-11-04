FROM airbyte/python-connector-base:4.1.1@sha256:a268b44c733ae699a60f5fbc06a324945dba98945c6e2ab7f8609f2f895b0d28 AS base

# The packages install on the build platform into a copy of the target's
# /usr/local, from target-arch wheels only, so no step here runs under QEMU and
# the result matches an in-place install on the target.
FROM --platform=$BUILDPLATFORM airbyte/python-connector-base:4.1.1@sha256:a268b44c733ae699a60f5fbc06a324945dba98945c6e2ab7f8609f2f895b0d28 AS deps
COPY --from=ghcr.io/astral-sh/uv:0.12.0@sha256:606e70c71c852d03f611b1e56a195d08648507018a7057fab82c4974c4eae105 /uv /bin/
COPY --from=base /usr/local /prefix
COPY . /src
ARG TARGETARCH
ENV UV_LINK_MODE=copy \
    UV_NO_CACHE=1 \
    UV_NO_CONFIG=1

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
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) platform=x86_64-manylinux_2_36 ;; \
      arm64) platform=aarch64-manylinux_2_36 ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    uv build --wheel --out-dir /wheels /src; \
    set -- --prefix /prefix --python /usr/local/bin/python3 --python-platform "${platform}" \
      --only-binary :all: --compile-bytecode; \
    uv pip install "$@" --upgrade /wheels/*.whl; \
    uv pip install "$@" --no-deps nltk==3.10.4 defusedxml==0.7.1 cryptography==50.0.1 \
      urllib3==2.8.0; \
    uv pip uninstall --prefix /prefix --python /usr/local/bin/python3 poetry dulwich \
      poetry-plugin-export cachecontrol virtualenv filelock msgpack

FROM base

# The base digest is pinned and ships no apt layer, so bookworm-security updates
# only reach the image if this stage applies them. Runs before the USER switch.
# Bump to re-run the upgrade when a fix lands without the base digest moving.
ARG SECURITY_EPOCH=2026-11-02
RUN echo "security epoch ${SECURITY_EPOCH}" \
    && apt-get update \
    && apt-get upgrade -y --no-install-recommends \
    && rm -rf /var/lib/apt/lists/*

COPY . /airbyte/integration_code
# The uninstalls above remove files a COPY over these directories would keep.
RUN rm -rf /usr/local/lib/python3.13/site-packages /usr/local/bin
COPY --from=deps /prefix/lib/python3.13/site-packages /usr/local/lib/python3.13/site-packages
COPY --from=deps /prefix/bin /usr/local/bin

RUN useradd --create-home --no-log-init --shell /bin/bash airbyte_user
USER airbyte_user

ENV AIRBYTE_ENTRYPOINT="source-bamboohr"
ENTRYPOINT ["source-bamboohr"]
