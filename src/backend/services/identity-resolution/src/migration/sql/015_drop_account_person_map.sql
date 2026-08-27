-- The derived SCD2 binding cache has no reader: every binding read resolves
-- from the `persons` journal directly. Nothing writes it either as of the
-- change that ships this migration.
DROP TABLE IF EXISTS account_person_map;
