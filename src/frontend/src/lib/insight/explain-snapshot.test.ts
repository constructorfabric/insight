import { describe, expect, it } from "vitest";

import { metricSnapshot } from "./explain-snapshot";
import type { KpiTileData } from "@/lib/insight/kpi-row";

function tile(overrides: Partial<KpiTileData> = {}): KpiTileData {
  return {
    key: "tasks.closed",
    label: "Tasks closed",
    value: "34",
    delta: { text: "+6", status: "neutral", down: false },
    medianLabel: "median 27",
    gapText: "+26%",
    gapStatus: "neutral",
    help: null,
    groupId: null,
    ...overrides,
  };
}

const CONTEXT = { periodNoun: "month", since: "2026-08-01", until: "2026-08-22" };

describe("metricSnapshot", () => {
  it("sends the same sentences the tile shows", () => {
    const snapshot = metricSnapshot(tile(), CONTEXT);

    expect(snapshot.delta).toBe("+6 since last month");
    expect(snapshot.peer).toBe("Team median 27 · +26%");
  });

  it("leaves the change empty when the tile has no earlier period", () => {
    const snapshot = metricSnapshot(tile({ delta: null }), CONTEXT);

    expect(snapshot.delta).toBe("");
  });

  it("leaves the comparison empty when there is no cohort", () => {
    const snapshot = metricSnapshot(
      tile({ medianLabel: null, gapText: null }),
      CONTEXT
    );

    expect(snapshot.peer).toBe("");
  });

  it("carries the window the reader is looking at", () => {
    const snapshot = metricSnapshot(tile(), CONTEXT);

    expect(snapshot.since).toBe("2026-08-01");
    expect(snapshot.until).toBe("2026-08-22");
  });
});
