# M365 Connector

Microsoft 365 activity reports (email, Teams, OneDrive, SharePoint).

## Prerequisites

1. Create an App Registration in Azure AD
2. Grant application permissions: `Reports.Read.All`, `User.Read.All`.
   For the calendar meeting-time metric also grant `Calendars.ReadBasic.All`
   and set `m365_calendar: "true"` (see below).
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
| `m365_calendar` | No | `"true"` reads calendar events for Calendar Meeting Hours. Default `"false"`: no calendar request is made. |

### Calendar (opt-in)

With `m365_calendar: "true"` the connector lists enabled users and reads each
mailbox's calendar for the 27 finished UTC days before today, on every sync.
It needs the `Calendars.ReadBasic.All` application permission with admin
consent; without it the sync fails with a configuration error rather than
report no meetings.

What is stored per event: its times, the person's response, busy status,
cancelled and all-day flags, and the number of other people invited. Subjects,
bodies, locations and invitee addresses are never read or stored.

A user without a mailbox, or a mailbox that an application access policy keeps
out of the app's reach, is skipped and the sync continues. Exchange "RBAC for
Applications" scoping is not supported: it offers no role for
`Calendars.ReadBasic.All`, and the mailboxes it leaves out are denied with the
same error as a missing permission, which fails the sync.

The connection check does not cover the calendar; a missing permission shows
up on the first sync.

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
