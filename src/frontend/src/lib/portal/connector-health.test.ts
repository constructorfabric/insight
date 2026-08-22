import { describe, expect, it } from "vitest";

import type { ConnectorRow } from "@/api/connector-health-client";

import {
  connectorState,
  elapsedSince,
  orderByAttention,
} from "./connector-health";

function row(
  connector: string,
  last_write: string | null,
  over: Partial<ConnectorRow> = {},
): ConnectorRow {
  return {
    connector,
    namespace: `bronze_${connector}`,
    streams: 1,
    streams_with_data: last_write == null ? 0 : 1,
    rows: last_write == null ? 0 : 1,
    last_write,
    ...over,
  };
}

describe("orderByAttention", () => {
  it("puts the connector whose data is oldest first", () => {
    const ordered = orderByAttention([
      row("recent", "2020-01-05T00:00:00Z"),
      row("ancient", "2020-01-01T00:00:00Z"),
      row("middling", "2020-01-03T00:00:00Z"),
    ]);

    expect(ordered.map((c) => c.connector)).toEqual([
      "ancient",
      "middling",
      "recent",
    ]);
  });

  it("sinks connectors that never delivered below those that did", () => {
    const ordered = orderByAttention([
      row("never", null),
      row("delivered", "2020-01-01T00:00:00Z"),
    ]);

    expect(ordered.map((c) => c.connector)).toEqual(["delivered", "never"]);
  });

  it("leaves the caller's array untouched", () => {
    const input = [row("b", "2020-01-05T00:00:00Z"), row("a", "2020-01-01T00:00:00Z")];

    orderByAttention(input);

    expect(input.map((c) => c.connector)).toEqual(["b", "a"]);
  });
});

describe("elapsedSince", () => {
  const now = new Date("2020-01-10T12:00:00Z");

  it("reports a connector that has never delivered as never", () => {
    expect(elapsedSince(null, now)).toEqual({ kind: "never" });
  });

  it("counts in hours within the first day", () => {
    expect(elapsedSince("2020-01-10T02:00:00Z", now)).toEqual({
      kind: "hours",
      value: 10,
    });
  });

  it("counts in days beyond the first", () => {
    expect(elapsedSince("2020-01-02T12:00:00Z", now)).toEqual({
      kind: "days",
      value: 8,
    });
  });

  it("switches to days at the day boundary rather than reporting 24h", () => {
    expect(elapsedSince("2020-01-09T12:00:00Z", now)).toEqual({
      kind: "days",
      value: 1,
    });
  });
  it("reports an arrival stamped ahead of the read as just now, not as negative", () => {
    expect(elapsedSince("2020-01-10T12:00:01Z", now)).toEqual({
      kind: "hours",
      value: 0,
    });
  });

  it("reports an unparseable timestamp as unknown rather than NaN", () => {
    expect(elapsedSince("not a timestamp", now)).toEqual({ kind: "unknown" });
  });
});

describe("connectorState", () => {
  it("is never for a connector that has delivered nothing", () => {
    expect(connectorState(row("a", null))).toBe("never");
  });

  it("is partial while any stream is still empty", () => {
    expect(
      connectorState(
        row("a", "2020-01-01T00:00:00Z", { streams: 3, streams_with_data: 2 }),
      ),
    ).toBe("partial");
  });

  it("is delivering once every stream carries data", () => {
    expect(
      connectorState(
        row("a", "2020-01-01T00:00:00Z", { streams: 3, streams_with_data: 3 }),
      ),
    ).toBe("delivering");
  });
});
