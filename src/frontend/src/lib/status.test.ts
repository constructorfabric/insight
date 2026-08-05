import { describe, expect, it } from "vitest";

import { applyFocusStatus, statusVsMedian } from "@/lib/status";

describe("applyFocusStatus", () => {
  it("passes everything through in 'all' mode", () => {
    expect(applyFocusStatus("good", "all")).toBe("good");
    expect(applyFocusStatus("bad", "all")).toBe("bad");
  });

  it("keeps only 'bad' in critical mode", () => {
    expect(applyFocusStatus("bad", "critical")).toBe("bad");
    expect(applyFocusStatus("good", "critical")).toBe("neutral");
    expect(applyFocusStatus("warn", "critical")).toBe("neutral");
  });

  it("keeps only 'good' in rewards mode", () => {
    expect(applyFocusStatus("good", "rewards")).toBe("good");
    expect(applyFocusStatus("bad", "rewards")).toBe("neutral");
  });

  it("mutes everything in neutral mode", () => {
    expect(applyFocusStatus("good", "neutral")).toBe("neutral");
    expect(applyFocusStatus("bad", "neutral")).toBe("neutral");
  });
});

describe("statusVsMedian", () => {
  it("is neutral when either operand is not finite", () => {
    expect(statusVsMedian(Number.NaN, 5, true)).toBe("neutral");
    expect(statusVsMedian(5, Number.POSITIVE_INFINITY, true)).toBe("neutral");
  });

  it("scores at-or-above median good when higher is better", () => {
    expect(statusVsMedian(5, 5, true)).toBe("good");
    expect(statusVsMedian(6, 5, true)).toBe("good");
    expect(statusVsMedian(4, 5, true)).toBe("bad");
  });

  it("scores at-or-below median good when lower is better", () => {
    expect(statusVsMedian(5, 5, false)).toBe("good");
    expect(statusVsMedian(4, 5, false)).toBe("good");
    expect(statusVsMedian(6, 5, false)).toBe("bad");
  });

  it("treats a zero median as a legitimate comparison point", () => {
    expect(statusVsMedian(0, 0, true)).toBe("good");
    expect(statusVsMedian(-1, 0, true)).toBe("bad");
  });
});
