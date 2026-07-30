using Insight.Identity.Api.Auth;
using Insight.Identity.Api.Configuration;
using Insight.Identity.Api.Contracts;
using Insight.Identity.Api.Endpoints;
using Insight.Identity.Domain.Services;
using Insight.Identity.Infrastructure;
using Insight.Identity.Infrastructure.MariaDb;
using Microsoft.AspNetCore.Authentication.JwtBearer;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Diagnostics;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using Microsoft.Extensions.Configuration;
using FluentValidation;
using Microsoft.Extensions.DependencyInjection;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.Logging;
using Microsoft.IdentityModel.Tokens;
using MySqlConnector;
using Serilog;
using Serilog.Formatting.Compact;

var builder = WebApplication.CreateBuilder(args);

// Mirror the Rust service's snake_case env-var layout (IDENTITY__bind_addr,
// IDENTITY__database_url, IDENTITY__mariadb__url, ...). The double underscore
// becomes the configuration section delimiter.
builder.Configuration
    .AddYamlFile("appsettings.yaml", optional: true, reloadOnChange: false)
    .AddEnvironmentVariables(prefix: "IDENTITY__");

builder.Host.UseSerilog((context, services, config) =>
{
    config
        .ReadFrom.Configuration(context.Configuration)
        .Enrich.FromLogContext()
        .Enrich.WithProperty("service", "identity")
        // RenderedCompactJsonFormatter emits the `@m` field with all
        // placeholders substituted (e.g. "HTTP GET /healthz responded
        // 200 in 0.2 ms"), in addition to the structured properties
        // (RequestMethod, RequestPath, …) and the source template via
        // `@mt`. The bare CompactJsonFormatter omits `@m`, leaving log
        // viewers that fall back to `@mt` showing placeholders
        // (`HTTP {RequestMethod} {RequestPath} …`) instead of values.
        .WriteTo.Console(new RenderedCompactJsonFormatter());
});

builder.Services
    .AddOptions<AppOptions>()
    .Bind(builder.Configuration.GetSection(AppOptions.SectionName))
    .ValidateDataAnnotations()
    .ValidateOnStart();

builder.Services
    .AddOptions<MariaDbOptions>()
    .Bind(builder.Configuration.GetSection(MariaDbOptions.SectionName))
    .ValidateDataAnnotations()
    .ValidateOnStart();

builder.Services.AddSingleton<MariaDbConnectionFactory>();
builder.Services.AddSingleton<PersonsRepository>();
builder.Services.AddSingleton<IPersonsReader>(sp => sp.GetRequiredService<PersonsRepository>());
builder.Services.AddSingleton<PersonLookupService>();
builder.Services.AddSingleton<ProfileLookupService>();

// #346 step 1: read-only access to the visibility / roles / person_roles
// tables. The services that use these ports (VisibilityService, the
// admin-role authz filter, CRUD endpoints) land in later steps; the
// readers exist now so the migrations stay paired with their consumers.
builder.Services.AddSingleton<VisibilityRepository>();
builder.Services.AddSingleton<IVisibilityReader>(sp => sp.GetRequiredService<VisibilityRepository>());
builder.Services.AddSingleton<RolesRepository>();
builder.Services.AddSingleton<IRolesReader>(sp => sp.GetRequiredService<RolesRepository>());
builder.Services.AddSingleton<IPersonRolesReader>(sp => sp.GetRequiredService<RolesRepository>());
builder.Services.AddSingleton<VisibilityService>();

// #348 Phase 3: depth-bounded subchart endpoint.
builder.Services.AddSingleton<SubchartRepository>();
builder.Services.AddSingleton<ISubchartReader>(sp => sp.GetRequiredService<SubchartRepository>());
builder.Services.AddSingleton<SubchartService>();

// persons-seed: admin-triggered bulk re-seed from ClickHouse
// identity_inputs. ClickHouse client + the generic operations audit
// store + the seed orchestrator + the background drainer.
builder.Services
    .AddOptions<Insight.Identity.Infrastructure.ClickHouse.ClickHouseOptions>()
    .Bind(builder.Configuration.GetSection(Insight.Identity.Infrastructure.ClickHouse.ClickHouseOptions.SectionName));
