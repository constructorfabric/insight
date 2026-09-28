import { describe, expect, it } from "vitest";

import { updatedLabel } from "./updated-label";

const NOW = Date.parse("2026-09-28T10:00:00Z");

describe("updatedLabel", () => {
  it.each([
    ["2026-09-25T10:00:00Z", "Updated 3 d ago"],
    ["2026-09-28T05:00:00Z", "Updated 5 h ago"],
    ["2026-09-28T09:40:00Z", "Updated 20 min ago"],
    ["2026-09-28T09:59:50Z", "Updated just now"],
    ["2026-09-28T10:05:00Z", "Updated just now"],
  ])("says %s was %s", (stamp, said) => {
    expect(updatedLabel(stamp, NOW)).toBe(said);
  });

  it.each([[undefined], [""], ["yesterday"]])(
    "says nothing for %j",
    (stamp) => {
      expect(updatedLabel(stamp, NOW)).toBeNull();
    }
  );
});
