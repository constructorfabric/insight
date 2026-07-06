using System.Text.Json;
using FluentAssertions;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// Serves the OpenAPI document and, for the external drift gate, dumps it.
///
/// The drift COMPARISON lives OUTSIDE this project — a CI step runs
/// <c>scripts/ci/openapi_spec.py check</c> to diff the dumped spec against the
/// committed contract at <c>docs/components/backend/identity/openapi.json</c>.
/// This mirrors the analytics split: the component only emits its spec; the
/// orchestration owns the committed-doc comparison. So this test knows nothing
/// about the repo layout or the docs/ location — it boots the API against the
/// Testcontainers MariaDB, checks <c>GET /openapi.json</c> serves a well-formed
/// OpenAPI document, and writes it verbatim to the sink path in
/// <c>IDENTITY_OPENAPI_DUMP</c> when set (CI provides it; unset locally = no
/// dump, the structural checks still run). Exact content — title, routes,
/// schemas — is the external gate's job, so drift surfaces there as a diff.
///
/// Regenerate the committed contract after an intentional route/schema change:
/// <code>
///     IDENTITY_OPENAPI_DUMP=/tmp/identity.openapi.json \
///         dotnet test --filter FullyQualifiedName~OpenApiContractTests
///     python3 scripts/ci/openapi_spec.py update \
///         --file docs/components/backend/identity/openapi.json \
///         --live-file /tmp/identity.openapi.json
/// </code>
/// then commit the updated doc.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class OpenApiContractTests
{
    private const string SpecRoute = "/openapi.json";
    private const string DumpEnvVar = "IDENTITY_OPENAPI_DUMP";
    private static readonly Guid TenantId = Guid.Parse("11111111-1111-1111-1111-111111111111");

    private readonly MariaDbFixture _fixture;

    public OpenApiContractTests(MariaDbFixture fixture) => _fixture = fixture;

    [Fact]
    public async Task Serves_openapi_document_and_dumps_it_for_the_drift_gate()
    {
        using var app = new TestApplicationFactory(_fixture.ConnectionString, TenantId);
        using var client = app.CreateClient();

        var response = await client.GetAsync(new Uri(SpecRoute, UriKind.Relative)).ConfigureAwait(false);
        response.IsSuccessStatusCode.Should()
            .BeTrue($"GET {SpecRoute} should serve the OpenAPI document (status {(int)response.StatusCode})");

        var liveJson = await response.Content.ReadAsStringAsync().ConfigureAwait(false);

        // Structural sanity only — enough to trust the dump is a real OpenAPI
        // document (not an error page or an empty body). The EXACT content
        // (title, version, paths, schemas) is deliberately NOT asserted here:
        // comparing it against the committed contract is the external drift
        // gate's job (scripts/ci/openapi_spec.py), so a real change surfaces
        // there as a readable committed-vs-generated diff — not as a brittle
        // in-test assertion that masks the diff.
        using (var doc = JsonDocument.Parse(liveJson))
        {
            doc.RootElement.TryGetProperty("openapi", out _).Should()
                .BeTrue("the response must be an OpenAPI document");
            doc.RootElement.TryGetProperty("paths", out var paths).Should()
                .BeTrue("the contract must expose paths");
            paths.EnumerateObject().Should().NotBeEmpty("the contract must expose routes");
        }

        // Hand the served spec to the external drift gate when a sink is set (CI
        // sets IDENTITY_OPENAPI_DUMP; scripts/ci/openapi_spec.py normalizes and
        // diffs it against the committed doc). No docs/ path is known here.
        var dumpPath = Environment.GetEnvironmentVariable(DumpEnvVar);
        if (!string.IsNullOrEmpty(dumpPath))
        {
            var dir = Path.GetDirectoryName(dumpPath);
            if (!string.IsNullOrEmpty(dir))
            {
                Directory.CreateDirectory(dir);
            }

            await File.WriteAllTextAsync(dumpPath, liveJson).ConfigureAwait(false);
        }
    }
}
