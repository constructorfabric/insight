# Active Directory (LDAP) Connector

On-prem **Active Directory user directory** via LDAP/LDAPS (`ldap3`). Pulls the
canonical user list — objectGUID, sAMAccountName, UPN, mail, employeeId — and feeds
the Identity Manager so users authenticated against AD can be resolved to their
accounts in other services (GitHub, Slack, Jira, BambooHR, …).

This is a **CDK (Python)** connector, not nocode: Active Directory is queried over
LDAP, not HTTP, so the declarative manifest framework cannot express it.

**Sibling of [`../ms-entra`](../ms-entra/) (cloud Microsoft Entra ID).** Both emit the
same `class_people` / `identity_inputs` Silver contract; only the transport differs
(LDAP vs Microsoft Graph). Run **both** when on-prem AD is synced to Entra — the
shared `sam_account` identity signal (`sAMAccountName` here ==
`onPremisesSamAccountName` in ms-entra) lets the Identity Manager reconcile the same
person across the on-prem and cloud directories.

## Prerequisites

1. A **read-only service account** in AD (e.g. `svc-insight`). It needs nothing beyond
   default authenticated-user read on the user objects in scope.
2. Network reachability from the cluster to a **domain controller** on the LDAP port
   (636 for LDAPS, 389 for plaintext). **Use LDAPS** — a plaintext simple bind sends
   the password in the clear.
3. The **base DN** to search (forest root `DC=corp,DC=example,DC=com`, or a narrower OU).

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-active-directory-main          # convention: insight-{connector}-{source-id}
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: active-directory   # must match descriptor.yaml name
    insight.cyberfabric.com/source-id: active-directory-main  # passed as insight_source_id
type: Opaque
stringData:
  ad_server_host: ""        # DC hostname/IP, no scheme/port (e.g. "dc1.corp.example.com")
  ad_bind_dn: ""            # service-account DN or UPN (e.g. "svc-insight@corp.example.com")
  ad_bind_password: ""      # service-account password (sensitive)
  ad_search_base: ""        # base DN (e.g. "DC=corp,DC=example,DC=com")
  # Optional:
  # ad_port: "636"          # default 636 (LDAPS) / 389 (plaintext)
  # ad_use_ssl: "true"      # default true
  # ad_user_filter: "(&(objectClass=user)(objectCategory=person)(!(sAMAccountName=krbtgt)))"
  # ad_page_size: "500"
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `ad_server_host` | Yes | Domain controller hostname/IP (no scheme, no port) |
| `ad_bind_dn` | Yes | Bind identity — full DN or UPN of a read-only account |
| `ad_bind_password` | Yes | Bind account password (sensitive) |
| `ad_search_base` | Yes | Base DN for the user search |
| `ad_port` | No | LDAP port (default 636 LDAPS / 389 plaintext) |
| `ad_use_ssl` | No | Use LDAPS/TLS (default `true`) |
| `ad_user_filter` | No | LDAP filter for user objects (default excludes computers/contacts/krbtgt) |
| `ad_page_size` | No | Paged-search page size (default 500; AD `MaxPageSize` is 1000) |

### Automatically injected

Set by `reconcile-connectors` / `connect.sh`, must NOT be in the Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

```bash
cp src/ingestion/secrets/connectors/active-directory.yaml.example src/ingestion/secrets/connectors/active-directory.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/active-directory.yaml
```

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `users` | AD user accounts (person objects) via LDAP paged search | Full refresh |

### Privacy

The connector fetches an **explicit attribute allowlist** (see
`source_active_directory/ldap_client.py:USER_ATTRIBUTES`) — only identity-resolution
fields. It never reads photos, addresses, phone numbers, or other PII even when the
bind account could. Mirrors the ms-entra `$select` allowlist.

## Silver Targets

- `class_people` — unified person registry (via `active_directory__to_class_people`)
- `identity_inputs` — identity signals for the Identity Manager (via `active_directory__identity_inputs`)

## Build & deploy (CDK)

```bash
cd src/ingestion
# Build image + register Airbyte source definition (reconcile-connectors / cdk-build.sh)
./reconcile-connectors/main.sh                       # discovers, builds, registers, connects
# Or per the connector skill:  /connector build hr-directory/active-directory
./run-sync.sh active-directory <tenant>              # e2e: Airbyte sync → dbt (Bronze → Silver)
./logs.sh -f latest
```

> objectGUID is AD's stable identity anchor: it survives renames, OU moves, and UPN
> changes, so `source_person_id` / `unique_key` stay constant for a person's lifetime.
