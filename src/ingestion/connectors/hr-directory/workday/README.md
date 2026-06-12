# Workday Connector

Worker directory and time-off requests from Workday via RaaS (Reports-as-a-Service) custom reports with ISU Basic authentication.

Unlike BambooHR, Workday has no fixed bulk-extraction endpoint: the customer builds two custom reports in Workday Report Writer following the **report contract** below, and the connector fetches them as JSON. The field set is therefore controlled on the Workday side; the connector validates the configured reports at `check` time.

## Prerequisites

1. Create an **Integration System User (ISU)** in Workday and an Integration System Security Group containing it.
2. Grant the security group the domains needed by the report fields (at minimum: Worker Data: Public Worker Reports, Person Data: Work Email).
3. Build the two custom reports per the contract below (type **Advanced**, data source **All Workers** / time-off requests), enable them as a web service, and share them with the ISU.
4. Note the RaaS base URL from the report's web-service URL: `https://<host>/ccx/service/customreport2/<workday_tenant>`.

## Report Contract

### Workers report (`workday_workers_report_path`)

The report MUST expose the following columns with these exact XML aliases (set the alias explicitly on each column — auto-generated aliases differ per tenant):

| Column alias | Workday field | Notes |
|--------------|---------------|-------|
| `Employee_ID` | Employee ID | Primary key |
| `Display_Name` | Preferred Name | |
| `First_Name` | Legal First Name | |
| `Last_Name` | Legal Last Name | |
| `Work_Email` | Primary Work Email | Identity key — may be empty for contingent workers |
| `Business_Title` | Business Title | |
| `Job_Profile` | Job Profile | |
| `Worker_Type` | Worker Type | `Employee` / `Contingent Worker` |
| `Worker_Status` | Active Status | `Active` / `On Leave` / `Terminated` |
| `Supervisory_Organization` | Supervisory Organization | Workday's standard org unit |
| `Manager_Employee_ID` | Manager Employee ID (management chain) | |
| `Manager_Work_Email` | Manager Work Email (management chain) | |
| `Location` | Location | |
| `Country` | Location Address — Country | |
| `City` | Location Address — City | |
| `Hire_Date` | Hire Date | |
| `Original_Hire_Date` | Original Hire Date | |
| `Termination_Date` | Termination Date | Empty if active |
| `Last_Functionally_Updated` | Last Functionally Updated | |
| `Scheduled_Weekly_Hours` | Scheduled Weekly Hours | |

Extra columns (custom fields, calculated fields) are allowed: they land in the Bronze `raw_data` column and can be tracked by dbt via the `workday_custom_fields` var.

### Leave report (`workday_leave_report_path`)

| Column alias | Workday field | Notes |
|--------------|---------------|-------|
| `Request_ID` | Time Off Request ID | Primary key |
| `Employee_ID` | Employee ID | Joins to workers |
| `Time_Off_Type` | Time Off Type | Policy-defined, client-specific |
| `Start_Date` | First Day of Time Off | |
| `End_Date` | Last Day of Time Off | |
| `Quantity` | Units Requested | |
| `Unit` | Unit of Time | hours / days |
| `Status` | Request Status | |
| `Submitted_Moment` | Request Initiated (moment) | |

The report MUST define two prompts enabled as web service parameters: `From_Date` and `To_Date` (date range filter on time-off dates). The connector passes `From_Date` from `workday_start_date` and `To_Date` as the current UTC date.

As with the workers report, extra columns are allowed and land in the Bronze `raw_data` column.

## K8s Secret

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: insight-workday-main
  labels:
    app.kubernetes.io/part-of: insight
  annotations:
    insight.cyberfabric.com/connector: workday
    insight.cyberfabric.com/source-id: workday-main
type: Opaque
stringData:
  workday_base_url: ""                  # https://<host>/ccx/service/customreport2/<workday_tenant>
  workday_isu_username: ""              # Integration System User
  workday_isu_password: ""              # ISU password
  workday_workers_report_path: ""       # <report_owner>/<Report_Name>
  workday_leave_report_path: ""         # <report_owner>/<Report_Name>
  workday_start_date: "2020-01-01"      # Optional: time-off history start date
```

### Fields

| Field | Required | Description |
|-------|----------|-------------|
| `workday_base_url` | Yes | RaaS base URL incl. Workday tenant, no trailing slash: `https://<host>/ccx/service/customreport2/<workday_tenant>` |
| `workday_isu_username` | Yes | Integration System User username |
| `workday_isu_password` | Yes | Integration System User password |
| `workday_workers_report_path` | Yes | Workers report path: `<report_owner>/<Report_Name>` |
| `workday_leave_report_path` | Yes | Leave report path: `<report_owner>/<Report_Name>` |
| `workday_start_date` | No | Time-off history start date, ISO format (default: `2020-01-01`) |

> **Note on `username` / `password` spec fields.**
> The Airbyte Builder auto-generates `username` and `password` properties in
> `connection_specification` because the connector uses `BasicHttpAuthenticator`.
> These are managed automatically by the authenticator config:
> `username` = `workday_isu_username`, `password` = `workday_isu_password`.
> Do **not** set them in the K8s Secret or credentials file — they are not
> user-provided values.

### Automatically injected

| Field | Source |
|-------|--------|
| `insight_tenant_id` | `tenant_id` from tenant YAML |
| `insight_source_id` | `insight.cyberfabric.com/source-id` annotation |

### Local development

Create `src/ingestion/secrets/connectors/workday.yaml` (gitignored) from the example:

```bash
cp src/ingestion/secrets/connectors/workday.yaml.example src/ingestion/secrets/connectors/workday.yaml
# Fill in real values, then apply:
kubectl apply -f src/ingestion/secrets/connectors/workday.yaml
```

There is no public Workday developer tenant. For local development without tenant access, use the bundled mock RaaS server ([fixtures/mock_raas.py](./fixtures/mock_raas.py)) — it replays the fixture responses (`{"Report_Entry": [...]}` wrapper) and enforces Basic auth, `format=json`, and the leave report's `From_Date`/`To_Date` prompts:

```bash
# Terminal 1: start the mock
python3 src/ingestion/connectors/hr-directory/workday/fixtures/mock_raas.py --port 8765

# Terminal 2: point the local secret at the mock and run the connector
# (workday_base_url: "http://host.docker.internal:8765/ccx/service/customreport2/acme",
#  report paths: ISU_Insight/Insight_Employee_Sync, ISU_Insight/Insight_Leave_Sync)
cd src/ingestion
./tools/declarative-connector/source.sh check hr-directory/workday example-tenant
./tools/declarative-connector/source.sh read  hr-directory/workday example-tenant
```

Validate against the customer's Sandbox tenant during onboarding.

## Streams

| Stream | Description | Sync Mode |
|--------|-------------|-----------|
| `workers` | Worker directory via RaaS custom report | Full refresh |
| `leave_requests` | Time-off requests via RaaS custom report (From_Date/To_Date prompts) | Full refresh |

## Silver Targets

- `class_people` — unified person registry
- `class_hr_events` — leave events
- `class_hr_working_hours` — scheduled working hours
- `identity_inputs` — identity observations via SCD2 snapshot → fields_history chain