builder.Services.AddSingleton<Insight.Identity.Infrastructure.ClickHouse.ClickHouseConnectionFactory>();
builder.Services.AddSingleton<Insight.Identity.Infrastructure.ClickHouse.ClickHouseIdentityInputsReader>();
builder.Services.AddSingleton<IIdentityInputsReader>(sp =>
    sp.GetRequiredService<Insight.Identity.Infrastructure.ClickHouse.ClickHouseIdentityInputsReader>());
builder.Services.AddSingleton<OperationsRepository>();
builder.Services.AddSingleton<IOperationsRepository>(sp => sp.GetRequiredService<OperationsRepository>());
builder.Services.AddSingleton<PersonsSeedRepository>();
builder.Services.AddSingleton<IPersonsSeedStore>(sp => sp.GetRequiredService<PersonsSeedRepository>());
builder.Services.AddSingleton<PersonsSeedService>();
builder.Services.AddSingleton<Insight.Identity.Api.Background.PersonsSeedQueue>();
builder.Services.AddHostedService<Insight.Identity.Api.Background.PersonsSeedWorker>();

// FluentValidation — Phase 2 POST /v1/profiles body. Scans the Api
// assembly for AbstractValidator<T> implementations.
builder.Services.AddValidatorsFromAssemblyContaining<Insight.Identity.Api.Validation.ResolveProfileCommandValidator>();

// Tenant resolver (NGINX_BFF §10 G2): the ONLY authority is the single signed
// gateway-JWT `tenant_id` claim. No config-default fallback — a request without a
// signed tenant resolves to null and fails closed (400 tenant_unresolved) rather
// than silently reading another tenant's data.
builder.Services.AddSingleton<ITenantContext, GatewayTenantContext>();

// Gateway-JWT bearer authentication — full, fail-closed validation
// (NGINX_BFF §6 R1). identity verifies the gateway JWT itself: signature
// (ES256, via the authenticator's JWKS), issuer (the gateway origin), audience
// (internal-services), and lifetime. Raw customer-IdP tokens are no longer
// accepted. There is no parse-only / disable path.
//
// The authenticator publishes JWKS at `/.well-known/jwks.json` but serves no
// OIDC discovery document, so `Authority` metadata discovery cannot be used;
// the signing keys are resolved directly from the configured JWKS URL with a
// caching ConfigurationManager (periodic refresh + auto-refresh on unknown
// kid, which covers key rotation).
var gatewayIssuer = builder.Configuration["identity:auth_gateway_issuer"] ?? "";
var gatewayJwksUrl = builder.Configuration["identity:auth_gateway_jwks_url"] ?? "";
if (string.IsNullOrWhiteSpace(gatewayIssuer) || string.IsNullOrWhiteSpace(gatewayJwksUrl))
{
    throw new InvalidOperationException(
        "identity:auth_gateway_issuer and identity:auth_gateway_jwks_url are required " +
        "(env IDENTITY__auth_gateway_issuer / IDENTITY__auth_gateway_jwks_url). " +
        "The gateway JWT is verified fail-closed — there is no disable knob.");
}

// In-cluster JWKS is plain HTTP (TLS terminates at the ingress); do not require
// HTTPS on the document fetch.
var jwksConfigManager = new Microsoft.IdentityModel.Protocols.ConfigurationManager<JsonWebKeySet>(
    gatewayJwksUrl,
    new JwksRetriever(),
    new Microsoft.IdentityModel.Protocols.HttpDocumentRetriever { RequireHttps = false });

builder.Services
    .AddAuthentication(JwtBearerDefaults.AuthenticationScheme)
    .AddJwtBearer(options =>
    {
        options.RequireHttpsMetadata = false;
        // Keep JWT claim names as-is (`sub`, `tenants`, `roles`, …) so resolvers
        // read them by their short names rather than the long ClaimTypes.* URIs
        // the default pipeline would rewrite them to.
        options.MapInboundClaims = false;
        options.TokenValidationParameters = new TokenValidationParameters
        {
            ValidateIssuer = true,
            ValidIssuer = gatewayIssuer,
            ValidateAudience = true,
            ValidAudience = "internal-services",
            ValidateLifetime = true,
            ValidateIssuerSigningKey = true,
            RequireSignedTokens = true,
            // Pin the algorithm to the authenticator's ES256 (ECDSA P-256) —
            // reject `alg=none` and RSA/HS confusion outright.
            ValidAlgorithms = new[] { SecurityAlgorithms.EcdsaSha256 },
            IssuerSigningKeyResolver = (_, _, _, _) =>
                jwksConfigManager.GetConfigurationAsync(CancellationToken.None)
                    .GetAwaiter().GetResult().GetSigningKeys(),
        };
    });

