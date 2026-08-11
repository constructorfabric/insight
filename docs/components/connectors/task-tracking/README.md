# Task Tracking Connectors

> Multi-source task tracking: Jira, GitHub Projects V2, Azure DevOps

## Silver Layer

Unified field history with full values (not deltas), field metadata tracking, identity resolution.

- [PRD](silver/specs/PRD.md) — Product requirements
- [DESIGN](silver/specs/DESIGN.md) — Technical design, table schemas, source mappings

## Bronze — Per-Source Specs

| Source | Bronze Schema | Connector Specs |
|--------|--------------|-----------------|
| Jira | [jira.md](jira/jira.md) | [specs/](jira/specs/) |
| GitHub Projects V2 | Planned | — |
| Azure DevOps | Planned | — |

## Key Tables

### Silver (new — full values)

| Table | Description |
|-------|------------|
| `task_tracker_field_history` | Every field change with complete value after event |
| `task_tracker_field_metadata` | Field type snapshots from source APIs |

### Silver (supporting — unchanged)

| Table | Description |
|-------|------------|
| `task_tracker_worklogs` | Logged time per issue |
| `task_tracker_comments` | Issue comments |
| `task_tracker_projects` | Project directory |
| `task_tracker_sprints` | Sprint/iteration metadata |
| `task_tracker_users` | User directory (identity anchor) |
| `task_tracker_issue_links` | Issue dependencies |
| `task_tracker_collection_runs` | Connector execution log |

## Architecture

```
Bronze (per-source) → dbt Step 1 (unified) → Enrich (full values) → dbt Step 2 (identity) → Gold
```
