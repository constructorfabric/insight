using Microsoft.IdentityModel.Protocols;
using Microsoft.IdentityModel.Tokens;

namespace Insight.Identity.Api.Auth;

/// <summary>
/// Fetches a bare JWKS document and parses it into a <see cref="JsonWebKeySet"/>.
///
/// The authenticator publishes its signing keys at
/// <c>/.well-known/jwks.json</c> but serves no OIDC discovery document, so the
/// standard <c>Authority</c>/metadata path is unavailable. Paired with a
/// <c>ConfigurationManager&lt;JsonWebKeySet&gt;</c> this gives periodic key
/// refresh and auto-refresh on an unknown <c>kid</c> — i.e. key rotation
/// support — with no discovery endpoint.
/// </summary>
public sealed class JwksRetriever : IConfigurationRetriever<JsonWebKeySet>
{
    public async Task<JsonWebKeySet> GetConfigurationAsync(
        string address,
        IDocumentRetriever retriever,
        CancellationToken cancel)
    {
        ArgumentNullException.ThrowIfNull(retriever);
        var json = await retriever.GetDocumentAsync(address, cancel).ConfigureAwait(false);
        return new JsonWebKeySet(json);
    }
}
