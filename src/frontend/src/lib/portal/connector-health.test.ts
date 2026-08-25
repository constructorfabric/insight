import { describe, expect, it } from "vitest";

import type {
  ConnectorHealthRow,
  RunFacts,
  StorageFacts,
  SyncFacts,
  SyncTrigger,
} from "@/api/connector-health-client";

import {
  connectorState,
  connectorStateLabel,
  formatAge,
  formatBytes,
  formatDelivery,
  formatDuration,
  stateCounts,
  triggerLabel,
} from "./connector-health";

function run(over: Partial<RunFacts> = {}): RunFacts {
  return {
    status: "ok",
    step: null,
    started_at: "2026-01-15T09:00:00Z",
    duration_ms: 90_000,
    transform_status: "ok",
    ...over,
  };
}

function sync(over: Partial<SyncFacts> = {}): SyncFacts {
  return {
    trigger: "claimed",
    status: "ok",
    started_at: "2026-01-15T09:00:00Z",
    duration_ms: 60_000,
    records_moved: 400,
    rows_landed: 400,
    ...over,
  };
}

function storage(over: Partial<StorageFacts> = {}): StorageFacts {
  return {
    observed_at: "2026-01-15T09:00:00Z",
    streams: 4,
    streams_with_data: 4,
    physical_rows: 100,
    bytes_on_disk: 1024,
    ...over,
  };
}

function row(over: Partial<ConnectorHealthRow> = {}): ConnectorHealthRow {
  return {
    connector: "example-tool",
    configured: true,
    last_run: run(),
    last_sync: sync(),
    storage: storage(),
    streams: [],
    ...over,
  };
}

describe("connectorState", () => {
  it("calls a connector delivering when the last run completed and nothing contradicts it", () => {
    expect(connectorState(row())).toBe("delivering");
  });

  it("reports a measured zero beside moved records as a delivery mismatch", () => {
    const state = connectorState(
      row({ last_sync: sync({ records_moved: 12_400, rows_landed: 0 }) })
    );

    expect(state).toBe("misdelivered");
  });

  it("does not read an unmeasured sync as a mismatch", () => {
    // The distinction the whole pairing exists for: a swept, out-of-band or
    // backfilled sync carries no measurement, and absence is not zero delivery.
    const state = connectorState(
      row({ last_sync: sync({ records_moved: 12_400, rows_landed: null }) })
    );

    expect(state).toBe("delivering");
  });

  it("does not read an uncounted sync as a mismatch either", () => {
    // The mover's counters arrive with the sweep. Until then nobody knows what
    // the sync moved, and unknown beside a measured zero is a gap.
    const state = connectorState(
      row({ last_sync: sync({ records_moved: null, rows_landed: 0 }) })
    );

    expect(state).toBe("delivering");
  });

  it("reports a failed sync rather than calling the connector delivering", () => {
    // The mover's own outcome. Nothing else on the page would have shown it.
    const state = connectorState(
      row({ last_run: null, last_sync: sync({ status: "failed", records_moved: 0 }) })
    );

    expect(state).toBe("sync_failed");
  });

  it("does not call a run still in flight healthy", () => {
    expect(connectorState(row({ last_run: run({ status: "running" }) }))).toBe(
      "in_flight"
    );
    expect(
      connectorState(row({ last_run: null, last_sync: sync({ status: "running" }) }))
    ).toBe("in_flight");
  });

  it("says a connector is no longer configured rather than still delivering", () => {
    // History outlives configuration by design, so runs with no configuration
    // are not "never configured" — they are a connector nothing manages now.
    const state = connectorState(row({ configured: false }));

    expect(state).toBe("unmanaged");
  });

  it("reports an out-of-band sync newer than the last run", () => {
    // A surviving older run must not mask it: the newest data was never
    // transformed, whether or not the pipeline ran yesterday.
    const state = connectorState(
      row({
        last_run: run({ started_at: "2026-01-14T09:00:00Z" }),
        last_sync: sync({ trigger: "out_of_band", started_at: "2026-01-15T09:00:00Z" }),
      })
    );

    expect(state).toBe("sync_without_transform");
  });

  it("does not report an out-of-band sync older than the last run", () => {
    const state = connectorState(
      row({
        last_run: run({ started_at: "2026-01-15T09:00:00Z" }),
        last_sync: sync({ trigger: "out_of_band", started_at: "2026-01-14T09:00:00Z" }),
      })
    );

    expect(state).toBe("delivering");
  });

  it("says nothing is stored rather than calling an empty connector healthy", () => {
    const state = connectorState(row({ storage: storage({ physical_rows: 0 }) }));

    expect(state).toBe("nothing_stored");
  });

  it("ranks a delivery mismatch above a failed run", () => {
    const state = connectorState(
      row({
        last_run: run({ status: "failed" }),
        last_sync: sync({ records_moved: 400, rows_landed: 0 }),
      })
    );

    expect(state).toBe("misdelivered");
  });

  it("separates a failed transform from a failed run", () => {
    expect(connectorState(row({ last_run: run({ status: "failed" }) }))).toBe(
      "run_failed"
    );
    expect(
      connectorState(row({ last_run: run({ transform_status: "failed" }) }))
    ).toBe("transform_failed");
  });

  it("says a sync ran without a transform when nothing but the mover started it", () => {
    const state = connectorState(
      row({ last_run: null, last_sync: sync({ trigger: "out_of_band" }) })
    );

    expect(state).toBe("sync_without_transform");
  });

  it("does not claim a transform was skipped when the sync's origin is unknown", () => {
    const state = connectorState(
      row({ last_run: null, last_sync: sync({ trigger: "unclaimed" }) })
    );

    expect(state).toBe("delivering");
  });

  it("separates configured-and-never-ran from a schema nobody configured", () => {
    expect(
      connectorState(row({ configured: true, last_run: null, last_sync: null }))
    ).toBe("never_ran");
    expect(
      connectorState(row({ configured: false, last_run: null, last_sync: null }))
    ).toBe("not_configured");
  });

  it("gives every state words as well as a tone, so colour is never alone", () => {
    const labelled = connectorStateLabel(
      row({ last_sync: sync({ records_moved: 9, rows_landed: 0 }) })
    );

    expect(labelled.state).toBe("misdelivered");
    expect(labelled.label).toBe("recorded, nothing landed");
    expect(labelled.tone).toBe("critical");
  });
});

