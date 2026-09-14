# MCP authentication

MCP clients authorize through the browser once, using an active administrator
account. Approval creates an independent authorization grant valid for 30 days
from approval. Refreshing does not extend this deadline.

Access tokens default to 10 minutes. Clients refresh them silently using rotating,
single-use refresh tokens. Every refresh checks the person's current administrator
role in the authorized tenant. Temporary Identity failures return a retryable
OAuth error without consuming the refresh token.

Browser session expiry and logout do not revoke MCP authorization. Browser session
settings and refresh behavior are unchanged. Revoke the current MCP refresh token
through `/auth/oauth/revoke` to prevent further refreshes. Already-issued access
tokens remain valid until expiry (10 minutes by default). Removing administrator
access likewise prevents refresh but does not invalidate an already-issued token.

After 30 days, authorize again through the client. Refresh grants are stored in
Redis; losing that state also requires reauthorization.

## Upgrade and rollback

This release uses versioned authorization-code and refresh-token keys. Existing
connections require one browser reauthorization after upgrade; session-bound
credentials are not converted into longer-lived grants. Previous keys expire
naturally. Client registrations are unchanged.

During a rolling upgrade, old replicas cannot refresh newly issued grants. Complete
the authenticator rollout before reconnecting clients. Rolling back requires
reauthorizing connections created by this release.
