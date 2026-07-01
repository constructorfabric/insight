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
using Microsoft.IdentityModel.JsonWebTokens;
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

// Composite tenant resolver: header → JWT → config default.
builder.Services.AddSingleton<HeaderTenantContext>();
builder.Services.AddSingleton<JwtTenantContext>();
builder.Services.AddSingleton<ConfigTenantContext>();
builder.Services.AddSingleton<ITenantContext>(sp => new CompositeTenantContext(new ITenantContext[]
{
    sp.GetRequiredService<HeaderTenantContext>(),
    sp.GetRequiredService<JwtTenantContext>(),
    sp.GetRequiredService<ConfigTenantContext>(),
}));

// JWT bearer authentication — parse-only mode. The api-gateway already
// validates the token upstream (issuer, audience, signature, lifetime)
// before forwarding the request, so this service treats the JWT as a
// context-bearing envelope: the middleware decodes the payload into a
// ClaimsPrincipal that downstream resolvers (JwtTenantContext, and the
// upcoming caller-id resolver tracked under #346) can read. No
// endpoint enforces authentication in this PR — anonymous requests
// still pass through unchanged.
//
// TODO(#346): switch to full validation once the IdP authority is
// pinned per environment. The block below is the swap-in skeleton —
// every line must flip together. The `SignatureValidator = null` line
// is load-bearing: without it the no-op below keeps short-circuiting
// signature checks even with `ValidateIssuerSigningKey = true`, which
// would also silently accept `alg=none` tokens.
//     options.Authority = configuration["identity:auth_authority"];
//     options.Audience  = configuration["identity:auth_audience"];
//     options.TokenValidationParameters.ValidateIssuer            = true;
//     options.TokenValidationParameters.ValidateAudience          = true;
//     options.TokenValidationParameters.ValidateLifetime          = true;
//     options.TokenValidationParameters.ValidateIssuerSigningKey  = true;
//     options.TokenValidationParameters.RequireSignedTokens       = true;
//     options.TokenValidationParameters.SignatureValidator        = null;
builder.Services
    .AddAuthentication(JwtBearerDefaults.AuthenticationScheme)
    .AddJwtBearer(options =>
    {
        options.RequireHttpsMetadata = false;
        // Keep JWT claim names as-is (`email`, `sub`, `oid`, …) so we
        // can read them by their short JWT names. Without this, the
        // default JwtBearer pipeline rewrites `email` / `sub` / `name`
        // into long ClaimTypes.* URIs (a legacy WS-Federation hand-me-
        // down), and `FindFirst("email")` would return null while
        // `oid` (not in the rewrite table) would still work — easy to
        // miss in review. False keeps every claim under one rule.
        options.MapInboundClaims = false;
        options.TokenValidationParameters = new TokenValidationParameters
        {
            ValidateIssuer = false,
            ValidateAudience = false,
            ValidateLifetime = false,
            ValidateIssuerSigningKey = false,
            RequireSignedTokens = false,
            // Accept any token shape; do not enforce signature. Returning
            // a parsed JsonWebToken short-circuits the default signature
            // verifier and lets the claim pipeline run.
            SignatureValidator = (token, _) => new JsonWebToken(token),
        };
    });

// Caller resolver — header first, then JWT claims (oid/sub via
// account_person_map, then email/preferred_username/upn via persons).
// Scoped because resolution hits MariaDB through IPersonsReader.
builder.Services.AddScoped<ICallerContext, HeaderCallerContext>();

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

// Populate HttpContext.User from a Bearer token when present. No
// UseAuthorization() — endpoints stay anonymous (#346 will add the
// caller-id check on top once visibility lands).
app.UseAuthentication();

app.MapPersonsEndpoints();
app.MapVisibilityEndpoints();
app.MapRoleEndpoints();
app.MapPersonRoleEndpoints();
app.MapSubchartEndpoints();
app.MapPersonsSeedEndpoints();

// Serve the OpenAPI document at /openapi.json (parity with analytics-api).
// Public — no caller/tenant header required — so docs tooling and the drift
// gate can fetch the contract; identity endpoints are anonymous today anyway.
app.MapOpenApi("/openapi.json");

await app.RunAsync().ConfigureAwait(false);

namespace Insight.Identity.Api
{
    /// <summary>Marker for the WebApplicationFactory in integration tests.</summary>
    public partial class Program;
}