describe("triggerLabel", () => {
  it.each<[SyncTrigger, string]>([
    ["claimed", "started by the pipeline"],
    ["out_of_band", "started outside the pipeline"],
    ["unclaimed", "origin unknown"],
  ])("reads %s as %s", (trigger, expected) => {
    expect(triggerLabel(trigger)).toBe(expected);
  });

  it("does not claim a schedule the ledger never recorded", () => {
    // `claimed` means a pipeline run claimed the job, not that a schedule
    // started it — the ledger records no schedule at all.
    expect(triggerLabel("claimed")).not.toContain("schedul");
  });

  it("reads a word outside the vocabulary as unknown, not as nothing", () => {
    // A silently absent label is indistinguishable from nothing recorded.
    expect(triggerLabel("something-new" as SyncTrigger)).toBe("origin unknown");
  });

  it("never presents unknown provenance as a manual sync", () => {
    expect(triggerLabel("unclaimed")).not.toContain("manual");
  });

  it("says nothing when no trigger was recorded", () => {
    expect(triggerLabel(null)).toBeNull();
  });
});

describe("formatDelivery", () => {
  it("shows both numbers when the delivery was measured", () => {
    expect(formatDelivery(12_400, 12_400)).toBe("12,400 / 12,400");
  });

  it("shows a measured zero as a zero", () => {
    expect(formatDelivery(12_400, 0)).toBe("12,400 / 0");
  });

  it("shows an unmeasured delivery as unmeasured rather than as zero", () => {
    expect(formatDelivery(12_400, null)).toBe("12,400 / not measured");
  });

  it("shows uncounted records as unrecorded rather than as zero", () => {
    expect(formatDelivery(null, 4_200)).toBe("not recorded / 4,200");
  });
});

describe("formatting", () => {
  it.each([
    [45_000, "45s"],
    [97_000, "1m 37s"],
    [3_723_000, "1h 2m"],
  ])("renders %sms as %s", (ms, expected) => {
    expect(formatDuration(ms)).toBe(expected);
  });

  it("renders a measured zero duration as a zero and an absent one as absent", () => {
    // Same discipline as the delivery pairing: 0 is a measurement.
    expect(formatDuration(0)).toBe("0ms");
    expect(formatDuration(null)).toBe("not recorded");
  });

  it.each([
    [1024, "1.0 KiB"],
    [314_572_800, "300 MiB"],
  ])("renders %s bytes as %s", (bytes, expected) => {
    expect(formatBytes(bytes)).toBe(expected);
  });

  it("renders a measured zero size as a zero and an absent one as absent", () => {
    // A connector with rows: 0 and size: "—" said one measured zero two ways.
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(null)).toBe("unknown");
  });

  it("renders an age against a fixed now", () => {
    const now = new Date("2026-01-15T12:00:00Z");

    expect(formatAge("2026-01-15T11:59:40Z", now)).toBe("just now");
    expect(formatAge("2026-01-15T09:00:00Z", now)).toBe("3h ago");
    expect(formatAge("2026-01-09T12:00:00Z", now)).toBe("6d ago");
  });

  it("renders an unparseable or absent stamp as unknown rather than as NaN", () => {
    expect(formatAge("not-a-date")).toBe("unknown");
    expect(formatAge(null)).toBe("unknown");
  });
});

describe("stateCounts", () => {
  it("counts each state once so a tile and a row badge cannot disagree", () => {
    const counts = stateCounts([
      row({ connector: "a" }),
      row({ connector: "b", last_run: run({ status: "failed" }) }),
      row({ connector: "c", last_run: run({ status: "failed" }) }),
      row({ connector: "d", configured: false, last_run: null, last_sync: null }),
    ]);

    expect(counts.get("delivering")).toBe(1);
    expect(counts.get("run_failed")).toBe(2);
    expect(counts.get("not_configured")).toBe(1);
    expect(counts.get("misdelivered")).toBeUndefined();
  });
});
