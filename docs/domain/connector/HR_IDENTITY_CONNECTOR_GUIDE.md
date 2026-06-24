# HR / Identity Connector Authoring Guide (agent-facing)

Companion to the canonical spec [`specs/DESIGN.md`](specs/DESIGN.md). DESIGN.md §4 is
the **general** connector guide (manifest structure, auth, pagination, AddFields,
schema rules, deployment). This file is the **HR / identity directory** specialization:
the exact, repeatable recipe for a connector whose job is "pull a user directory and
feed the Identity Manager". Read DESIGN.md first; this narrows it.

> Scope guard for agents: an HR/identity connector lives under
> `src/ingestion/connectors/hr-directory/{name}/`. Before building a new one, check
> the three references below — the pattern is fixed; you are filling in a template,
> not inventing structure.

## Reference connectors (copy these, do not start blank)

| Connector | Transport | Type | Use as template when… |
|-----------|-----------|------|------------------------|
| [`hr-directory/bamboohr`](../../../src/ingestion/connectors/hr-directory/bamboohr/) | REST + API key | **nocode** | Source is a REST/HTTP HR system. Adds `leave_requests`/`working_hours`/`hr_events`. |
| [`hr-directory/ms-entra`](../../../src/ingestion/connectors/hr-directory/ms-entra/) | MS Graph REST + OAuth2 | **nocode** | Source is a cloud directory over HTTP. Minimal users-only identity connector. |
| [`hr-directory/active-directory`](../../../src/ingestion/connectors/hr-directory/active-directory/) | LDAP/LDAPS | **cdk** | Source is **not HTTP** (LDAP, SOAP, file, DB). Plain `Stream` + custom transport. |

`bamboohr` and `ms-entra` are nocode (declarative `connector.yaml`). `active-directory`
is the canonical example of a **CDK connector for a non-HTTP source** — see below.

## The HR/identity Silver contract (this is the point)

Every HR/identity connector, regardless of transport, MUST emit the **same** Silver
surface so the Identity Manager and the unified person registry work across sources:

```
bronze_{name}.users (or .employees)        ← extraction writes here
        │  (connector AddFields / parse: tenant_id, source_id, unique_key)
        ▼
{name}__bronze_promoted.sql                 ← promote_bronze_to_rmt(table, order_by='unique_key')
        ▼
{name}__users_snapshot.sql                  ← snapshot(...) SCD2 append-only
        ▼
{name}__users_fields_history.sql            ← fields_history(...) one row per field change
        ├──────────────► {name}__identity_inputs.sql   ← identity_inputs_from_history(...)  → Identity Manager
        └──────────────► {name}__to_class_people.sql   ← Silver Step 1 → silver.class_people
```

Shared dbt macros (already exist; do not reimplement): `promote_bronze_to_rmt`,
`snapshot`, `fields_history`, `identity_inputs_from_history`.

### `to_class_people` column contract (must match exactly)

`tenant_id, source_id, unique_key, workspace_id, person_id, valid_from, valid_to,
source, source_person_id, employee_number, display_name, first_name, last_name, email,
job_title, department_name, org_unit_id, manager_person_id, status, employment_type,
hire_date, termination_date, location, country, fte, custom_str_attrs (Map(String,String)),
custom_num_attrs (Map(String,Float64)), ingested_at`.

Rules learned from the references:
- `source_person_id` = the source's **stable** id (Entra `oid`, AD `objectGUID`, BambooHR employee id). Survives renames.
- `unique_key` in `to_class_people` = Bronze `unique_key` + `-` + `valid_from` (SCD2 grain — single ORDER BY column).
- `email` = `coalesce(mail, userPrincipalName)` for directories.
- `status` ∈ `active | on_leave | terminated`. Map the source's enabled/disabled or status flag.
- Types must match `silver.class_people` exactly — `cpt-dataflow-constraint-staging-class-column-types-match`. `CAST(NULL AS Nullable(...))` for unknowns.

### `identity_inputs` value types

