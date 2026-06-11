# Rebuild marker (2026-06-11): republish so the chart-pinned tag advances to
# an SLSA-attested image — current appVersion predates the provenance
# attestation steps in build-images.yml.
# Identity Resolution — .NET 9 service.
#
# Build context: src/backend/services/identity/
# Usage: docker build -t insight-identity:dev src/backend/services/identity

FROM mcr.microsoft.com/dotnet/sdk:9.0 AS build
WORKDIR /src

# Restore layer — copy projects first so dependency resolution caches.
COPY Directory.Build.props global.json Insight.Identity.sln ./
COPY src/Insight.Identity.Domain/Insight.Identity.Domain.csproj src/Insight.Identity.Domain/
COPY src/Insight.Identity.Infrastructure/Insight.Identity.Infrastructure.csproj src/Insight.Identity.Infrastructure/
COPY src/Insight.Identity.Api/Insight.Identity.Api.csproj src/Insight.Identity.Api/
RUN dotnet restore src/Insight.Identity.Api/Insight.Identity.Api.csproj

# Build + publish.
COPY src/ src/
RUN dotnet publish src/Insight.Identity.Api/Insight.Identity.Api.csproj \
    -c Release -o /app/publish \
    --no-restore /p:UseAppHost=false

FROM mcr.microsoft.com/dotnet/aspnet:9.0 AS runtime
WORKDIR /app
COPY --from=build /app/publish ./
RUN useradd -U -u 1000 appuser \
    && chown -R 1000:1000 /app
USER 1000
EXPOSE 8082
ENTRYPOINT ["dotnet", "Insight.Identity.Api.dll"]