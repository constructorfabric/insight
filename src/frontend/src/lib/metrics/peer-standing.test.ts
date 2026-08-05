import { describe, expect, it } from "vitest";

import { derivePeerStanding } from "@/lib/metrics/peer-standing";
import type { PeerEntityStats } from "@/lib/metrics/collection";

function peer(overrides: Partial<PeerEntityStats> = {}): PeerEntityStats {
  return {
    entity_id: "me@x.com",
    target_value: 14,
    p25: 5,
    median: 10,
    p75: 15,
    min: 0,
    max: 30,
    n: 8,
    ...overrides,
  };
}

describe("derivePeerStanding", () => {
  it("ranks an eligible value against the quartiles", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 14,
      peer: peer(),
    });
    expect(standing).toMatchObject({
      eligible: true,
      reason: "ok",
      rank: "in_pack",
      observed: true,
    });
    expect(standing.gapDelta).toBe(4);
  });

  it("keeps the gap sign arithmetic when direction flips favorability", () => {
    const lower = derivePeerStanding("lower_is_better", {
      value: 14,
      peer: peer(),
    });
    expect(lower.gapDelta).toBe(4);
    expect(lower.gapPct).toBeCloseTo(0.4);

    const below = derivePeerStanding("lower_is_better", {
      value: 6,
      peer: peer(),
    });
    expect(below.gapDelta).toBe(-4);
  });

  it("returns no_value without a period value", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: null,
      peer: peer(),
    });
    expect(standing).toMatchObject({ eligible: false, reason: "no_value", rank: "neutral" });
  });

  it("returns neutral_direction for neutral metrics", () => {
    const standing = derivePeerStanding("neutral", { value: 3, peer: peer() });
    expect(standing).toMatchObject({ eligible: false, reason: "neutral_direction" });
  });

  it("returns unmeasured when the peer target is null (zero-filled own total)", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 0,
      peer: peer({ target_value: null }),
    });
    expect(standing).toMatchObject({
      eligible: false,
      reason: "unmeasured",
      observed: false,
    });
  });

  it("returns no_stats when percentiles are suppressed", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 14,
      peer: peer({ p25: null, median: null, p75: null, min: null, max: null }),
    });
    expect(standing).toMatchObject({
      eligible: false,
      reason: "no_stats",
      stats: null,
    });
  });

  it("treats a metric without a peer view as unrankable but observed", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 14,
      peer: null,
    });
    expect(standing).toMatchObject({
      eligible: false,
      reason: "no_stats",
      observed: true,
    });
  });

  it("returns flat_pool when the cohort has zero spread", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 0,
      peer: peer({ target_value: 0, p25: 0, median: 0, p75: 0, min: 0, max: 0 }),
    });
    expect(standing).toMatchObject({
      eligible: false,
      reason: "flat_pool",
      rank: "neutral",
    });
  });

  it("marks a median tie as in-pack, never an outlier", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 0,
      peer: peer({ target_value: 0, p25: 0, median: 0, p75: 4, min: 0, max: 9 }),
    });
    expect(standing).toMatchObject({
      eligible: true,
      rank: "in_pack",
    });
  });

  it("normalizes severity by spread when the median is zero", () => {
    const standing = derivePeerStanding("higher_is_better", {
      value: 8,
      peer: peer({ target_value: 8, p25: 0, median: 0, p75: 4, min: 0, max: 9 }),
    });
    expect(standing.rank).toBe("top");
    expect(standing.gapPct).toBeNull();
    expect(standing.severity).toBe(2); // gapDelta 8 / IQR 4
  });
});
