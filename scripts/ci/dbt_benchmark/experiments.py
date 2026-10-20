from __future__ import annotations

import json
import sys
from dataclasses import dataclass

from topology import TOPOLOGIES

MIB = 1024 * 1024
MODEL = "git_authored_commits"
SERVING = "serving_writes"


@dataclass(frozen=True)
class Experiment:
    name: str
    parameter: str
    value: str | int
    workloads: tuple[str, ...]

    def candidate(self, baseline: dict[str, str | int]) -> dict[str, str | int]:
        if self.parameter not in baseline:
            raise ValueError(f"baseline has no {self.parameter}")
        candidate = {**baseline, self.parameter: self.value}
        changed = {key for key in baseline if candidate[key] != baseline[key]}
        if changed != {self.parameter}:
            raise ValueError(f"{self.name} must change exactly {self.parameter}; got {sorted(changed)}")
        return candidate


EXPERIMENTS = (
    Experiment("cpu-threads", "max_threads", 4, (MODEL, "aggregation", "sort", "join", SERVING)),
    Experiment("aggregation-spill", "max_bytes_before_external_group_by", 512 * MIB, (MODEL, "aggregation")),
    Experiment("sort-spill", "max_bytes_before_external_sort", 512 * MIB, (MODEL, "sort")),
    Experiment("join-memory", "max_bytes_in_join", 512 * MIB, (MODEL, "join")),
    Experiment("read-block", "max_block_size", 131072, (MODEL, SERVING)),
    Experiment("insert-max-rows", "max_insert_block_size", 131072, (MODEL, SERVING)),
    Experiment("insert-min-rows", "min_insert_block_size_rows", 131072, (MODEL, SERVING)),
    Experiment("insert-min-bytes", "min_insert_block_size_bytes", 64 * MIB, (MODEL, SERVING)),
    Experiment("insert-dedup", "optimize_on_insert", 0, ("insert_dedup",)),
    Experiment("partition-yearly", "partition_by", "toYear(metric_date)", (SERVING,)),
    Experiment("partition-none", "partition_by", "tuple()", (SERVING,)),
)
BY_NAME = {experiment.name: experiment for experiment in EXPERIMENTS}


if __name__ == "__main__":
    matrix = []
    for experiment in EXPERIMENTS:
        matrix.extend({"experiment": experiment.name, "topology": topology} for topology in TOPOLOGIES)
    sys.stdout.write(json.dumps({"include": matrix}) + "\n")
