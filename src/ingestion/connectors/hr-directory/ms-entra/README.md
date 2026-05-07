# MS Entra Connector

Microsoft Entra ID (formerly Azure AD) **user directory** via Microsoft Graph API. Pulls the canonical user list with email, UPN, employeeId, and other identity signals — feeds the Identity Manager so users authenticated against Entra can be resolved to their accounts in other services (GitHub, Slack, Jira, BambooHR, …).

Distinct from the `collaboration/m365` connector. m365 fetches **activity reports** (Reports.Read.All); this connector fetches **directory data** (User.Read.All). They use separate App Registrations to keep concerns and audit trails split.

## Prerequisites

1. In **Microsoft Entra Admin Center → App registrations**, create a dedicated App Registration (suggested name: `insight-entra-readonly`).
2. **API permissions → Add a permission → Microsoft Graph → Application permissions**:
   - `User.Read.All` — Read all users' full profiles
3. **Grant admin consent for {tenant}** — required, otherwise the role won't be in the access token (`roles` claim) and `/v1.0/users` returns `403 Authorization_RequestDenied`.
4. **Certificates & secrets → New client secret** — copy the value (shown only once).

> **Why a separate App Registration?**
> A single App Registration accumulating `Reports.Read.All` + `User.Read.All` is a wide blast radius. Splitting per-purpose apps keeps audit trails clean (Entra sign-in logs show which app touched which API), simplifies rotation, and reduces what's at risk if a secret leaks.
>
> **Why `User.Read.All` and not `User.ReadBasic.All`?**
> Identity resolution needs `proxyAddresses[]` (alternate emails — main signal for matching) and `employeeId` (cross-check with HR). Both are excluded from `User.ReadBasic.All`. The connector compensates with an explicit `$select` allowlist (see below) so the *actual* data volume collected stays small.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-ms-entra-main                      # convention: insight-{connector}-{source-id}
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: ms-entra      # must match descriptor.yaml name
    insight.cyberfabric.com/source-id: ms-entra-main # passed as insight_source_id
type: Opaque
stringData:
  azure_tenant_id: ""        # Microsoft Entra tenant (directory) ID
  azure_client_id: ""        # App registration client ID
  azure_client_secret: ""    # App registration client secret (sensitive)
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `azure_tenant_id` | Yes | Microsoft Entra tenant (directory) ID |
| `azure_client_id` | Yes | App registration client ID |
| `azure_client_secret` | Yes | App registration client secret (sensitive) |

### Automatically injected

These are set by `airbyte-toolkit/connect.sh` and must NOT be in the Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/ms-entra.yaml.example src/ingestion/secrets/connectors/ms-entra.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/ms-entra.yaml
```

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `users` | Entra user directory via Microsoft Graph `/v1.0/users` with `$select` allowlist | Full refresh |

### Field allowlist (privacy by default)

The `users` stream calls `/v1.0/users` with an explicit `$select` parameter. **Only** these fields are fetched, even though `User.Read.All` would allow the full profile:

| Field | Why we collect it |
|---|---|
| `id` | Stable Entra Object ID — equals JWT `oid` claim, primary key |
| `userPrincipalName` | UPN/SSO login |
| `mail` | Primary email |
| `proxyAddresses` | Alternate SMTP addresses — main fuzzy-match signal |
| `otherMails` | Additional emails — secondary match signal |
| `displayName` / `givenName` / `surname` | Display fields and fuzzy-name matching |
| `employeeId` | Cross-check with BambooHR / HR system |
| `department` / `jobTitle` | Org context |
| `accountEnabled` | Distinguish active / disabled accounts |
| `onPremisesSamAccountName` | Match users synced from on-prem AD |
| `createdDateTime` | Provenance / valid_from for SCD2 |
| `userType` | Member / Guest discriminator |

The connector intentionally does **not** request: `birthday`, `aboutMe`, `interests`, `skills`, `pastProjects`, `schools`, `mySite`, `mobilePhone`, `streetAddress`, `imAddresses`, `hireDate`, `ageGroup`, `legalAgeGroupClassification`, `consentProvidedForMinor`, `identities`. These are out of scope for identity resolution and represent a privacy risk if collected.

### Why no incremental sync (yet)

Microsoft Graph supports `/users/delta` with an opaque `$deltatoken` for change tracking, but the declarative Airbyte runtime can't drive an opaque-token cursor without a custom component. For the first iteration this connector runs **full refresh** — Bronze is `ReplacingMergeTree`, so re-emitting the same `unique_key` is a no-op. Directory size is typically small (1k–50k users) and a daily full pull is acceptable.

Delta-token support is tracked as a follow-up enhancement.

## Multi-instance

Multiple Entra tenants can be synced by creating separate Secrets with different `source-id` annotations:

```yaml
# Secret 1: insight-ms-entra-main      → source-id: ms-entra-main
# Secret 2: insight-ms-entra-emea-tenant → source-id: ms-entra-emea-tenant
```

## Bronze schema

Database: `bronze_ms_entra`

| Table | Primary key | Description |
|---|---|---|
| `users` | `unique_key` | One row per Entra user. `id` field equals JWT `oid` claim. |

See `docs/CONNECTORS_REFERENCE.md` for full Bronze column-level details.

## dbt pipeline

| Model | Purpose |
|---|---|
| `ms_entra__bronze_promoted` | Promote raw Bronze tables to `ReplacingMergeTree` (per `promote_bronze_to_rmt` macro) |
| `ms_entra__users_snapshot` | SCD2 append-only snapshot of users (change tracking) |
| `ms_entra__users_fields_history` | Field-level change log derived from snapshot |
| `ms_entra__identity_inputs` | Emit identity signals (`mail`, `userPrincipalName`, `employeeId`, `displayName`, `onPremisesSamAccountName`) into `silver:identity_inputs` |
| `to_class_people` | Bronze users → Silver Step 1 `class_people` shape |

## Silver Targets

- `class_people` — unified person registry (joined with BambooHR and other directory sources via `union_by_tag('silver:class_people')`).
- `identity_inputs` — feeds the Identity Manager for `oid → person_id` resolution.
