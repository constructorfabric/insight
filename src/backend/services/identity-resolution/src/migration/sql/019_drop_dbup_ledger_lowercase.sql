-- WORKAROUND: the retired DbUp applier named its ledger in lowercase, and table
-- names are case-sensitive where lower_case_table_names = 0.
DROP TABLE IF EXISTS schemaversions;
