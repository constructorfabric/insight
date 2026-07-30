-- presentation_ro: read-only-by-construction role for the presentation query
-- path (#1963). Contract = SELECT only; `presentation` = SELECT/INSERT/CREATE;
-- no DROP/ALTER/TRUNCATE anywhere. Idempotent. Needs an admin with
-- access_management (compose/e2e/bitnami already have it; see README.md).
-- Spec: docs/domain/presentation-layer/specs.
-- The grant-less `presentation` user that carries this role is created by
-- provision-presentation-access.sh (#1964; needs a password → not static DDL).

CREATE ROLE IF NOT EXISTS presentation_ro;

-- Contract (read-only): silver + identity/person + legacy gold in `insight`.
GRANT SELECT ON silver.* TO presentation_ro;
GRANT SELECT ON person.* TO presentation_ro;
GRANT SELECT ON identity.* TO presentation_ro;
GRANT SELECT ON insight.* TO presentation_ro;

-- presentation (writable): no destructive DDL.
GRANT SELECT, INSERT, CREATE ON presentation.* TO presentation_ro;
