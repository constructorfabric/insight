# Multi-tenancy

In plain words: what it takes for one Insight install to serve many tenants, who manages what, and how people sign in. Tracking issue: [#4299](https://github.com/constructorfabric/insight/issues/4299).

## TL;DR

- Today an install serves exactly one tenant, and its id is a chart value.
- The goal is many tenants in one install:
  - an operator with cluster access creates and removes tenants;
  - a **tenant admin** configures their own tenant: connectors and, where the identity provider cannot supply them, roles.
- Every tenant has its own subdomain, for example `acme.insight.example`. Its sign-in page lists the identity providers that tenant uses, and the user picks one.
- Every tenant signs in through an identity provider. Keycloak only brokers to those providers and stamps the tenant into the token.
- Data isolation ([#3661](https://github.com/constructorfabric/insight/issues/3661)) must land before a second tenant exists in an install.

## Where things stand

- **The tenant is a chart value.** It is set by `authenticator.oidc.defaultTenantId`, plus copies under `global.tenantDefaultId` and `ingestion.reconcile.tenantId`. The Keycloak config job stamps that one value into every login.
- **There is no tenant registry.** No table anywhere lists the tenants of an install.
- **Connectors are K8s Secrets.** Someone with cluster access applies them, and the reconcile job turns them into Airbyte and Argo objects, stamping them with the install-wide tenant. There is no API or UI for creating a connector.

## What needs to be done

### 1. Data isolation

Rows of different tenants never mix, and a service cannot read another tenant's rows:

- one tenant value end to end;
- one tenant column name from staging up;
- tenant-scoped dbt;
- row-level security in the warehouse, so a query that forgets the tenant filter still sees only its own tenant;
- tenant-aware analytics, insight-v3-core and MariaDB.

This is the [#3661](https://github.com/constructorfabric/insight/issues/3661) series; [#3875](https://github.com/constructorfabric/insight/pull/3875) explains it. It is a hard prerequisite for everything below.

### 2. Tenant registry

A tenant becomes a record, not a single chart value. Roughly:

- id (UUID) and name;
- status: active, suspended, deleting;
- its subdomain;
- its identity providers: at least one is required.

An operator with cluster access manages the registry; there is no admin UI above tenants for now. Once the registry exists, the chart copies of the tenant go away. Related: [#725](https://github.com/constructorfabric/insight/issues/725).

### 3. Sign-in and routing

**Every tenant has its own subdomain.** `acme.insight.example` belongs to one tenant, and the subdomain alone tells which tenant a visitor means. No e-mail domains are involved.

- **The sign-in page is per tenant.** It shows that tenant's branding and the identity providers it uses (for example GitHub, Entra, Google), and the user picks one.
- **Keycloak is a broker only.** It holds no users of its own. Each tenant's identity providers are registered in Keycloak separately, and each registration has a mapper with that tenant's id hardcoded. The tenant in the token always comes from that value, never from what the provider sends.
- **One person in several tenants works naturally.** The same GitHub account signs in at `a.insight.example` and at `b.insight.example`. These are two registrations, two sessions and two tokens, each carrying its own tenant. The session cookie is bound to the host, so the sessions never mix.
- Every subdomain needs DNS, a TLS certificate (a wildcard covers all of them) and its redirect URIs in Keycloak.

### 4. Tenant admin and connectors

- **Users come from the identity provider.** Insight does not create, invite or remove users.
- **Roles** are managed in Insight only where the identity provider cannot supply them.
- An API and a UI let the tenant admin create connectors and their secrets. Tenant isolation covers the secrets too: one tenant's admin never sees another tenant's secrets. How the secrets are stored is still open.
- Ingestion takes the tenant from each connector's record, never from an install-wide setting.
- Connector health and similar views show the caller's tenant only.
- Connectors run inside the cluster against URLs the tenant admin types in, so their egress has to be restricted (SSRF).

### 5. Tenant lifecycle

- **Suspend:** nobody can sign in, and syncs are paused.
- **Delete:** remove the tenant's data in every layer and store, its connectors and their Airbyte and Argo objects, and its identity provider registrations in Keycloak. This is one operation that can be verified and produces a report.

### 6. Operations

- **No tenant, no access.** Without a tenant there is no sign-in, so no connectors and no work that could write a record without a tenant. A request, job or record that reaches the system with an empty tenant is a fatal error, never something to fall back from.
- Quotas and limits per tenant: connectors, sync frequency, query load ([#724](https://github.com/constructorfabric/insight/issues/724)).
- Audit of what tenant admins do.

## Not now

Each of these is a separate task, picked up once there is an explicit requirement:

- **An admin UI above tenants.** For now, cluster access is that role.
- **A tenant hierarchy:** a main tenant with subtenants, and their subtenants.
- **Users kept by Insight itself**, for a small organization without an identity provider of its own.

## Open questions

- How are connector secrets stored, so that tenant isolation also holds for them?

## Order

1. Data isolation (block 1) comes before a second tenant exists in an install.
2. The registry and sign-in (blocks 2–3) can run in parallel with it. They touch the authenticator, identity and Keycloak, not dbt or the warehouse.
3. Connectors (block 4) come after the registry and sign-in.
4. Lifecycle and operations (blocks 5–6) come last.
