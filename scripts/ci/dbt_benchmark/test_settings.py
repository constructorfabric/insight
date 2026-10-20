from __future__ import annotations

import unittest

from client import Client, checked_body
from run import MIB, variants


class SettingsTests(unittest.TestCase):
    def test_a_late_clickhouse_error_is_rejected_even_with_http_success(self) -> None:
        with self.assertRaises(RuntimeError):
            checked_body(200, "Code: 241. DB::Exception: synthetic memory limit exceeded")
        self.assertEqual(checked_body(200, '{"value":"Code: example"}\n'), '{"value":"Code: example"}\n')

    def test_benchmark_refuses_non_loopback_or_non_http_endpoints(self) -> None:
        for url in ("https://127.0.0.1:18127", "file:///tmp/input", "http://example.com", "http://127.0.0.1/admin"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                Client(url, "synthetic", "synthetic")

    def test_experiments_preserve_the_query_memory_cap_and_automatic_join_fallback(self) -> None:
        baseline = {
            "max_memory_usage": 2 * 1024 * MIB,
            "max_threads": 2,
            "max_bytes_before_external_group_by": 256 * MIB,
            "max_bytes_before_external_sort": 256 * MIB,
            "max_bytes_in_join": 256 * MIB,
            "join_algorithm": "auto",
        }
        original = dict(baseline)
        for name, settings in variants(baseline).items():
            with self.subTest(variant=name):
                self.assertEqual(settings["max_memory_usage"], baseline["max_memory_usage"])
                self.assertEqual(settings["join_algorithm"], "auto")
                self.assertLess(settings["max_bytes_before_external_group_by"], settings["max_memory_usage"])
                self.assertLess(settings["max_bytes_before_external_sort"], settings["max_memory_usage"])
                self.assertLess(settings["max_bytes_in_join"], settings["max_memory_usage"])
        self.assertEqual(baseline, original)


if __name__ == "__main__":
    unittest.main()