// Fail-closed by default (NGINX_BFF R1): every endpoint requires a valid
// gateway JWT unless it opts out with AllowAnonymous (health probes, the
// OpenAPI document). A request without a valid gateway JWT gets 401.
builder.Services.AddAuthorization(options =>
{
    options.FallbackPolicy = new Microsoft.AspNetCore.Authorization.AuthorizationPolicyBuilder()
        .RequireAuthenticatedUser()
        .Build();
});

// Caller resolver — the gateway JWT's `sub` IS the internal person_id
// (NGINX_BFF step 07). No DB lookup, no header trust path.
builder.Services.AddScoped<ICallerContext, SubjectCallerContext>();

// Admin-probe — used by CRUD endpoints on /v1/visibility, /v1/roles,
// /v1/person-roles to gate by the `admin` role. Scoped to match the
// scoped ICallerContext above (a singleton holding a scoped resolver
// captures the first-request scope for every later request).
builder.Services.AddScoped<CallerAdminCheck>();

// JSON wire convention: snake_case on every Minimal-API surface (request
// body + response body). Lets DTOs in `Contracts/` declare plain PascalCase
// properties and rely on the policy for serialisation — no per-property
// `[JsonPropertyName]` attributes. Test clients deliberately use the same
// policy via `JsonExtensions.PostJsonAsync` / `ReadJsonAsync` so wire-format
// drift between server and tests is impossible.
builder.Services.Configure<Microsoft.AspNetCore.Http.Json.JsonOptions>(o =>
{
    o.SerializerOptions.PropertyNamingPolicy = System.Text.Json.JsonNamingPolicy.SnakeCaseLower;
    o.SerializerOptions.DictionaryKeyPolicy  = System.Text.Json.JsonNamingPolicy.SnakeCaseLower;
});

builder.Services.AddRouting();

// OpenAPI document (parity with analytics-api). The committed contract at
// docs/components/backend/identity/openapi.json is regenerated from the live
// `GET /openapi.json` this serves, and gated against drift by the
// OpenApiContractTests integration test. Title/Version are pinned to the API
// contract — deliberately NOT the assembly version — so the drift gate fires
// only on real route/schema changes, not on every release bump.
builder.Services.AddOpenApi(options =>
{
    options.AddDocumentTransformer((document, _, _) =>
    {
        document.Info.Title = "Identity API";
        document.Info.Version = "1.0.0";
        document.Info.Description =
            "Resolves people, org-chart parent/subordinates, roles, and row-level "
            + "visibility for Insight. Backed by MariaDB (identity tables) with a "
            + "ClickHouse-sourced bulk re-seed. Fronted by the API Gateway.";
        // Drop the request-derived `servers` entry (e.g. the internal bind
        // http://0.0.0.0:8082). Consumers reach this service through the API
        // Gateway, not its pod address, and a host-specific URL would make the
        // committed contract environment-dependent — drifting the gate between
        // local generation and CI. Parity with analytics-api's empty `servers`.
        document.Servers.Clear();
        return Task.CompletedTask;
    });
});

var bindAddr = builder.Configuration[$"{AppOptions.SectionName}:bind_addr"]
    ?? builder.Configuration["bind_addr"]
    ?? "0.0.0.0:8082";
builder.WebHost.UseUrls($"http://{bindAddr}");

var app = builder.Build();

