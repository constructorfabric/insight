/**
 * What the page claims, and what it refuses to claim.
 *
 * The rules under test are the ones a reader would otherwise have to infer
 * from cells: that configuration is read before the sync outcome, that an
 * unmeasured value never prints as a zero, and that the page stops presenting
 * its own facts as current once recording has plainly stopped.
 */
import { describe, expect, it } from "vitest";

import type {
  ConnectorHealth,
  SyncFact,
  SyncStatus,
} from "@/api/connector-health-client";
import {
  STALE_AFTER_INTERVALS,
  UNMEASURED,
  describeAge,
  describeConnector,
  describeRecording,
  describeSync,
  formatDuration,
  formatRecords,
  formatStarted,
} from "@/lib/portal/connector-health";

const MINUTE = 60_000;

function sync(over: Partial<SyncFact> = {}): SyncFact {
  return {
    job_id: "8412",
    status: "succeeded",
    started_at: "2026-01-15T09:00:00.000Z",
    duration_ms: 142_000,
    records_reported: 12_400,
    ...over,
  };
}

function row(over: Partial<ConnectorHealth> = {}): ConnectorHealth {
  return {
    connector: "example-tracker",
    configured: true,
    last_sync: sync(),
    ...over,
  };
}

describe("what a row says", () => {
  it("gives every recorded status its own word", () => {
    const words: Record<SyncStatus, string> = {
      pending: "syncing",
      running: "syncing",
      incomplete: "sync incomplete",
      succeeded: "sync ok",
      failed: "sync failed",
      cancelled: "sync cancelled",
      unknown: "state unknown",
    };
    for (const [status, label] of Object.entries(words)) {
      expect(describeConnector(row({ last_sync: sync({ status: status as SyncStatus }) })).label).toBe(
        label,
      );
    }
  });

  it("never says a connector is delivering", () => {
    const statuses: SyncStatus[] = [
      "pending",
      "running",
      "incomplete",
      "succeeded",
      "failed",
      "cancelled",
      "unknown",
    ];
    const labels = statuses.map(
      (status) => describeConnector(row({ last_sync: sync({ status }) })).label,
    );
    for (const label of [...labels, "no longer configured", "never synced"]) {
      expect(label).not.toMatch(/deliver|healthy|fresh|up to date/i);
    }
  });

  it("reads configuration before the sync outcome", () => {
    // A connector taken out of configuration is a decision, not a fault, even
    // when the last thing it did was fail.
    const removed = describeConnector(
      row({ configured: false, last_sync: sync({ status: "failed" }) }),
    );
    expect(removed.state).toBe("no_longer_configured");
    expect(removed.tone).not.toBe("failing");
  });

  it("separates never synced from no longer configured", () => {
    expect(describeConnector(row({ last_sync: null })).state).toBe(
      "never_synced",
    );
    expect(
      describeConnector(row({ configured: false, last_sync: null })).state,
    ).toBe("no_longer_configured");
  });

  it("gives an unreadable state its own tone, not the quiet one", () => {
    const murky = describeConnector(row({ last_sync: sync({ status: "unknown" }) }));
    expect(murky.tone).toBe("unknown");
    expect(murky.tone).not.toBe("ok");
    expect(murky.tone).not.toBe("idle");
  });

  it("carries a word for every tone, so colour is never the only signal", () => {
    const statuses: SyncStatus[] = ["succeeded", "failed", "running", "unknown"];
    for (const status of statuses) {
      expect(describeSync(sync({ status })).label.length).toBeGreaterThan(0);
    }
  });
});

describe("an unmeasured value is not a zero", () => {
  it("prints absence as absence", () => {
    expect(formatDuration(null)).toBe(UNMEASURED);
    expect(formatRecords(null)).toBe(UNMEASURED);
    expect(formatStarted(null)).toBe(UNMEASURED);
  });

  it("prints a measured zero as zero", () => {
    expect(formatRecords(0)).toBe("0");
    expect(formatDuration(0)).toBe("0 ms");
  });

  it("prints an unparseable stamp as absence rather than as an epoch", () => {
    expect(formatStarted("not a date")).toBe(UNMEASURED);
  });

  it("reads durations at the scale they arrive in", () => {
    expect(formatDuration(900)).toBe("900 ms");
    expect(formatDuration(1_500)).toBe("1.5 s");
    expect(formatDuration(90_000)).toBe("1m 30s");
    expect(formatDuration(3_930_000)).toBe("1h 5m");
  });
});

describe("what the page says about its own freshness", () => {
  const summary = (over: Partial<Parameters<typeof describeRecording>[0]> = {}) => ({
    as_of: "2026-01-15T12:00:00.000Z",
    checked_at: "2026-01-15T11:59:00.000Z",
    typical_read_interval_ms: 15 * MINUTE,
    history_available: true,
    ...over,
  });

  it("says nothing has been read rather than implying health", () => {
    const view = describeRecording(summary({ history_available: false }));
    expect(view.state).toBe("never_read");
    expect(view.label).not.toMatch(/healthy|ok|fine/i);
  });

  it("says nothing has been read when no tick has sealed", () => {
    expect(describeRecording(summary({ checked_at: null })).state).toBe(
      "never_read",
    );
  });

  it("presents recent facts as current", () => {
    expect(describeRecording(summary()).state).toBe("current");
  });

  it("tolerates one missed read", () => {
    const view = describeRecording(
      summary({ checked_at: "2026-01-15T11:40:00.000Z" }),
    );
    expect(view.state).toBe("current");
  });

  it("stops presenting its facts as current once recording has plainly stopped", () => {
    // Past STALE_AFTER_INTERVALS of the measured interval.
    const age = 15 * MINUTE * (STALE_AFTER_INTERVALS + 1);
    const checked = new Date(
      Date.parse("2026-01-15T12:00:00.000Z") - age,
    ).toISOString();
    const view = describeRecording(summary({ checked_at: checked }));
    expect(view.state).toBe("stopped");
    expect(view.detail).toMatch(/may no longer be current/i);
  });

  it("claims nothing about a cadence it has not measured", () => {
    // With no measured interval there is nothing to compare an age against, so
    // the page reports the age and stops there.
    const view = describeRecording(
      summary({
        typical_read_interval_ms: null,
        checked_at: "2026-01-01T00:00:00.000Z",
      }),
    );
    expect(view.state).toBe("current");
    expect(view.label).toMatch(/last checked/i);
  });

  it("survives an unreadable stamp without asserting a time", () => {
    const view = describeRecording(summary({ checked_at: "nope" }));
    expect(view.label).toMatch(/unreadable/i);
  });
});

describe("ages read coarsely", () => {
  it("does not pretend to a precision a periodic read does not have", () => {
    expect(describeAge(5_000)).toBe("just now");
    expect(describeAge(5 * MINUTE)).toBe("5 min ago");
    expect(describeAge(3 * 60 * MINUTE)).toBe("3 h ago");
    expect(describeAge(50 * 60 * MINUTE)).toBe("2 d ago");
  });
});