`identity_inputs_from_history(...)` emits rows with `value_type` ∈
`id | email | employee_id | display_name | sam_account` (extend the model's
`accepted_values` test if you add more). The **`sam_account`** signal is what reconciles
on-prem AD (`sAMAccountName`) with cloud Entra (`onPremisesSamAccountName`) and with
self-hosted Git/GitLab usernames — always emit it when the source has a SAM/legacy login.

## CDK-for-non-HTTP recipe (the new bit `active-directory` demonstrates)

DESIGN.md §4.9 says "use CDK when the declarative manifest can't express it." A non-HTTP
source (LDAP, SOAP, JDBC, file) is the clearest such case. The CDK's `HttpStream`
**also** assumes HTTP — so do **not** subclass it. Subclass the plain
`airbyte_cdk.sources.streams.Stream` and drive the transport yourself:

```
source_{name}/
  __init__.py
  spec.json                 # insight_tenant_id + insight_source_id required; prefix others ({name}_*)
  source.py                 # AbstractSource: spec(), check_connection(), streams()
  {transport}_client.py      # transport helper: connect(config), attribute allowlist, value coercion
  streams/
    __init__.py
    users.py                # class X(Stream): name, primary_key, get_json_schema(), read_records()
```

Key contract points (all enforced by the references):
- `check_connection` returns `(False, "actionable message")` — never raise. Validate
  `insight_source_id` non-empty FIRST (empty → silent dedup collisions), then connect, then a size-limited probe.
- Every record carries `tenant_id`, `source_id`, `unique_key` (`{tenant}-{source}-{natural_key}`), injected in the stream — there is no AddFields in CDK.
- `get_json_schema()` per stream with `additionalProperties: true`; `unique_key`+stable-id `required`. Match the sibling nocode connector's field names so the dbt models stay parallel.
- **Privacy by default**: fetch an explicit attribute allowlist, never the whole object. See `active_directory/ldap_client.py:USER_ATTRIBUTES`.
- `pyproject.toml` declares the transport lib as a dependency (`ldap3` for AD). `[project.scripts]` maps `source-{name}` → `source_{name}.source:main`. Dockerfile `ENTRYPOINT ["source-{name}"]`.
- `descriptor.yaml` `type: cdk` + `images.cdk` block (ADR-0016); `images.cdk.image: ""` until first CI build patches it.

## File-by-file checklist for a new HR/identity connector

1. `descriptor.yaml` — `name`, `type` (nocode|cdk), `version`, `schedule`, `workflow: sync`, `dbt_select: tag:{name}+`, `connection.namespace: bronze_{name_with_underscores}`, `secret.required_fields`. CDK adds `images.cdk`.
2. Extraction — nocode `connector.yaml` (DeclarativeSource + AddFields) **or** CDK `source_{name}/` (AbstractSource + Stream).
3. `dbt/{name}__bronze_promoted.sql`, `__users_snapshot.sql`, `__users_fields_history.sql`, `__identity_inputs.sql`, `__to_class_people.sql`, `schema.yml` (source + freshness + model tests).
4. `README.md` — prerequisites, K8s Secret block + field table, streams, Silver targets.
5. `src/ingestion/secrets/connectors/{name}.yaml.example` — Secret template (gitignored real file alongside).

## Tooling note (docs vs reality)

DESIGN.md §4 and the connector SKILL.md reference an `airbyte-toolkit/` directory and
scripts like `register.sh` / `build-connector.sh`. The live tooling is under
[`src/ingestion/reconcile-connectors/`](../../../src/ingestion/reconcile-connectors/)
(`main.sh`, `lib/cdk-build.sh`, `lib/discover.sh`) plus
[`src/ingestion/tools/declarative-connector/source.sh`](../../../src/ingestion/tools/declarative-connector/source.sh)
for local nocode runs. Prefer `reconcile-connectors/main.sh` for build+register+connect
and `run-sync.sh {name} <tenant>` for e2e. The connector-spec scripts names are stale;
the workflow steps still apply.