// Schema migrations — apply before opening the HTTP listener so requests
// never hit an unmigrated database. DbUp tracks applied scripts in its
// own SchemaVersions table; safe to re-run.
{
    var factory = app.Services.GetRequiredService<MariaDbConnectionFactory>();
    var loggerFactory = app.Services.GetRequiredService<ILoggerFactory>();
    var migrationLogger = loggerFactory.CreateLogger("Insight.Identity.Migrations");
    MigrationRunner.Run(factory.ConnectionString, migrationLogger);

    // Bootstrap admin — chicken-and-egg seed for the OrgChart Visibility tables.
    // Idempotent: only inserts when no active assignment for the
    // configured (tenant, person, admin-role) triple exists.
    var bootstrapLogger = loggerFactory.CreateLogger("Insight.Identity.Bootstrap");
    var appOptions = app.Services
        .GetRequiredService<Microsoft.Extensions.Options.IOptions<AppOptions>>().Value;
    await BootstrapAdminRunner.RunAsync(
        factory, appOptions.TenantDefaultId, appOptions.BootstrapAdminPersonId, bootstrapLogger)
        .ConfigureAwait(false);
}

// Request-logging redaction (PRD NFR-3). The default
// `UseSerilogRequestLogging` enricher captures `RequestPath` as the raw
// URL, which for `/v1/persons/{email}` would expose the email — PII.
// Override the property with a redacted form so logs never carry the
// caller's email address.
app.UseSerilogRequestLogging(options =>
{
    options.EnrichDiagnosticContext = (diagnosticContext, httpContext) =>
    {
        var path = httpContext.Request.Path.Value ?? string.Empty;
        if (path.StartsWith("/v1/persons/", StringComparison.OrdinalIgnoreCase))
        {
            path = "/v1/persons/<redacted>";
        }
        diagnosticContext.Set("RequestPath", path);
    };
});

app.UseExceptionHandler(handler =>
{
    handler.Run(async context =>
    {
        var feature = context.Features.Get<IExceptionHandlerFeature>();
        var ex = feature?.Error;

        var logger = context.RequestServices.GetRequiredService<ILoggerFactory>()
            .CreateLogger("Insight.Identity.Api.UnhandledException");
        // Log the route TEMPLATE, not the raw path (`/v1/persons/<email>`)
        // — see PRD NFR-3.
        var routeTemplate = (context.GetEndpoint() as Microsoft.AspNetCore.Routing.RouteEndpoint)?.RoutePattern.RawText
            ?? "<unmatched>";
#pragma warning disable CA1848 // single-call low-frequency error path; LoggerMessage adds noise here
        logger.LogError(ex, "Unhandled exception in {Route}", routeTemplate);
#pragma warning restore CA1848

        // db_target is meaningful only for DB-origin failures. Including
        // it on a generic NullReference / DI failure leaks irrelevant
        // infra detail and confuses callers debugging non-DB errors.
        var isDbException = ex is MySqlException or System.Data.Common.DbException;
        string detail;
        if (ex is null)
        {
            detail = "unknown error";
        }
        else if (isDbException)
        {
            var dbTarget = context.RequestServices.GetService<MariaDbConnectionFactory>()?.Target ?? "unknown";
            detail = $"{ex.GetType().Name}: {ex.Message} (db_target={dbTarget})";
        }
        else
        {
            detail = $"{ex.GetType().Name}: {ex.Message}";
        }

        context.Response.StatusCode = StatusCodes.Status500InternalServerError;
        var problem = new ProblemResponse(
            Type: "urn:insight:error:internal",
            Title: "Internal Server Error",
            Status: StatusCodes.Status500InternalServerError,
            Detail: detail);
        await context.Response.WriteAsJsonAsync(problem).ConfigureAwait(false);
    });
});

// Verify the gateway JWT, then enforce the fail-closed fallback policy
// (NGINX_BFF R1): every endpoint requires a valid gateway JWT unless it is
// marked AllowAnonymous (health probes, the OpenAPI document).
app.UseAuthentication();
app.UseAuthorization();

app.MapPersonsEndpoints();
app.MapVisibilityEndpoints();
app.MapRoleEndpoints();
app.MapPersonRoleEndpoints();
app.MapSubchartEndpoints();
app.MapPersonsSeedEndpoints();

// Serve the OpenAPI document at /openapi.json (parity with analytics-api).
// Anonymous so docs tooling and the drift gate can fetch the contract.
app.MapOpenApi("/openapi.json").AllowAnonymous();

await app.RunAsync().ConfigureAwait(false);

namespace Insight.Identity.Api
{
    /// <summary>Marker for the WebApplicationFactory in integration tests.</summary>
    public partial class Program;
}
