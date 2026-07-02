using System.Reflection;
using System.Text;
using System.Text.Encodings.Web;
using System.Text.Json;
using FluentAssertions;
using Xunit;

namespace Insight.Identity.Tests.Integration;

/// <summary>
/// OpenAPI contract drift gate — the .NET-native counterpart of analytics-api's
/// <c>scripts/ci/openapi_spec.py check</c>. Boots the API against the
/// Testcontainers MariaDB, fetches the live <c>GET /openapi.json</c> the service
/// now serves (see <c>Program.cs</c> AddOpenApi/MapOpenApi), and asserts it
/// matches the committed contract at
/// <c>docs/components/backend/identity/openapi.json</c>. A route or schema change
/// that isn't reflected in the committed doc fails this test (and the CI .NET
/// job that runs it).
///
/// Regenerate after an intentional change:
/// <code>
///     IDENTITY_OPENAPI_UPDATE=1 dotnet test \
///         --filter FullyQualifiedName~OpenApiContractTests
/// </code>
/// then commit the updated <c>docs/components/backend/identity/openapi.json</c>.
/// </summary>
[Collection(MariaDbCollection.Name)]
public sealed class OpenApiContractTests
{
    private const string SpecRoute = "/openapi.json";
    private const string UpdateEnvVar = "IDENTITY_OPENAPI_UPDATE";
    private static readonly Guid TenantId = Guid.Parse("11111111-1111-1111-1111-111111111111");

    private readonly MariaDbFixture _fixture;

    public OpenApiContractTests(MariaDbFixture fixture) => _fixture = fixture;

    [Fact]
    public async Task Live_openapi_json_matches_committed_doc()
    {
        using var app = new TestApplicationFactory(_fixture.ConnectionString, TenantId);
        using var client = app.CreateClient();

        var response = await client.GetAsync(new Uri(SpecRoute, UriKind.Relative)).ConfigureAwait(false);
        response.IsSuccessStatusCode.Should()
            .BeTrue($"GET {SpecRoute} should serve the OpenAPI document (status {(int)response.StatusCode})");
        var liveJson = await response.Content.ReadAsStringAsync().ConfigureAwait(false);
        var live = Canonical(liveJson);

        var specPath = LocateCommittedSpec();

        if (!string.IsNullOrEmpty(Environment.GetEnvironmentVariable(UpdateEnvVar)))
        {
            Directory.CreateDirectory(Path.GetDirectoryName(specPath)!);
            await File.WriteAllTextAsync(specPath, live).ConfigureAwait(false);
            return; // regenerated on request — nothing to assert
        }

        File.Exists(specPath).Should().BeTrue(
            $"the committed OpenAPI doc must exist at {specPath} — generate it with "
            + $"`{UpdateEnvVar}=1 dotnet test --filter FullyQualifiedName~OpenApiContractTests`");

        var committed = Canonical(await File.ReadAllTextAsync(specPath).ConfigureAwait(false));

        live.Should().Be(committed,
            $"the live {SpecRoute} drifted from the committed contract — regenerate with "
            + $"`{UpdateEnvVar}=1 dotnet test --filter FullyQualifiedName~OpenApiContractTests` "
            + "and commit docs/components/backend/identity/openapi.json");
    }

    /// <summary>
    /// Canonical, order-independent JSON form: object keys sorted, 2-space
    /// indent, relaxed (non-ASCII-escaping) encoder, trailing newline. Mirrors
    /// the intent of <c>openapi_spec.py.normalize</c> so the committed doc is a
    /// minimal, review-friendly diff and the comparison ignores the emission
    /// order of the in-process OpenAPI generator.
    /// </summary>
    private static string Canonical(string json)
    {
        using var doc = JsonDocument.Parse(json);
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(
            stream,
            new JsonWriterOptions { Indented = true, Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping }))
        {
            WriteCanonical(writer, doc.RootElement);
        }

        return Encoding.UTF8.GetString(stream.ToArray()) + "\n";
    }

    private static void WriteCanonical(Utf8JsonWriter writer, JsonElement element)
    {
        switch (element.ValueKind)
        {
            case JsonValueKind.Object:
                writer.WriteStartObject();
                foreach (var property in element.EnumerateObject().OrderBy(p => p.Name, StringComparer.Ordinal))
                {
                    writer.WritePropertyName(property.Name);
                    WriteCanonical(writer, property.Value);
                }

                writer.WriteEndObject();
                break;
            case JsonValueKind.Array:
                writer.WriteStartArray();
                foreach (var item in element.EnumerateArray())
                {
                    WriteCanonical(writer, item);
                }

                writer.WriteEndArray();
                break;
            default:
                element.WriteTo(writer);
                break;
        }
    }

    /// <summary>
    /// The committed <c>docs/components/backend/identity/openapi.json</c>. Its
    /// absolute path is resolved at build time from the test project's location
    /// and injected as <c>CommittedOpenApiSpec</c> assembly metadata (see the
    /// <c>.csproj</c>) — no runtime filesystem walk for a docs/ marker.
    /// </summary>
    private static string LocateCommittedSpec()
    {
        var path = typeof(OpenApiContractTests).Assembly
            .GetCustomAttributes<AssemblyMetadataAttribute>()
            .FirstOrDefault(a => a.Key == "CommittedOpenApiSpec")?.Value;

        if (string.IsNullOrEmpty(path))
        {
            throw new InvalidOperationException(
                "CommittedOpenApiSpec assembly metadata is missing — it is injected at "
                + "build time by Insight.Identity.Tests.Integration.csproj.");
        }

        return path;
    }
}
