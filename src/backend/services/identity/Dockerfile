# Rebuild marker (2026-06-11, take 2): republish so the chart-pinned tag advances to
# an SLSA-attested image — current appVersion predates the provenance
# attestation steps in build-images.yml.
# Take 2: the first publish-chart attempt failed before committing the
# appVersion bumps back to main (chart attest step lacked registry login).
# Identity Resolution — .NET 9 service.
#
# Build context: src/backend/   (was services/identity/ historically — moved
# up so the Dockerfile can reach src/backend/docker-entrypoint.sh, keeping
# context uniform with the Rust services).
#
# Usage:
#   cd src/backend && docker build \
#       -f services/identity/Dockerfile \
#       -t insight-identity:dev .

FROM mcr.microsoft.com/dotnet/sdk:9.0 AS build
WORKDIR /src

# Restore layer — copy projects first so dependency resolution caches.
COPY services/identity/Directory.Build.props \
     services/identity/global.json \
     services/identity/Insight.Identity.sln \
     ./
COPY services/identity/src/Insight.Identity.Domain/Insight.Identity.Domain.csproj         src/Insight.Identity.Domain/
COPY services/identity/src/Insight.Identity.Infrastructure/Insight.Identity.Infrastructure.csproj src/Insight.Identity.Infrastructure/
COPY services/identity/src/Insight.Identity.Api/Insight.Identity.Api.csproj               src/Insight.Identity.Api/
RUN dotnet restore src/Insight.Identity.Api/Insight.Identity.Api.csproj

# Build + publish.
COPY services/identity/src/ src/
RUN dotnet publish src/Insight.Identity.Api/Insight.Identity.Api.csproj \
    -c Release -o /app/publish \
    --no-restore /p:UseAppHost=false

FROM mcr.microsoft.com/dotnet/aspnet:9.0 AS runtime

ARG TARGETARCH
ARG WATCHEXEC_VERSION=2.3.2

# Watchexec for ENABLE_AUTO_RELOAD-driven respawn (docker-compose dev only).
# See api-gateway/Dockerfile for the full rationale.
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in \
      amd64) WX_ARCH=x86_64-unknown-linux-musl ;; \
      arm64) WX_ARCH=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends curl xz-utils; \
    curl -fsSL "https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-${WX_ARCH}.tar.xz" \
        | tar -xJ -C /tmp; \
    install -m 0755 "/tmp/watchexec-${WATCHEXEC_VERSION}-${WX_ARCH}/watchexec" /usr/local/bin/watchexec; \
    rm -rf /tmp/watchexec-*; \
    apt-get purge -y curl xz-utils; \
    apt-get autoremove -y; \
    rm -rf /var/lib/apt/lists/*

COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

WORKDIR /app
COPY --from=build /app/publish ./
RUN useradd -U -u 1000 -m appuser \
    && chown -R 1000:1000 /app
USER 1000
EXPOSE 8082

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/app/Insight.Identity.Api.dll", "--", "dotnet", "/app/Insight.Identity.Api.dll"]
