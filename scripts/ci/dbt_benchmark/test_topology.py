from __future__ import annotations

import unittest

from topology import changed_database, replicated_statements


class TopologyTests(unittest.TestCase):
    def test_cluster_databases_replay_table_metadata_and_tables_replicate_rows(self) -> None:
        sql = replicated_statements("CREATE DATABASE silver")[0]
        self.assertIn("ON CLUSTER rig ENGINE = Replicated(", sql)
        self.assertIn("'{shard}', '{replica}'", sql)
        create = replicated_statements(
            "CREATE TABLE benchmark.example ENGINE = ReplacingMergeTree(_version) ORDER BY id AS "
            "SELECT number AS id, toUInt64(1) AS _version FROM numbers(10)"
        )
        self.assertEqual(len(create), 2)
        self.assertIn("ReplicatedReplacingMergeTree(_version)", create[0])
        self.assertIn("EMPTY AS SELECT number AS id", create[0])
        self.assertEqual(
            create[1], "INSERT INTO benchmark.example SELECT number AS id, toUInt64(1) AS _version FROM numbers(10)"
        )
        self.assertNotIn("ON CLUSTER", create[0])

    def test_query_settings_and_final_reads_are_not_rewritten(self) -> None:
        sql = "INSERT INTO benchmark.example WITH c AS (SELECT * FROM silver.example FINAL) SELECT * FROM c"
        self.assertEqual(replicated_statements(sql), (sql,))
        for sql in (
            "CREATE TABLE benchmark.example (id UInt64) ENGINE = MergeTree ORDER BY id",
            "TRUNCATE TABLE benchmark.example",
            "DROP TABLE IF EXISTS benchmark.example",
        ):
            self.assertEqual(changed_database(sql), "benchmark")
        self.assertIsNone(changed_database("INSERT INTO benchmark.example SELECT 1"))

    def test_a_compiled_model_with_ctes_splits_ddl_from_its_insert(self) -> None:
        query = "-- synthetic compiled SQL\nWITH c AS (SELECT number AS id FROM numbers(10)) SELECT id FROM c"
        statements = replicated_statements("CREATE TABLE benchmark.example ENGINE = MergeTree ORDER BY id AS " + query)
        self.assertEqual(len(statements), 2)
        self.assertIn("EMPTY AS -- synthetic compiled SQL\nWITH c AS (", statements[0])
        self.assertEqual(statements[1], "INSERT INTO benchmark.example " + query)


if __name__ == "__main__":
    unittest.main()
