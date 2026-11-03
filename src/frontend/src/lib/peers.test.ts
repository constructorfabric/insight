import { describe, expect, it } from "vitest";

import { peerStatusVsQuartiles } from "@/lib/peers";

describe("peerStatusVsQuartiles", () => {
  it("keeps inclusive quartile boundaries when they sit beyond the median", () => {
    const stats = { p25: 2, p50: 5, p75: 8 };
    expect(peerStatusVsQuartiles(8, stats, true)).toBe("top");
    expect(peerStatusVsQuartiles(2, stats, true)).toBe("bottom");
    expect(peerStatusVsQuartiles(5, stats, true)).toBe("in_pack");
    expect(peerStatusVsQuartiles(2, stats, false)).toBe("top");
    expect(peerStatusVsQuartiles(8, stats, false)).toBe("bottom");
  });

  it("ranks nobody in a flat pool (everyone at the same value)", () => {
    const flat = { p25: 0, p50: 0, p75: 0 };
    expect(peerStatusVsQuartiles(0, flat, true)).toBe("in_pack");
    expect(peerStatusVsQuartiles(0, flat, false)).toBe("in_pack");
  });

  it("still ranks values strictly beyond a collapsed pool", () => {
    const collapsed = { p25: 0, p50: 0, p75: 0 };
    expect(peerStatusVsQuartiles(5, collapsed, true)).toBe("top");
    expect(peerStatusVsQuartiles(-1, collapsed, true)).toBe("bottom");
    expect(peerStatusVsQuartiles(5, collapsed, false)).toBe("bottom");
  });

  it("never brands a median tie as an outlier in a zero-inflated pool", () => {
    // Many peers at 0, a few above: p25 == median == 0 but the pool has
    // spread. A person at 0 is at the median — in the pack, not "Bottom 25%".
    const zeroInflated = { p25: 0, p50: 0, p75: 4 };
    expect(peerStatusVsQuartiles(0, zeroInflated, true)).toBe("in_pack");
    expect(peerStatusVsQuartiles(5, zeroInflated, true)).toBe("top");
    expect(peerStatusVsQuartiles(0, zeroInflated, false)).toBe("in_pack");
    expect(peerStatusVsQuartiles(5, zeroInflated, false)).toBe("bottom");
  });

  it("requires the median side for the top rank symmetrically", () => {
    // Right-heavy ties: p50 == p75. Sitting on them is the pack's middle.
    const tiedHigh = { p25: 1, p50: 6, p75: 6 };
    expect(peerStatusVsQuartiles(6, tiedHigh, true)).toBe("in_pack");
    expect(peerStatusVsQuartiles(7, tiedHigh, true)).toBe("top");
    expect(peerStatusVsQuartiles(1, tiedHigh, true)).toBe("bottom");
  });
});
