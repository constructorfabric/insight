-- Identity Resolution & Person domain: database DDL
-- Idempotent — safe to re-run.

CREATE DATABASE IF NOT EXISTS identity;

-- identity.identity_inputs is created by dbt (silver/_shared/identity_inputs.sql
-- plus the per-connector models unioned into it), and identity.identity_persons
-- by the identity-resolution service's persons-sync. Neither belongs here.
