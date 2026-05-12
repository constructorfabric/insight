# Salesforce Connector

CDK-based Python connector for Salesforce CRM. Pulls data via Bulk API 2.0 with REST `/queryAll` fallback; describe-driven field discovery means no SOQL maintenance as SF orgs evolve; custom (`__c`) fields are captured into a single `custom_fields` JSON column so Bronze stays stable across orgs.

Architecture adapted from the [official Airbyte Salesforce connector](https://github.com/airbytehq/airbyte/tree/master/airbyte-integrations/connectors/source-salesforce) (ELv2). Per-file copyright headers preserved.

## Prerequisites

1. Create an **External Client App** in Salesforce Setup with **OAuth 2.0 Client Credentials Flow** enabled.
2. On the app's **Policies** tab, set a **Run-As user** (service account) and save. Wait 5–10 minutes for propagation.
3. Grant scopes: `api`, `refresh_token`.
4. Note the **Consumer Key** (→ `salesforce_client_id`), **Consumer Secret** (→ `salesforce_client_secret`), and the org's **Instance URL** (e.g. `https://acme.my.salesforce.com`).

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-salesforce-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: salesforce
    insight.cyberfabric.com/source-id: salesforce-main
type: Opaque
stringData:
  salesforce_instance_url: ""                      # https://mycompany.my.salesforce.com
  salesforce_client_id: ""                         # OAuth Consumer Key
  salesforce_client_secret: ""                     # OAuth Consumer Secret
  salesforce_start_date: "2024-01-01T00:00:00Z"    # Optional
  salesforce_num_workers: "20"                     # Optional (1–50)
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `salesforce_instance_url` | Yes | Salesforce instance URL (OAuth token endpoint is derived from this) |
| `salesforce_client_id` | Yes | OAuth Consumer Key |
| `salesforce_client_secret` | Yes | OAuth Consumer Secret (sensitive) |
| `salesforce_start_date` | No | Incremental sync start (ISO 8601). Defaults to two years before current date (computed in `source.py`). |
| `salesforce_streams` | No | JSON array of sobject names to sync. Overrides curated default |
| `salesforce_stream_slice_step` | No | Concurrent cursor window. Default `P30D` |
| `salesforce_lookback_window` | No | Re-read window for SystemModstamp consistency. Default `PT10M` |
| `salesforce_force_use_bulk_api` | No | Force Bulk even for unsupported types. Default `false` |
| `salesforce_num_workers` | No | Max concurrent jobs (1–50). Default `20` |

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Multi-instance

Deploy additional Secrets with distinct `source-id` annotations to ingest multiple Salesforce orgs as separate sources.

### Local development

```bash
cp src/ingestion/secrets/connectors/salesforce.yaml.example src/ingestion/secrets/connectors/salesforce.yaml
# fill in real values, then:
./src/ingestion/secrets/apply.sh
```

## Streams

Active: 10 sobjects (curated for analytics value vs sync cost). Operator can
override via `salesforce_streams` config; additional disabled sobjects listed
at the bottom of `constants.CRM_STREAMS` can be re-enabled there or supplied
ad-hoc via config.

### Active

| Stream (sobject) | Purpose |
|---|---|
| `Account` | Companies; every CRM join flows through it |
| `Contact` | People; deal-contact + activity joins |
| `Opportunity` | Deals / revenue pipeline |
| `OpportunityHistory` | Stage/amount change history — pipeline velocity |
| `Task` | Activities: calls, emails, tasks |
| `Event` | Activities: meetings, scheduled events |
| `User` | Sellers / reps — ownership attribution, leaderboards |
| `Lead` | Top-of-funnel; lead→contact conversion analytics |
| `Case` | Support tickets — CX analytics |
| `OpportunityContactRole` | Buyer-committee / multi-threading |

All incremental via `SystemModstamp` (except `OpportunityHistory` → `CreatedDate`).
Bulk API by default; REST fallback for sobjects with compound / base64 fields.
Soft-deleted records (`IsDeleted=true`) included — `queryAll` used throughout.

### Available, disabled by default

`OpportunityLineItem`, `Product2`, `Pricebook2`, `PricebookEntry`,
`Campaign`, `CampaignMember`. Re-enable when revenue-by-product or marketing
attribution work lands.

## Bronze schema

Every stream's Bronze table has:

- **SF fields** — typed columns generated from `describe()`. Standard fields only; custom (`__c`) fields go to the JSON blob below.
- **`tenant_id`, `source_id`** — tenant/instance scope.
- **`unique_key`** — `{tenant_id}-{source_id}-{Id}` — stable surrogate PK.
- **`data_source`** — literal `"salesforce"`.
- **`collected_at`** — UTC ISO-8601 timestamp of the sync.
- **`custom_fields`** — JSON string containing every `__c` field. Access in dbt via `JSONExtractString(custom_fields, 'MyField__c')`.

## Silver targets

Staging models under `dbt/` feed these Silver classes:

- `class_crm_accounts`
- `class_crm_contacts`
- `class_crm_deals`
- `class_crm_activities`
- `class_crm_users`

Silver classes for Lead / Case / OpportunityContactRole / OpportunityLineItem / CampaignMember land in Bronze only for now; Silver unification happens once the HubSpot connector lands for cross-source parity.

## Build & deploy

```bash
cd src/ingestion
./airbyte-toolkit/build-connector.sh crm/salesforce   # docker build + Kind load + Airbyte definition update
./airbyte-toolkit/connect.sh <tenant>                 # create source + connection from K8s Secret
./run-sync.sh salesforce <tenant>                     # e2e via Argo (sync + dbt)
./logs.sh -f latest                                   # follow
```

## Troubleshooting

- `invalid_grant: request not supported on this domain` — Run-As user missing on External Client App, or app just created (wait 5–10 min for policy propagation), or `salesforce_instance_url` points to `.salesforce-setup.com` (setup UI domain) instead of `.my.salesforce.com`.
- `INVALID_FIELD` — describe-driven; this error means the sobject genuinely lacks the field in the Run-As user's profile. Check Field-Level Security on that profile.
- `REQUEST_LIMIT_EXCEEDED` — 24-hour rolling quota reached. Error is surfaced as transient; sync will fail cleanly. Reduce `salesforce_num_workers` or `salesforce_stream_slice_step` to throttle.
- `INVALID_SESSION_ID` — token expired. Connector auto-refreshes every 30 min and on 401; if persistent, rotate `salesforce_client_secret`.
