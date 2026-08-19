# github-directory

Collects the GitHub organization member roster and feeds it into
`identity.identity_inputs`, so a person who signs in through a GitHub-brokered
IdP resolves to an Insight person.

This connector carries **no** repository, pull-request or commit data — that is
the separate `github` connector. The split is deliberate: identity is a
prerequisite for logging in at all, and it should not wait on a connector with
a much larger surface and its own dependencies.

Split in deployment, however, the two are one identity space: an account is
keyed on (source type, source id, login), and this roster binds a login to a
person while the `github` connector claims the e-mails that login commits
under. Both Secrets therefore carry the SAME
`insight.cyberfabric.com/source-id` — `github-main`, not this connector's own
name. Given different ids the two halves never meet, and a member whose profile
hides their e-mail resolves to nobody or to a second, nameless person.

## Stream

| Stream | Mode | Source |
|---|---|---|
| `org_members` | full refresh | GraphQL `organization.membersWithRole`, one query per configured org |

The roster is current state and the API offers no change filter, so each sync
re-reads it. Bronze is append-only and collapses by `unique_key` under
ReplacingMergeTree, so re-delivery is free.

## Why this makes login work

The authenticator resolves a session by calling identity-resolution with
`GET /internal/persons/by-external-id?source_type=<idp.source_type>&external_id=<claim>`.
Those `persons` rows are seeded from `identity.identity_inputs`. With no GitHub
contributor there, every GitHub login resolves to nothing and the callback
returns 403 — indistinguishable from a permissions problem.

The dbt chain here closes that gap:

```text
bronze_github_directory.org_members
  → github_directory__org_members_snapshot      SCD2 versions
  → github_directory__org_members_fields_history  per-field changes
  → github_directory__identity_inputs           tagged silver:identity_inputs
  → silver.identity_inputs                      (union_by_tag)
```

`insight_source_type` is stamped **`github`**, not `github-directory`: it names
the vendor a person authenticated against, not the connector package that
supplied the roster.

## The join key

`identity_inputs` emits a `value_type='id'` row whose value is the **lowercased
GitHub login**, and that is what the login lookup matches.

Lowercasing is not cosmetic. The lookup compares against
`persons.value_id`, declared `VARCHAR(320) COLLATE utf8mb4_bin` — a byte-exact,
case-sensitive comparison — while Keycloak lowercases the username it brokers
from GitHub. Storing GitHub's original casing (`Some-User`) against a
lowercased claim (`some-user`) yields no match and a 403 that looks exactly
like missing data. GitHub logins are unique case-insensitively, so normalizing
loses nothing.

The raw login is kept alongside as `login`, and emitted as a `username`
observation.

## Deployment

The connector is necessary but not sufficient. The instance must also point the
authenticator at a claim that carries the GitHub login:

```yaml
authenticator:
  oidc:
    sourceType: github          # must equal insight_source_type above
    externalIdClaim: <claim>    # must carry the GitHub login
```

GitHub is OAuth2, not OIDC — it issues no id_token for user login — so the
claim is minted by the broker. In Keycloak, either:

- `preferred_username`, which for a GitHub-brokered user is the login,
  lowercased by the realm's username policy; or
- a dedicated claim: an **Attribute Importer** identity-provider mapper copying
  GitHub's `login` into a user attribute, plus a **User Attribute** protocol
  mapper exposing it on the ID token.

If you add a mapper, set **Claim JSON Type = String**. A numeric claim is
dropped by the authenticator's string extraction and login fails closed — which
is also why the numeric GitHub user id is a poor choice of external id here.

Whichever claim is used, its value must arrive lowercased to match.

## Configuration

| Field | Required | Notes |
|---|---|---|
| `github_token` | yes | `read:org` **and** `user:email` (or `read:user`) — both required |
| `github_organizations` | yes | JSON array of org logins |
| `insight_tenant_id` / `insight_source_id` | yes | injected by the reconcile loop |

`user:email` is not optional. GitHub validates the whole GraphQL document
before executing it and rejects it outright when the token cannot read the
`email` field, so a `read:org`-only token collects nothing at all rather than
returning members with null emails. The connector fails the check with GitHub's
own message in that case.

With the scope granted, `email` carries a member's org email wherever one is
verified and visible to the token. Where it is absent, identity resolution
leans on the id binding and display name instead — the login is the join key
either way, so a missing email degrades merging with other sources rather than
breaking login.

A member of two configured orgs produces one bronze row per org and a single
identity entity — both rows carry the same normalized login. The duplicate
observations are harmless; the latest wins.

## Limitation: removal is not observed

Removing someone from the organization does **not** deactivate their binding.
GitHub exposes no per-member disabled flag — a removed member simply stops
appearing in the roster — and the identity macro's deactivation branch fires on
field *changes*, so an absence produces nothing to match on. Detecting it needs
a roster diff across syncs, which is a separate model.

Treat this connector as an account *directory*, not an authorization source.
Revocation belongs at the IdP: a directory synced on a schedule cannot be an
access-revocation mechanism at any fidelity, because even a perfect roster diff
would leave access standing until the next sync. Restrict membership at the
broker rather than relying on this pipeline to withdraw it.

## Development

```bash
src/ingestion/tools/declarative-connector/source.sh validate-strict git/github-directory
```

```bash
cd src/ingestion/tests/connectors && pytest ../../connectors/git/github-directory/tests -q
```
