using FluentAssertions;
using Insight.Identity.Infrastructure.ClickHouse;
using Microsoft.Extensions.Options;
using Octonica.ClickHouseClient;
using Xunit;

namespace Insight.Identity.Tests.Unit;

/// <summary>
/// Covers the URL → connection-string parsing branches in
/// <see cref="ClickHouseConnectionFactory"/> (scheme strip, optional
/// credentials, optional port, optional database, percent-decoding)
/// without touching a real ClickHouse.
/// </summary>
public sealed class ClickHouseConnectionFactoryTests
{
    private static ClickHouseConnectionStringBuilder Build(ClickHouseOptions options) =>
        new(new ClickHouseConnectionFactory(Options.Create(options)).ConnectionString);

    [Fact]
    public void Parses_full_http_url_with_creds_port_and_database()
    {
        var b = Build(new ClickHouseOptions { Url = "http://insight:secret@ch-host:19000/identity" });

        b.Host.Should().Be("ch-host");
        b.Port.Should().Be(19000);
        b.User.Should().Be("insight");
        b.Password.Should().Be("secret");
        b.Database.Should().Be("identity");
    }

    [Fact]
    public void Parses_bare_clickhouse_scheme()
    {
        var b = Build(new ClickHouseOptions { Url = "clickhouse://u:p@host:9000/db" });

        b.Host.Should().Be("host");
        b.Port.Should().Be(9000);
        b.User.Should().Be("u");
        b.Password.Should().Be("p");
        b.Database.Should().Be("db");
    }

    [Fact]
    public void Parses_url_without_credentials()
    {
        var b = Build(new ClickHouseOptions { Url = "http://ch-host:8123/identity" });

        b.Host.Should().Be("ch-host");
        b.Port.Should().Be(8123);
        b.Database.Should().Be("identity");
    }

    [Fact]
    public void Parses_url_without_port()
    {
        var b = Build(new ClickHouseOptions { Url = "http://ch-host/identity" });

        b.Host.Should().Be("ch-host");
        b.Database.Should().Be("identity");
    }

    [Fact]
    public void Percent_decodes_credentials()
    {
        // A password containing reserved characters arrives percent-
        // encoded in the URL and must be decoded before use.
        var b = Build(new ClickHouseOptions { Url = "http://us%40er:p%40ss@host:9000/db" });

        b.User.Should().Be("us@er");
        b.Password.Should().Be("p@ss");
    }

    [Fact]
    public void Uses_discrete_fields_when_no_url()
    {
        var b = Build(new ClickHouseOptions
        {
            Host = "ch-host",
            Port = 19000,
            User = "insight",
            Password = "secret",
            Database = "identity",
        });

        b.Host.Should().Be("ch-host");
        b.Port.Should().Be(19000);
        b.User.Should().Be("insight");
        b.Password.Should().Be("secret");
        b.Database.Should().Be("identity");
    }

    [Fact]
    public void Throws_when_neither_url_nor_host_set()
    {
        // The build is lazy — the throw surfaces on first connection-
        // string access, not at construction.
        var factory = new ClickHouseConnectionFactory(Options.Create(new ClickHouseOptions()));

        Action act = () => _ = factory.ConnectionString;
        act.Should().Throw<ArgumentException>();
    }
}
