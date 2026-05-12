# M365 Connector

Microsoft 365 activity reports (email, Teams, OneDrive, SharePoint).

## Prerequisites

1. Create an App Registration in Azure AD
2. Grant application permissions: `Reports.Read.All`, `User.Read.All`
3. Create a client secret

## K8s Secret

Create a Kubernetes Secret with the connector credentials:

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-m365-main                          # convention: insight-{connector}-{source-id}
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: m365          # must match descriptor.yaml name
    insight.cyberfabric.com/source-id: main          # passed as insight_source_id
type: Opaque
stringData:
  azure_tenant_id: ""       # Azure AD tenant ID
  azure_client_id: ""       # App registration client ID
  azure_client_secret: ""   # App registration client secret
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `azure_tenant_id` | Yes | Azure AD tenant ID |
| `azure_client_id` | Yes | App registration client ID |
| `azure_client_secret` | Yes | App registration client secret (sensitive) |

### Automatically injected

These fields are set by `reconcile-connectors.sh` and should NOT be in the Secret:

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from ConfigMap `insight-config` (ns `data`) or `INSIGHT_TENANT_ID` env |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

All connector parameters are in the K8s Secret. Tenant identity is read from the cluster ConfigMap.

## Multi-Instance

To sync multiple Azure AD tenants, create separate Secrets with different `source-id` annotations:

```yaml
# Secret 1: insight-m365-main
annotations:
  insight.cyberfabric.com/source-id: main

# Secret 2: insight-m365-emea
annotations:
  insight.cyberfabric.com/source-id: emea
```
