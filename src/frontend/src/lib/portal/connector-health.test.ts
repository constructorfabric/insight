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
      pending: "queued",
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

  it("keeps queued apart from syncing", () => {
    // Merging them makes a row say "syncing" beside a start of "—", and erases
    // the only signal separating "not picked up" from "running now".
    const queued = describeConnector(row({ last_sync: sync({ status: "pending" }) }));
    const running = describeConnector(row({ last_sync: sync({ status: "running" }) }));
    expect(queued.state).not.toBe(running.state);
    expect(queued.label).not.toBe(running.label);
  });

  it("reads a status this build has never heard of as unreadable", () => {
    // The contract types status as a bare string, so the mover's vocabulary can
    // grow without this build changing. That must not crash and must not be
    // filed as quiet.
    const view = describeConnector(
      row({ last_sync: sync({ status: "materialising" as never }) }),
    );
    expect(view.state).toBe("state_unknown");
    expect(view.tone).toBe("unknown");
  });

  it("carries a word for every tone, so colour is never the only signal", () => {
    const statuses: SyncStatus[] = ["succeeded", "failed", "running", "unknown"];
    for (const status of statuses) {
      expect(describeSync(sync({ status })).label.length).toBeGreaterThan(0);
    }
  });
});

describe("a contract-legal response with absent keys", () => {
  // Every nullable field is optional in the generated contract, so the key can
  // be missing rather than null. Typing them as `T | null` let `tsc` believe
  // otherwise, and the page threw on a legal response.
  it("a row with no last_sync key reads as never synced", () => {
    const view = describeConnector({ connector: "a", configured: true });
    expect(view.state).toBe("never_synced");
  });

  it("absent measurements print as absence rather than throwing", () => {
    expect(formatRecords(undefined)).toBe(UNMEASURED);
    expect(formatDuration(undefined)).toBe(UNMEASURED);
    expect(formatStarted(undefined)).toBe(UNMEASURED);
  });

  it("a summary with no checked_at key says nothing has been read", () => {
    const view = describeRecording({
      as_of: "2026-01-15T12:00:00.000Z",
      history_available: true,
    });
    expect(view.state).toBe("never_read");
  });

  it("a sync with no status at all is unreadable, not quiet", () => {
    expect(describeSync({ job_id: "1" } as never).state).toBe("state_unknown");
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

  it("never carries sixty of the unit below", () => {
    // Rounding each unit independently produces `1m 60s` and `60.0 s`, neither
    // of which is a duration.
    expect(formatDuration(119_600)).toBe("2m 0s");
    expect(formatDuration(59_950)).toBe("1m 0s");
    expect(formatDuration(3_599_600)).toBe("1h 0m");
  });

  it("prints a malformed measurement as absence, not as a number", () => {
    expect(formatDuration(-5_000)).toBe(UNMEASURED);
    expect(formatRecords(-1)).toBe(UNMEASURED);
    expect(formatDuration(Number.NaN)).toBe(UNMEASURED);
  });

  it("refuses a stamp that is not a real timestamp", () => {
    // `Date.parse` reads "2026" and "0" as dates, so a truncated stamp would
    // render as a confident absolute time.
    for (const junk of ["2026", "0", "yesterday", "2026-13-45T99:99:99Z"]) {
      expect(formatStarted(junk)).toBe(UNMEASURED);
    }
    expect(formatStarted("2026-01-15T09:00:00.000Z")).toBe("2026-01-15 09:00:00Z");
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

  const checkedAgo = (ms: number) =>
    new Date(Date.parse("2026-01-15T12:00:00.000Z") - ms).toISOString();

  it("says nothing has been read rather than implying health", () => {
    const view = describeRecording(summary({ history_available: false }));
    expect(view.state).toBe("never_read");
    expect(view.label).not.toMatch(/healthy|ok|fine/i);
  });

  it("presents recent facts as current", () => {
    expect(describeRecording(summary()).state).toBe("current");
  });

  it("tolerates one missed read but not three", () => {
    const interval = 15 * MINUTE;
    // Just inside the threshold, and just past it — the boundary the multiplier
    // actually decides, rather than a value nowhere near it.
    expect(
      describeRecording(
        summary({ checked_at: checkedAgo(interval * STALE_AFTER_INTERVALS) }),
      ).state,
    ).toBe("current");
    expect(
      describeRecording(
        summary({ checked_at: checkedAgo(interval * STALE_AFTER_INTERVALS + 1) }),
      ).state,
    ).toBe("stopped");
  });

  it("states the age without claiming a stop it cannot support", () => {
    // With no measured interval there is no cadence to be late against.
    // Inventing a threshold here would have the page conclude something
    // nothing in the record says — so it reports the age and says the cadence
    // is unknown.
    const view = describeRecording(
      summary({ typical_read_interval_ms: null, checked_at: checkedAgo(6 * 60 * MINUTE) }),
    );
    expect(view.state).toBe("unmeasured");
    expect(view.label).toMatch(/last checked 6 h ago/i);
    expect(view.label).not.toMatch(/stopped/i);
    expect(view.detail).toMatch(/too few reads/i);
  });

  it("treats a zero measured interval as no measurement at all", () => {
    // Two ticks inside one millisecond make the median zero, which is not a
    // cadence either.
    const view = describeRecording(
      summary({ typical_read_interval_ms: 0, checked_at: checkedAgo(6 * 60 * MINUTE) }),
    );
    expect(view.state).toBe("unmeasured");
  });

  it("still says the age is long, so the fact is not hidden", () => {
    const view = describeRecording(
      summary({ typical_read_interval_ms: null, checked_at: checkedAgo(200 * 24 * 60 * MINUTE) }),
    );
    expect(view.label).toMatch(/200 d ago/);
  });

  it("does not cry stopped because a burst shortened the interval", () => {
    // A restart loop or a manual tick drags the median down; multiplying that
    // by three would flag a live install.
    const view = describeRecording(
      summary({ typical_read_interval_ms: 1_000, checked_at: checkedAgo(5 * MINUTE) }),
    );
    expect(view.state).toBe("current");
  });

  it("does not let a long measured interval hide a stopped recorder", () => {
    // Clamped to an hour, so three of them is three hours.
    const view = describeRecording(
      summary({
        typical_read_interval_ms: 30 * 24 * 60 * MINUTE,
        checked_at: checkedAgo(4 * 60 * MINUTE),
      }),
    );
    expect(view.state).toBe("stopped");
  });

  it("dates nothing when the two clocks disagree", () => {
    // `checked_at` after `as_of` means neither stamp can date the page.
    // Reporting "just now" there asserts a freshness with no basis.
    const view = describeRecording(
      summary({ checked_at: "2026-01-16T00:00:00.000Z" }),
    );
    expect(view.state).toBe("unreadable");
    expect(view.label).not.toMatch(/just now/i);
  });

  it("dates nothing when a stamp will not parse", () => {
    expect(describeRecording(summary({ checked_at: "nope" })).state).toBe(
      "unreadable",
    );
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
