"""The relations a run seeded for each suite, so the leg planner's reading of the tree
is checked against what the tests actually wrote."""

from __future__ import annotations

from dataclasses import dataclass, field

from insight_datapath.suite_scan import Relation


@dataclass
class SeedAudit:
    suite: str | None = None
    seeded: dict[str, set[Relation]] = field(default_factory=dict)

    def record(self, schema: str, table: str) -> None:
        if self.suite is not None:
            self.seeded.setdefault(self.suite, set()).add((schema, table))


#: The seeder records into it; the session compares it with the tree once tests end.
AUDIT = SeedAudit()
