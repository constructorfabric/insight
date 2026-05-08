# MS Entra Connector — Spec Index

Microsoft Entra ID (formerly Azure AD) cloud directory connector. Pulls the canonical user list via Microsoft Graph and emits identity signals into the Identity Manager so users authenticated against Entra can be resolved to their accounts in other services (GitHub, Slack, Jira, BambooHR, …).

## Specs

- [PRD.md](./specs/PRD.md) — product requirements
- [DESIGN.md](./specs/DESIGN.md) — technical design
- [ADR/](./specs/ADR/) — architecture decisions (none yet)

## Implementation

- Package: `src/ingestion/connectors/hr-directory/ms-entra/`
- Bronze namespace: `bronze_ms_entra`
- Streams: `users`
- dbt models: `ms_entra__bronze_promoted`, `ms_entra__users_snapshot`, `ms_entra__users_fields_history`, `ms_entra__identity_inputs`, `to_class_people`

## Distinct from m365 connector

This connector is separate from `collaboration/m365`. m365 fetches **activity reports** (App permission `Reports.Read.All`); ms-entra fetches **directory data** (App permission `User.Read.All`). They use separate App Registrations to keep audit trails split and minimise blast radius.
